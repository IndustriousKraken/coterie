//! Persistence for `event_series` rows. Exists alongside (not inside)
//! `EventRepository` because the two have different lifecycles —
//! occurrences are CRUDed per-row by handlers; series rows are
//! touched only by the create / end-series / horizon-extend code paths.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    domain::{EventSeries, OccurrenceException, OccurrenceExceptionKind},
    error::{AppError, Result},
};

#[async_trait]
pub trait EventSeriesRepository: Send + Sync {
    async fn create(&self, series: EventSeries) -> Result<EventSeries>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<EventSeries>>;
    /// All series whose `until_date` is null OR strictly greater than
    /// `now`. The horizon-extend job iterates these.
    async fn list_active(&self, now: DateTime<Utc>) -> Result<Vec<EventSeries>>;
    /// Bump `materialized_through` to the latest occurrence we just
    /// generated. Touches `updated_at` too.
    async fn set_materialized_through(&self, id: Uuid, through: DateTime<Utc>) -> Result<()>;
    /// Cap the series at `until` (also touches updated_at). Used by
    /// the "end the series after this date" admin action.
    async fn set_until_date(&self, id: Uuid, until: DateTime<Utc>) -> Result<()>;
    async fn delete(&self, id: Uuid) -> Result<()>;

    // ---- Per-occurrence exceptions ------------------------------------
    //
    // Exception rows are owned by the series — they cascade-delete with
    // the series row via FK ON DELETE CASCADE. Each method below operates
    // on the `event_series_exceptions` table; the events row itself is
    // mutated by the service layer (cancel deletes, override updates).

    /// Insert (or replace) an exception row. The primary key
    /// `(series_id, occurrence_index)` means a second insert for the
    /// same pair overwrites — caller relies on this for "set the
    /// override even if a previous one existed."
    async fn insert_exception(&self, exception: OccurrenceException) -> Result<()>;
    /// Remove an exception row. No-op if absent.
    async fn delete_exception(&self, series_id: Uuid, occurrence_index: i32) -> Result<()>;
    /// Look up a single exception by `(series_id, occurrence_index)`.
    async fn find_exception(
        &self,
        series_id: Uuid,
        occurrence_index: i32,
    ) -> Result<Option<OccurrenceException>>;
    /// All exceptions for one series, ordered by occurrence_index ASC.
    /// Used by the materializer's batch path and by the admin UI.
    async fn list_exceptions_for_series(&self, series_id: Uuid)
        -> Result<Vec<OccurrenceException>>;
}

pub struct SqliteEventSeriesRepository {
    pool: SqlitePool,
}

