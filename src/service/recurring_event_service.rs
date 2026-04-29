//! Materialization for recurring events. Owns the "given a series + a
//! template + a target horizon, generate the missing occurrence rows"
//! operation. Stateless — instances are cheap.
//!
//! Two callers:
//!   - The admin "create event with recurrence" handler — calls
//!     `create_series_with_initial_materialization` to set up the
//!     series row + the first 12 months of occurrences.
//!   - The daily horizon-extension job — calls
//!     `extend_horizon_for_active_series` to roll the
//!     materialized window forward to (today + 12 months).

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    domain::{Event, EventSeries, Recurrence, generate_occurrences},
    error::{AppError, Result},
    repository::{EventRepository, EventSeriesRepository},
};

/// 12 months. Operator-facing, not RFC-driven — we want the calendar
/// to always show "the next year of meetings" without operators
/// manually extending. The daily job rolls this forward; on creation,
/// we materialize once at this depth.
pub const DEFAULT_HORIZON: Duration = Duration::weeks(52);

pub struct RecurringEventService {
    event_repo: Arc<dyn EventRepository>,
    series_repo: Arc<dyn EventSeriesRepository>,
    /// Held alongside the trait-objects so the few queries that don't
    /// merit a trait method (e.g. "min start_time across this series")
    /// can be served directly. Cheap (Arc'd internally).
    pool: SqlitePool,
}

/// What `create_series_with_initial_materialization` returns.
pub struct CreatedSeries {
    pub series: EventSeries,
    /// All occurrences inserted during the initial materialization,
    /// in `start_time` order. The first one is the "anchor"
    /// occurrence (matching `template.start_time` exactly) — useful
    /// for redirecting the admin to that page after creation.
    pub occurrences: Vec<Event>,
}

impl RecurringEventService {
    pub fn new(
        event_repo: Arc<dyn EventRepository>,
        series_repo: Arc<dyn EventSeriesRepository>,
        pool: SqlitePool,
    ) -> Self {
        Self { event_repo, series_repo, pool }
    }

    /// Persist a series + its first 12 months of occurrences.
    ///
    /// `template` is treated as the prototype for every occurrence:
    /// title, description, type, visibility, location,
    /// max_attendees, rsvp_required, image_url all carry over.
    /// `template.start_time` is the anchor (defines time-of-day and
    /// the first occurrence).
    ///
    /// `until_date` caps the series. The materializer stops here even
    /// if it's earlier than 12 months out. `None` = open-ended.
    pub async fn create_series_with_initial_materialization(
        &self,
        rule: Recurrence,
        template: Event,
        until_date: Option<DateTime<Utc>>,
        created_by: Uuid,
    ) -> Result<CreatedSeries> {
        rule.validate().map_err(|m| AppError::BadRequest(m.to_string()))?;

        let now = Utc::now();
        let target_horizon = (now + DEFAULT_HORIZON)
            .min(until_date.unwrap_or(DateTime::<Utc>::MAX_UTC));

        let rule_json = serde_json::to_string(&rule)
            .map_err(|e| AppError::Internal(format!("rule serialize: {}", e)))?;

        // The series's anchor for occurrence generation is
        // template.start_time. We materialize from anchor up to
        // target_horizon — generate_occurrences excludes anything
        // before `from` so passing anchor as `from` keeps the first
        // occurrence in the result.
        let occurrence_times = generate_occurrences(
            template.start_time,
            &rule,
            template.start_time,
            target_horizon,
        );

        if occurrence_times.is_empty() {
            return Err(AppError::BadRequest(
                "Recurrence rule produced no occurrences before the cutoff".to_string()
            ));
        }

        let series_id = Uuid::new_v4();
        let materialized_through = *occurrence_times.last().expect("non-empty");

        let series = EventSeries {
            id: series_id,
            rule_kind: rule.kind_str().to_string(),
            rule_json,
            until_date,
            materialized_through,
            created_by,
            created_at: now,
            updated_at: now,
        };
        let series = self.series_repo.create(series).await?;

        // Insert each occurrence, copying the template fields.
        // duration carries over: end_time is preserved as a Duration
        // offset from start_time.
        let original_duration = template.end_time
            .map(|e| e - template.start_time);
        let mut inserted = Vec::with_capacity(occurrence_times.len());
        for (idx, start) in occurrence_times.iter().enumerate() {
            let occurrence = Event {
                id: Uuid::new_v4(),
                title: template.title.clone(),
                description: template.description.clone(),
                event_type: template.event_type.clone(),
                event_type_id: template.event_type_id,
                visibility: template.visibility.clone(),
                start_time: *start,
                end_time: original_duration.map(|d| *start + d),
                location: template.location.clone(),
                max_attendees: template.max_attendees,
                rsvp_required: template.rsvp_required,
                image_url: template.image_url.clone(),
                created_by,
                created_at: now,
                updated_at: now,
                series_id: Some(series_id),
                occurrence_index: Some((idx + 1) as i32),
            };
            inserted.push(self.event_repo.create(occurrence).await?);
        }

        Ok(CreatedSeries { series, occurrences: inserted })
    }

