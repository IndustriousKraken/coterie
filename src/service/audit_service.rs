//! Admin audit log. Records a row in the `audit_logs` table every time
//! an admin performs a mutation we care about. Designed to be called
//! fire-and-forget: logging failures are recorded via tracing but never
//! bubble up to the caller, because a DB failure on the audit log
//! shouldn't mask or block the primary operation.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::error::Result;

pub struct AuditService {
    pool: SqlitePool,
}

/// A single audit-log entry as returned to the admin UI.
#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub actor_id: Option<Uuid>,
    pub actor_name: Option<String>,
    pub action: String,
    pub entity_type: String,
    pub entity_id: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub ip_address: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct AuditRow {
    id: String,
    actor_id: Option<String>,
    actor_name: Option<String>,
    action: String,
    entity_type: String,
    entity_id: String,
    old_value: Option<String>,
    new_value: Option<String>,
    ip_address: Option<String>,
    created_at: NaiveDateTime,
}

impl AuditService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Record an audit entry. Never fails the caller — if the INSERT
    /// errors, we log it and move on. The primary operation has already
    /// happened; dropping an audit row is strictly better than reverting
    /// or 500-ing the user.
    pub async fn log(
        &self,
        actor_id: Option<Uuid>,
        action: &str,
        entity_type: &str,
        entity_id: &str,
        old_value: Option<&str>,
        new_value: Option<&str>,
        ip_address: Option<&str>,
    ) {
        let id = Uuid::new_v4().to_string();
        let actor = actor_id.map(|u| u.to_string());
        let result = sqlx::query(
            "INSERT INTO audit_logs \
             (id, actor_id, action, entity_type, entity_id, old_value, new_value, ip_address) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&actor)
        .bind(action)
        .bind(entity_type)
        .bind(entity_id)
        .bind(old_value)
        .bind(new_value)
        .bind(ip_address)
        .execute(&self.pool)
        .await;

        if let Err(e) = result {
            tracing::error!(
                "Failed to write audit log (action={}, entity={}:{}): {}",
                action,
                entity_type,
                entity_id,
                e
            );
        }
    }

    /// Delete audit entries older than `retention_days`. Returns the
    /// number of rows removed. Intended to be called periodically by
    /// a background task.
    pub async fn prune_older_than(&self, retention_days: i64) -> Result<u64> {
        let days = retention_days.clamp(1, 3650); // refuse both absurdly short and absurdly long
        let result = sqlx::query(
            "DELETE FROM audit_logs WHERE created_at < datetime('now', '-' || ? || ' days')",
        )
        .bind(days)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Fetch the N most recent audit entries, joined with member for
    /// the actor's display name.
    pub async fn recent(&self, limit: i64) -> Result<Vec<AuditEntry>> {
        let rows = sqlx::query_as::<_, AuditRow>(
            "SELECT al.id, al.actor_id, m.full_name AS actor_name, \
                    al.action, al.entity_type, al.entity_id, \
                    al.old_value, al.new_value, al.ip_address, al.created_at \
             FROM audit_logs al \
             LEFT JOIN members m ON m.id = al.actor_id \
             ORDER BY al.created_at DESC \
             LIMIT ?",
        )
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| AuditEntry {
                id: Uuid::parse_str(&r.id).unwrap_or_default(),
                actor_id: r.actor_id.and_then(|s| Uuid::parse_str(&s).ok()),
                actor_name: r.actor_name,
                action: r.action,
                entity_type: r.entity_type,
                entity_id: r.entity_id,
                old_value: r.old_value,
                new_value: r.new_value,
                ip_address: r.ip_address,
                created_at: DateTime::from_naive_utc_and_offset(r.created_at, Utc),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{Executor, Row};

    async fn fresh_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .after_connect(|conn, _| {
                Box::pin(async move {
                    conn.execute("PRAGMA foreign_keys = ON").await?;
                    Ok(())
                })
            })
            .connect("sqlite::memory:")
            .await
            .expect(":memory:");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrate");
        pool
    }

    async fn insert_row(pool: &SqlitePool) {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO audit_logs \
             (id, action, entity_type, entity_id) \
             VALUES (?, 'test.action', 'test', 'test-entity')",
        )
        .bind(&id)
        .execute(pool)
        .await
        .expect("insert audit row");
    }

    async fn count_rows(pool: &SqlitePool) -> i64 {
        let row = sqlx::query("SELECT COUNT(*) as c FROM audit_logs")
            .fetch_one(pool)
            .await
            .expect("count");
        row.get::<i64, _>("c")
    }

    #[tokio::test]
    async fn prune_older_than_clamps_below_one_day() {
        let pool = fresh_pool().await;
        insert_row(&pool).await;
        let svc = AuditService::new(pool.clone());

        let removed = svc.prune_older_than(0).await.expect("prune returns Ok");
        assert_eq!(
            removed, 0,
            "clamp must lift 0 to 1 day so today's row is not deleted"
        );
        assert_eq!(
            count_rows(&pool).await,
            1,
            "row inserted now must NOT be wiped"
        );
    }

    #[tokio::test]
    async fn prune_older_than_clamps_above_3650() {
        let pool = fresh_pool().await;
        let svc = AuditService::new(pool.clone());

        let removed = svc
            .prune_older_than(i64::MAX)
            .await
            .expect("prune must clamp i64::MAX and not propagate SQL overflow");
        assert_eq!(
            removed, 0,
            "empty table — nothing to delete after clamp to 3650"
        );
    }

    #[tokio::test]
    async fn recent_clamps_limit_below_one() {
        let pool = fresh_pool().await;
        for _ in 0..3 {
            insert_row(&pool).await;
        }
        let svc = AuditService::new(pool.clone());

        let rows = svc.recent(0).await.expect("recent returns Ok");
        assert_eq!(rows.len(), 1, "clamp must lift LIMIT 0 to LIMIT 1");
    }

    #[tokio::test]
    async fn recent_clamps_limit_above_500() {
        let pool = fresh_pool().await;
        let mut tx = pool.begin().await.expect("begin tx");
        for _ in 0..600 {
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO audit_logs \
                 (id, action, entity_type, entity_id) \
                 VALUES (?, 'test.action', 'test', 'test-entity')",
            )
            .bind(&id)
            .execute(&mut *tx)
            .await
            .expect("insert");
        }
        tx.commit().await.expect("commit");
        let svc = AuditService::new(pool.clone());

        let rows = svc.recent(10_000).await.expect("recent returns Ok");
        assert_eq!(rows.len(), 500, "clamp must hold LIMIT at 500");
    }
}
