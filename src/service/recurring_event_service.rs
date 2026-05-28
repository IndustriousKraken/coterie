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
    domain::{
        generate_occurrences, Event, EventSeries, OccurrenceExceptionKind, OccurrenceOverride,
        Recurrence,
    },
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
        Self {
            event_repo,
            series_repo,
            pool,
        }
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
        rule.validate()
            .map_err(|m| AppError::BadRequest(m.to_string()))?;

        let now = Utc::now();
        let target_horizon =
            (now + DEFAULT_HORIZON).min(until_date.unwrap_or(DateTime::<Utc>::MAX_UTC));

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
                "Recurrence rule produced no occurrences before the cutoff".to_string(),
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
        //
        // Consult the exception table per-index — on a brand-new series
        // this is virtually always empty, but the same code path runs
        // for the "create, then immediately cancel one, then call
        // materializer again" scenario covered by tests.
        let original_duration = template.end_time.map(|e| e - template.start_time);
        let mut inserted = Vec::with_capacity(occurrence_times.len());
        for (idx, start) in occurrence_times.iter().enumerate() {
            let occurrence_index = (idx + 1) as i32;
            let exception = self
                .series_repo
                .find_exception(series_id, occurrence_index)
                .await?;

            if matches!(
                exception.as_ref().map(|e| e.kind),
                Some(OccurrenceExceptionKind::Cancelled),
            ) {
                continue;
            }

            let mut occurrence = Event {
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
                occurrence_index: Some(occurrence_index),
            };

            if let Some(ex) = exception {
                if ex.kind == OccurrenceExceptionKind::Overridden {
                    apply_payload(&ex.override_payload, &mut occurrence)?;
                }
            }

            inserted.push(self.event_repo.create(occurrence).await?);
        }

        Ok(CreatedSeries {
            series,
            occurrences: inserted,
        })
    }

    /// Materialize occurrences from `series.materialized_through`
    /// (exclusive) up to `target` (exclusive), capped at `until_date`.
    /// Idempotent: calling repeatedly with a non-advancing target is a
    /// no-op. Returns the count inserted.
    pub async fn extend_horizon(&self, series: &EventSeries, target: DateTime<Utc>) -> Result<u64> {
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
        let first_occurrence_start: Option<chrono::NaiveDateTime> =
            sqlx::query_scalar("SELECT MIN(start_time) FROM events WHERE series_id = ?")
                .bind(series.id.to_string())
                .fetch_one(&self.pool)
                .await
                .map_err(AppError::Database)?;

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
            self.series_repo
                .set_materialized_through(series.id, target)
                .await?;
            return Ok(0);
        }

        let next_index = self
            .event_repo
            .max_occurrence_index_for_series(series.id)
            .await?
            .unwrap_or(0);

        // Use the first existing occurrence as the prototype for
        // titles/etc — the user might have edited the template since
        // creation.
        let prototype = self.fetch_series_prototype(series.id).await?;

        let mut count = 0u64;
        for (i, start) in new_times.iter().enumerate() {
            let occurrence_index = next_index + (i as i32) + 1;
            let exception = self
                .series_repo
                .find_exception(series.id, occurrence_index)
                .await?;

            if matches!(
                exception.as_ref().map(|e| e.kind),
                Some(OccurrenceExceptionKind::Cancelled),
            ) {
                continue;
            }

            let mut occ = prototype.clone();
            occ.id = Uuid::new_v4();
            // Preserve duration of the prototype.
            let dur = prototype.end_time.map(|e| e - prototype.start_time);
            occ.start_time = *start;
            occ.end_time = dur.map(|d| *start + d);
            occ.created_at = Utc::now();
            occ.updated_at = Utc::now();
            occ.occurrence_index = Some(occurrence_index);

            if let Some(ex) = exception {
                if ex.kind == OccurrenceExceptionKind::Overridden {
                    apply_payload(&ex.override_payload, &mut occ)?;
                }
            }

            self.event_repo.create(occ).await?;
            count += 1;
        }

        let new_through = *new_times.last().expect("non-empty");
        self.series_repo
            .set_materialized_through(series.id, new_through.max(target))
            .await?;
        Ok(count)
    }

    /// Compute the start_time for a specific `occurrence_index` of a
    /// series. Used to re-create a single cancelled occurrence on
    /// restore.
    ///
    /// Anchor inference matches `extend_horizon`'s approach: take
    /// `MIN(start_time)` over the series's existing events as the
    /// anchor. If the series has no events at all this fails — there's
    /// nothing to extrapolate from. If occurrence 1 itself has been
    /// cancelled the inferred anchor is slightly off, but cancelling
    /// the very first occurrence of a series and then restoring a
    /// later index is rare enough that v1 accepts the edge case rather
    /// than schema-changing to store the anchor explicitly.
    pub async fn compute_occurrence_start_time(
        &self,
        series: &EventSeries,
        occurrence_index: i32,
    ) -> Result<chrono::DateTime<Utc>> {
        if occurrence_index < 1 {
            return Err(AppError::BadRequest(
                "occurrence_index must be >= 1".to_string(),
            ));
        }

        let rule: Recurrence = serde_json::from_str(&series.rule_json)
            .map_err(|e| AppError::Internal(format!("rule parse: {}", e)))?;

        let first_occurrence_start: Option<chrono::NaiveDateTime> =
            sqlx::query_scalar("SELECT MIN(start_time) FROM events WHERE series_id = ?")
                .bind(series.id.to_string())
                .fetch_one(&self.pool)
                .await
                .map_err(AppError::Database)?;

        let anchor = match first_occurrence_start {
            Some(naive) => DateTime::from_naive_utc_and_offset(naive, Utc),
            None => {
                return Err(AppError::Internal(
                    "cannot infer series anchor — no occurrences exist".to_string(),
                ));
            }
        };

        // Materialize a far-enough horizon to be confident the index is
        // within reach. We cap generate_occurrences internally at 10_000
        // entries — for a weekly rule that's ~190 years; for monthly
        // ~830 years. Both well past any realistic occurrence_index.
        let horizon = anchor + Duration::weeks(52 * 200);
        let times = generate_occurrences(anchor, &rule, anchor, horizon);
        let idx = (occurrence_index - 1) as usize;
        times.get(idx).copied().ok_or_else(|| {
            AppError::BadRequest(format!(
                "occurrence_index {} is beyond the series's generated occurrences",
                occurrence_index,
            ))
        })
    }

    /// Re-create a single occurrence row for a series at a specific
    /// index, applying any pending `Overridden` exception. Used by the
    /// restore-cancelled service path.
    ///
    /// Returns the inserted event. If a row at this `(series, index)`
    /// already exists the caller should not call this — there's no
    /// resurrect-or-overwrite semantic baked in.
    pub async fn materialize_single_occurrence(
        &self,
        series: &EventSeries,
        occurrence_index: i32,
    ) -> Result<Event> {
        let start = self
            .compute_occurrence_start_time(series, occurrence_index)
            .await?;
        let prototype = self.fetch_series_prototype(series.id).await?;
        let duration = prototype.end_time.map(|e| e - prototype.start_time);

        let mut occ = prototype.clone();
        occ.id = Uuid::new_v4();
        occ.start_time = start;
        occ.end_time = duration.map(|d| start + d);
        occ.created_at = Utc::now();
        occ.updated_at = Utc::now();
        occ.occurrence_index = Some(occurrence_index);

        if let Some(ex) = self
            .series_repo
            .find_exception(series.id, occurrence_index)
            .await?
        {
            if ex.kind == OccurrenceExceptionKind::Overridden {
                apply_payload(&ex.override_payload, &mut occ)?;
            }
        }

        self.event_repo.create(occ).await
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
                Err(e) => tracing::error!("horizon-extend failed for series {}: {}", series.id, e,),
            }
        }
        Ok(total)
    }

    /// Internal: fetch any one occurrence and use it as the prototype
    /// for new occurrences. We pick the earliest existing occurrence —
    /// stable across cancel/restore cycles and the closest reflection
    /// of the series's intended fields.
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
        .map_err(AppError::Database)?;

        let id_str: String = row
            .ok_or_else(|| AppError::NotFound("series has no occurrences".to_string()))?
            .try_get("id")
            .map_err(AppError::Database)?;
        let id = Uuid::parse_str(&id_str).map_err(|e| AppError::Internal(e.to_string()))?;
        self.event_repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::Internal("prototype occurrence vanished".to_string()))
    }
}