    /// Materialize occurrences from `series.materialized_through`
    /// (exclusive) up to `target` (exclusive), capped at `until_date`.
    /// Idempotent: calling repeatedly with a non-advancing target is a
    /// no-op. Returns the count inserted.
    pub async fn extend_horizon(
        &self,
        series: &EventSeries,
        target: DateTime<Utc>,
    ) -> Result<u64> {
        let cap = series.until_date.unwrap_or(DateTime::<Utc>::MAX_UTC);
        let target = target.min(cap);
        if target <= series.materialized_through {
            return Ok(0);
        }

        let rule: Recurrence = serde_json::from_str(&series.rule_json)
            .map_err(|e| AppError::Internal(format!("rule parse: {}", e)))?;

        // Anchor: we re-derive from the FIRST occurrence so the
        // generator stays consistent across calls. Pulling the anchor
        // from the series row would require persisting it — using
        // events.start_time avoids that extra column.
        let first_occurrence_start: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
            "SELECT MIN(start_time) FROM events WHERE series_id = ?",
        )
        .bind(series.id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        let anchor = match first_occurrence_start {
            Some(naive) => DateTime::from_naive_utc_and_offset(naive, Utc),
            None => {
                // No occurrences left — series exists but everything
                // was deleted. Don't spontaneously regenerate; that's
                // surprising behavior. Operator should re-create.
                return Ok(0);
            }
        };

        // Generate strictly after `materialized_through` to avoid
        // colliding with already-inserted rows. Using a tiny epsilon
        // works for second-resolution timestamps; +1 second is fine.
        let from = series.materialized_through + Duration::seconds(1);
        let new_times = generate_occurrences(anchor, &rule, from, target);
        if new_times.is_empty() {
            // Still bump materialized_through to `target` so the next
            // run doesn't re-scan the same window. Otherwise an
            // empty-but-advancing window is a no-op forever.
            self.series_repo.set_materialized_through(series.id, target).await?;
            return Ok(0);
        }

        let next_index = self.event_repo
            .max_occurrence_index_for_series(series.id).await?
            .unwrap_or(0);

        // Use the first existing occurrence as the prototype for
        // titles/etc — the user might have edited the template since
        // creation.
        let prototype = self.fetch_series_prototype(series.id).await?;

        let mut count = 0u64;
        for (i, start) in new_times.iter().enumerate() {
            let mut occ = prototype.clone();
            occ.id = Uuid::new_v4();
            // Preserve duration of the prototype.
            let dur = prototype.end_time.map(|e| e - prototype.start_time);
            occ.start_time = *start;
            occ.end_time = dur.map(|d| *start + d);
            occ.created_at = Utc::now();
            occ.updated_at = Utc::now();
            occ.occurrence_index = Some(next_index + (i as i32) + 1);
            self.event_repo.create(occ).await?;
            count += 1;
        }

        let new_through = *new_times.last().expect("non-empty");
        self.series_repo.set_materialized_through(
            series.id,
            new_through.max(target),
        ).await?;
        Ok(count)
    }

    /// Roll every active series forward to (now + DEFAULT_HORIZON).
    /// Called by the daily background job. Errors on individual
    /// series are logged and skipped — a malformed rule for one
    /// series shouldn't stop the rest.
    pub async fn extend_horizon_for_active_series(&self) -> Result<u64> {
        let now = Utc::now();
        let target = now + DEFAULT_HORIZON;
        let active = self.series_repo.list_active(now).await?;

        let mut total = 0u64;
        for series in active {
            match self.extend_horizon(&series, target).await {
                Ok(n) => total += n,
                Err(e) => tracing::error!(
                    "horizon-extend failed for series {}: {}",
                    series.id, e,
                ),
            }
        }
        Ok(total)
    }

    /// Internal: fetch any one occurrence and use it as the prototype
    /// for new occurrences. We pick the most recent past occurrence —
    /// that's the closest reflection of the operator's intent.
    async fn fetch_series_prototype(&self, series_id: Uuid) -> Result<Event> {
        // Read directly via sqlx since the trait doesn't have a
        // "find one in series" method (and adding one for this single
        // case isn't worth it).
        use sqlx::Row;
        let row = sqlx::query(
            "SELECT id FROM events WHERE series_id = ? ORDER BY start_time ASC LIMIT 1",
        )
        .bind(series_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        let id_str: String = row
            .ok_or_else(|| AppError::NotFound("series has no occurrences".to_string()))?
            .try_get("id")
            .map_err(|e| AppError::Database(e.to_string()))?;
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| AppError::Database(e.to_string()))?;
        self.event_repo.find_by_id(id).await?.ok_or_else(|| {
            AppError::Database("prototype occurrence vanished".to_string())
        })
    }

}