impl SqliteEventSeriesRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct SeriesRow {
    id: String,
    rule_kind: String,
    rule_json: String,
    until_date: Option<NaiveDateTime>,
    materialized_through: NaiveDateTime,
    created_by: String,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

impl SeriesRow {
    fn into_domain(self) -> Result<EventSeries> {
        Ok(EventSeries {
            id: Uuid::parse_str(&self.id).map_err(|e| AppError::Internal(e.to_string()))?,
            rule_kind: self.rule_kind,
            rule_json: self.rule_json,
            until_date: self
                .until_date
                .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            materialized_through: DateTime::from_naive_utc_and_offset(
                self.materialized_through,
                Utc,
            ),
            created_by: Uuid::parse_str(&self.created_by)
                .map_err(|e| AppError::Internal(e.to_string()))?,
            created_at: DateTime::from_naive_utc_and_offset(self.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(self.updated_at, Utc),
        })
    }
}

#[async_trait]
impl EventSeriesRepository for SqliteEventSeriesRepository {
    async fn create(&self, series: EventSeries) -> Result<EventSeries> {
        let id_str = series.id.to_string();
        let until_naive = series.until_date.map(|d| d.naive_utc());
        let through_naive = series.materialized_through.naive_utc();
        let created_by_str = series.created_by.to_string();
        let now = Utc::now().naive_utc();

        sqlx::query(
            "INSERT INTO event_series \
                (id, rule_kind, rule_json, until_date, materialized_through, \
                 created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id_str)
        .bind(&series.rule_kind)
        .bind(&series.rule_json)
        .bind(until_naive)
        .bind(through_naive)
        .bind(&created_by_str)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(series.id)
            .await?
            .ok_or_else(|| AppError::Internal("event_series row vanished after insert".to_string()))
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<EventSeries>> {
        let row = sqlx::query_as::<_, SeriesRow>(
            "SELECT id, rule_kind, rule_json, until_date, materialized_through, \
                    created_by, created_at, updated_at \
             FROM event_series WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        row.map(SeriesRow::into_domain).transpose()
    }

    async fn list_active(&self, now: DateTime<Utc>) -> Result<Vec<EventSeries>> {
        let rows = sqlx::query_as::<_, SeriesRow>(
            "SELECT id, rule_kind, rule_json, until_date, materialized_through, \
                    created_by, created_at, updated_at \
             FROM event_series \
             WHERE until_date IS NULL OR until_date > ? \
             ORDER BY created_at ASC",
        )
        .bind(now.naive_utc())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter().map(SeriesRow::into_domain).collect()
    }

    async fn set_materialized_through(&self, id: Uuid, through: DateTime<Utc>) -> Result<()> {
        sqlx::query(
            "UPDATE event_series SET materialized_through = ?, updated_at = ? WHERE id = ?",
        )
        .bind(through.naive_utc())
        .bind(Utc::now().naive_utc())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    async fn set_until_date(&self, id: Uuid, until: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE event_series SET until_date = ?, updated_at = ? WHERE id = ?")
            .bind(until.naive_utc())
            .bind(Utc::now().naive_utc())
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        // ON DELETE CASCADE on `events.series_id` removes occurrences
        // automatically.
        sqlx::query("DELETE FROM event_series WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn insert_exception(&self, exception: OccurrenceException) -> Result<()> {
        sqlx::query(
            "INSERT INTO event_series_exceptions \
                (series_id, occurrence_index, kind, override_payload, \
                 created_at, created_by, audit_reason) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(series_id, occurrence_index) DO UPDATE SET \
                kind = excluded.kind, \
                override_payload = excluded.override_payload, \
                created_at = excluded.created_at, \
                created_by = excluded.created_by, \
                audit_reason = excluded.audit_reason",
        )
        .bind(exception.series_id.to_string())
        .bind(exception.occurrence_index)
        .bind(exception.kind.as_str())
        .bind(&exception.override_payload)
        .bind(exception.created_at.naive_utc())
        .bind(exception.created_by.to_string())
        .bind(&exception.audit_reason)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    async fn delete_exception(&self, series_id: Uuid, occurrence_index: i32) -> Result<()> {
        sqlx::query(
            "DELETE FROM event_series_exceptions \
             WHERE series_id = ? AND occurrence_index = ?",
        )
        .bind(series_id.to_string())
        .bind(occurrence_index)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    async fn find_exception(
        &self,
        series_id: Uuid,
        occurrence_index: i32,
    ) -> Result<Option<OccurrenceException>> {
        let row = sqlx::query_as::<_, ExceptionRow>(
            "SELECT series_id, occurrence_index, kind, override_payload, \
                    created_at, created_by, audit_reason \
             FROM event_series_exceptions \
             WHERE series_id = ? AND occurrence_index = ?",
        )
        .bind(series_id.to_string())
        .bind(occurrence_index)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        row.map(ExceptionRow::into_domain).transpose()
    }

    async fn list_exceptions_for_series(
        &self,
        series_id: Uuid,
    ) -> Result<Vec<OccurrenceException>> {
        let rows = sqlx::query_as::<_, ExceptionRow>(
            "SELECT series_id, occurrence_index, kind, override_payload, \
                    created_at, created_by, audit_reason \
             FROM event_series_exceptions \
             WHERE series_id = ? \
             ORDER BY occurrence_index ASC",
        )
        .bind(series_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter().map(ExceptionRow::into_domain).collect()
    }
}

#[derive(sqlx::FromRow)]
struct ExceptionRow {
    series_id: String,
    occurrence_index: i32,
    kind: String,
    override_payload: Option<String>,
    created_at: NaiveDateTime,
    created_by: String,
    audit_reason: Option<String>,
}

impl ExceptionRow {
    fn into_domain(self) -> Result<OccurrenceException> {
        let kind = OccurrenceExceptionKind::parse(&self.kind).ok_or_else(|| {
            AppError::Internal(format!(
                "unknown event_series_exceptions.kind: {}",
                self.kind
            ))
        })?;
        Ok(OccurrenceException {
            series_id: Uuid::parse_str(&self.series_id)
                .map_err(|e| AppError::Internal(e.to_string()))?,
            occurrence_index: self.occurrence_index,
            kind,
            override_payload: self.override_payload,
            created_at: DateTime::from_naive_utc_and_offset(self.created_at, Utc),
            created_by: Uuid::parse_str(&self.created_by)
                .map_err(|e| AppError::Internal(e.to_string()))?,
            audit_reason: self.audit_reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CreateMemberRequest, OccurrenceOverride};
    use crate::repository::{MemberRepository, SqliteMemberRepository};
    use chrono::Duration;
    use sqlx::{Executor, SqlitePool};

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

    async fn make_member(pool: &SqlitePool) -> Uuid {
        let repo = SqliteMemberRepository::new(pool.clone());
        let m = repo
            .create(CreateMemberRequest {
                email: format!("e-{}@example.com", Uuid::new_v4()),
                username: format!("u_{}", Uuid::new_v4().simple()),
                full_name: "Member".to_string(),
                password: "p4ssword_long_enough".to_string(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .unwrap();
        m.id
    }

    async fn make_series(pool: &SqlitePool, created_by: Uuid) -> Uuid {
        let repo = SqliteEventSeriesRepository::new(pool.clone());
        let now = Utc::now();
        let series = EventSeries {
            id: Uuid::new_v4(),
            rule_kind: "weekly_by_day".to_string(),
            rule_json: r#"{"kind":"weekly_by_day","interval":1,"weekdays":["mon"]}"#.to_string(),
            until_date: None,
            materialized_through: now + Duration::weeks(52),
            created_by,
            created_at: now,
            updated_at: now,
        };
        repo.create(series).await.unwrap().id
    }

    #[tokio::test]
    async fn exception_insert_find_delete_roundtrip() {
        let pool = fresh_pool().await;
        let member = make_member(&pool).await;
        let series = make_series(&pool, member).await;
        let repo = SqliteEventSeriesRepository::new(pool.clone());

        let ex = OccurrenceException {
            series_id: series,
            occurrence_index: 5,
            kind: OccurrenceExceptionKind::Cancelled,
            override_payload: None,
            created_at: Utc::now(),
            created_by: member,
            audit_reason: Some("holiday".to_string()),
        };
        repo.insert_exception(ex).await.unwrap();

        let found = repo.find_exception(series, 5).await.unwrap().unwrap();
        assert_eq!(found.occurrence_index, 5);
        assert_eq!(found.kind, OccurrenceExceptionKind::Cancelled);
        assert_eq!(found.audit_reason.as_deref(), Some("holiday"));

        repo.delete_exception(series, 5).await.unwrap();
        assert!(repo.find_exception(series, 5).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn exception_list_returns_all_for_series() {
        let pool = fresh_pool().await;
        let member = make_member(&pool).await;
        let series = make_series(&pool, member).await;
        let repo = SqliteEventSeriesRepository::new(pool.clone());

        for idx in [2_i32, 4, 1, 3] {
            repo.insert_exception(OccurrenceException {
                series_id: series,
                occurrence_index: idx,
                kind: OccurrenceExceptionKind::Cancelled,
                override_payload: None,
                created_at: Utc::now(),
                created_by: member,
                audit_reason: None,
            })
            .await
            .unwrap();
        }
        let list = repo.list_exceptions_for_series(series).await.unwrap();
        assert_eq!(
            list.iter().map(|e| e.occurrence_index).collect::<Vec<_>>(),
            vec![1, 2, 3, 4],
        );
    }

    #[tokio::test]
    async fn exception_insert_overwrites_on_conflict() {
        let pool = fresh_pool().await;
        let member = make_member(&pool).await;
        let series = make_series(&pool, member).await;
        let repo = SqliteEventSeriesRepository::new(pool.clone());

        repo.insert_exception(OccurrenceException {
            series_id: series,
            occurrence_index: 1,
            kind: OccurrenceExceptionKind::Cancelled,
            override_payload: None,
            created_at: Utc::now(),
            created_by: member,
            audit_reason: Some("first".to_string()),
        })
        .await
        .unwrap();

        let payload = serde_json::to_string(&OccurrenceOverride {
            location: Some("Room Z".to_string()),
            ..Default::default()
        })
        .unwrap();
        repo.insert_exception(OccurrenceException {
            series_id: series,
            occurrence_index: 1,
            kind: OccurrenceExceptionKind::Overridden,
            override_payload: Some(payload.clone()),
            created_at: Utc::now(),
            created_by: member,
            audit_reason: Some("changed mind".to_string()),
        })
        .await
        .unwrap();

        let found = repo.find_exception(series, 1).await.unwrap().unwrap();
        assert_eq!(found.kind, OccurrenceExceptionKind::Overridden);
        assert_eq!(found.override_payload.as_deref(), Some(payload.as_str()));
        assert_eq!(found.audit_reason.as_deref(), Some("changed mind"));
    }
}
