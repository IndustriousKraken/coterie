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
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
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
                action, entity_type, entity_id, e
            );
        }
    }

    /// Delete audit entries older than `retention_days`. Returns the
    /// number of rows removed. Intended to be called periodically by
    /// a background task.
    pub async fn prune_older_than(&self, retention_days: i64) -> Result<u64> {
        let days = retention_days.clamp(1, 3650); // refuse both absurdly short and absurdly long
        let result = sqlx::query(
            "DELETE FROM audit_logs WHERE created_at < datetime('now', '-' || ? || ' days')"
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
             LIMIT ?"
        )
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| AuditEntry {
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
        }).collect())
    }
}
