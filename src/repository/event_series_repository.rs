//! Persistence for `event_series` rows. Exists alongside (not inside)
//! `EventRepository` because the two have different lifecycles —
//! occurrences are CRUDed per-row by handlers; series rows are
//! touched only by the create / end-series / horizon-extend code paths.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    domain::EventSeries,
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
            id: Uuid::parse_str(&self.id)
                .map_err(|e| AppError::Database(e.to_string()))?,
            rule_kind: self.rule_kind,
            rule_json: self.rule_json,
            until_date: self.until_date.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            materialized_through: DateTime::from_naive_utc_and_offset(self.materialized_through, Utc),
            created_by: Uuid::parse_str(&self.created_by)
                .map_err(|e| AppError::Database(e.to_string()))?,
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
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(series.id).await?.ok_or_else(|| {
            AppError::Database("event_series row vanished after insert".to_string())
        })
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
        .map_err(|e| AppError::Database(e.to_string()))?;

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
        .map_err(|e| AppError::Database(e.to_string()))?;

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
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    async fn set_until_date(&self, id: Uuid, until: DateTime<Utc>) -> Result<()> {
        sqlx::query(
            "UPDATE event_series SET until_date = ?, updated_at = ? WHERE id = ?",
        )
        .bind(until.naive_utc())
        .bind(Utc::now().naive_utc())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        // ON DELETE CASCADE on `events.series_id` removes occurrences
        // automatically.
        sqlx::query("DELETE FROM event_series WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}