/// Parse a stored `override_payload` JSON blob and apply its non-null
/// fields onto `target`. `None` payload (shouldn't happen for an
/// Overridden exception, but handled defensively) is a no-op.
fn apply_payload(payload: &Option<String>, target: &mut Event) -> Result<()> {
    let Some(json) = payload.as_deref() else {
        return Ok(());
    };
    let ov: OccurrenceOverride = serde_json::from_str(json)
        .map_err(|e| AppError::Internal(format!("override_payload parse: {}", e)))?;
    ov.apply(target);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{CreateMemberRequest, EventType, EventVisibility, WeekdayCode},
        repository::{
            MemberRepository, SqliteEventRepository, SqliteEventSeriesRepository,
            SqliteMemberRepository,
        },
    };
    use chrono::{Datelike, Weekday};
    use sqlx::Executor;

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

    fn make_service(pool: SqlitePool) -> RecurringEventService {
        let event_repo: Arc<dyn EventRepository> =
            Arc::new(SqliteEventRepository::new(pool.clone()));
        let series_repo: Arc<dyn EventSeriesRepository> =
            Arc::new(SqliteEventSeriesRepository::new(pool.clone()));
        RecurringEventService::new(event_repo, series_repo, pool)
    }

    async fn make_creator(pool: &SqlitePool) -> Uuid {
        let repo = SqliteMemberRepository::new(pool.clone());
        let m = repo
            .create(CreateMemberRequest {
                email: format!("a-{}@example.com", Uuid::new_v4()),
                username: format!("u_{}", Uuid::new_v4().simple()),
                full_name: "Test Admin".to_string(),
                password: "p4ssword_long_enough".to_string(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .unwrap();
        m.id
    }

    /// Next Tuesday at 18:00 UTC strictly after `now + 1 day`. The
    /// weekly-by-Tuesday tests need the anchor itself to fall on a
    /// Tuesday so the generator's first candidate matches.
    fn next_tuesday_anchor() -> DateTime<Utc> {
        let now = Utc::now();
        let start = now + Duration::days(1);
        let days_until_tue = (Weekday::Tue.num_days_from_monday() as i64
            - start.weekday().num_days_from_monday() as i64)
            .rem_euclid(7);
        let date = start.date_naive() + Duration::days(days_until_tue);
        date.and_hms_opt(18, 0, 0).unwrap().and_utc()
    }

    fn template(creator: Uuid, start: DateTime<Utc>) -> Event {
        Event {
            id: Uuid::new_v4(),
            title: "Tuesday Coffee".to_string(),
            description: "Weekly hangout".to_string(),
            event_type: EventType::Social,
            event_type_id: None,
            visibility: EventVisibility::MembersOnly,
            start_time: start,
            end_time: Some(start + Duration::hours(2)),
            location: Some("HQ".to_string()),
            max_attendees: Some(20),
            rsvp_required: true,
            image_url: None,
            created_by: creator,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            series_id: None,
            occurrence_index: None,
        }
    }

    async fn count_series(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM event_series")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    // -----------------------------------------------------------------
    // Error-path tests — every explicit Err(...) arm in
    // create_series_with_initial_materialization and
    // compute_occurrence_start_time gets a dedicated case asserting
    // the typed AppError variant and message substring.

    #[tokio::test]
    async fn create_series_errors_on_invalid_recurrence_rule() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let creator = make_creator(&pool).await;
        let anchor = next_tuesday_anchor();

        // Empty weekday set is invalid per Recurrence::validate.
        let rule = Recurrence::WeeklyByDay {
            interval: 1,
            weekdays: vec![],
        };

        let result = svc
            .create_series_with_initial_materialization(
                rule,
                template(creator, anchor),
                None,
                creator,
            )
            .await;
        match result {
            Err(AppError::BadRequest(_)) => {}
            Ok(_) => panic!("expected BadRequest error, got Ok"),
            Err(other) => panic!("expected BadRequest, got {other:?}"),
        }

        // No event_series row written — short-circuit before persist.
        assert_eq!(count_series(&pool).await, 0);
    }

    #[tokio::test]
    async fn create_series_errors_when_until_date_before_start_time() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let creator = make_creator(&pool).await;

        // start_time = now + 7 days, until_date = now + 1 day. The
        // generator's [from, to) window becomes empty (to <= from)
        // and the service maps that to BadRequest.
        let now = Utc::now();
        let start = now + Duration::days(7);
        let until = now + Duration::days(1);
        let rule = Recurrence::WeeklyByDay {
            interval: 1,
            weekdays: vec![WeekdayCode::Tue],
        };

        let result = svc
            .create_series_with_initial_materialization(
                rule,
                template(creator, start),
                Some(until),
                creator,
            )
            .await;
        match result {
            Err(AppError::BadRequest(msg)) => assert!(
                msg.contains("no occurrences"),
                "expected 'no occurrences' in msg: {msg}",
            ),
            Ok(_) => panic!("expected BadRequest error, got Ok"),
            Err(other) => panic!("expected BadRequest, got {other:?}"),
        }

        // No series row and no events written.
        assert_eq!(count_series(&pool).await, 0);
        let event_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(event_count.0, 0);
    }

    #[tokio::test]
    async fn compute_occurrence_start_time_errors_on_zero_index() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let creator = make_creator(&pool).await;
        let anchor = next_tuesday_anchor();
        let rule = Recurrence::WeeklyByDay {
            interval: 1,
            weekdays: vec![WeekdayCode::Tue],
        };
        let created = svc
            .create_series_with_initial_materialization(
                rule,
                template(creator, anchor),
                None,
                creator,
            )
            .await
            .unwrap();

        // index == 0
        let err = svc
            .compute_occurrence_start_time(&created.series, 0)
            .await
            .unwrap_err();
        match err {
            AppError::BadRequest(msg) => assert!(
                msg.contains("occurrence_index must be >= 1"),
                "expected zero-index msg, got: {msg}",
            ),
            other => panic!("expected BadRequest, got {other:?}"),
        }

        // index < 0
        let err = svc
            .compute_occurrence_start_time(&created.series, -3)
            .await
            .unwrap_err();
        match err {
            AppError::BadRequest(msg) => assert!(
                msg.contains("occurrence_index must be >= 1"),
                "expected negative-index msg, got: {msg}",
            ),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn compute_occurrence_start_time_errors_when_series_has_no_events() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let creator = make_creator(&pool).await;
        let anchor = next_tuesday_anchor();
        let rule = Recurrence::WeeklyByDay {
            interval: 1,
            weekdays: vec![WeekdayCode::Tue],
        };
        let created = svc
            .create_series_with_initial_materialization(
                rule,
                template(creator, anchor),
                None,
                creator,
            )
            .await
            .unwrap();

        // Wipe every materialized occurrence — series row stays, anchor
        // inference (MIN(start_time)) now returns NULL.
        sqlx::query("DELETE FROM events WHERE series_id = ?")
            .bind(created.series.id.to_string())
            .execute(&pool)
            .await
            .unwrap();

        let err = svc
            .compute_occurrence_start_time(&created.series, 1)
            .await
            .unwrap_err();
        match err {
            AppError::Internal(msg) => assert!(
                msg.contains("cannot infer series anchor"),
                "expected anchor-inference msg, got: {msg}",
            ),
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn compute_occurrence_start_time_errors_on_index_beyond_horizon() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let creator = make_creator(&pool).await;
        let anchor = next_tuesday_anchor();
        let rule = Recurrence::WeeklyByDay {
            interval: 1,
            weekdays: vec![WeekdayCode::Tue],
        };
        let created = svc
            .create_series_with_initial_materialization(
                rule,
                template(creator, anchor),
                None,
                creator,
            )
            .await
            .unwrap();

        // 20_000 is well past generate_occurrences' internal 10_000-entry
        // cap; times.get(19_999) is None and the service reports a
        // BadRequest rather than panicking.
        let err = svc
            .compute_occurrence_start_time(&created.series, 20_000)
            .await
            .unwrap_err();
        match err {
            AppError::BadRequest(msg) => assert!(
                msg.contains("beyond the series's generated occurrences"),
                "expected beyond-horizon msg, got: {msg}",
            ),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }
}
