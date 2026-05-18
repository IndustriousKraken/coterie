//! Service that owns the full side-effect chain for admin-driven
//! event mutations: repo update → audit log → integration dispatch.
//! Handlers parse the wire shape and render the response; the service
//! owns everything between.
//!
//! Mirrors `MemberService`'s shape — a per-domain service that
//! co-locates validation, persistence, and the post-work chain so a
//! contributor adding a new admin action can't accidentally forget
//! one piece (audit, integration event). See the
//! `event-admin-service` capability spec for the contract.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    domain::{Event, EventType, EventVisibility, Recurrence},
    error::{AppError, Result},
    integrations::{IntegrationEvent, IntegrationManager},
    repository::{EventRepository, EventSeriesRepository},
    service::{audit_service::AuditService, recurring_event_service::RecurringEventService},
};

/// Typed input for creating an event. The handler parses the
/// multipart form into one of these and hands it off. When
/// `recurrence` is `Some`, the service materializes a full series;
/// otherwise it persists a single one-off event.
pub struct CreateEventInput {
    pub title: String,
    pub description: String,
    pub event_type: EventType,
    pub event_type_id: Option<Uuid>,
    pub visibility: EventVisibility,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub max_attendees: Option<i32>,
    pub rsvp_required: bool,
    pub image_url: Option<String>,
    /// Some → materialize a full recurring series via
    /// `RecurringEventService`. None → single-row insert.
    pub recurrence: Option<Recurrence>,
    /// Optional cutoff for series materialization. Ignored when
    /// `recurrence` is None.
    pub recurrence_until: Option<DateTime<Utc>>,
}

/// Typed input for updating an event. Carries the editable subset of
/// `Event` fields; immutable identity fields (id, created_by,
/// created_at, series_id, occurrence_index) are not part of this.
#[derive(Clone)]
pub struct UpdateEventInput {
    pub title: String,
    pub description: String,
    pub event_type: EventType,
    pub event_type_id: Option<Uuid>,
    pub visibility: EventVisibility,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub max_attendees: Option<i32>,
    pub rsvp_required: bool,
    pub image_url: Option<String>,
}

pub struct EventAdminService {
    event_repo: Arc<dyn EventRepository>,
    event_series_repo: Arc<dyn EventSeriesRepository>,
    recurring_event_service: Arc<RecurringEventService>,
    audit_service: Arc<AuditService>,
    integration_manager: Arc<IntegrationManager>,
}

impl EventAdminService {
    pub fn new(
        event_repo: Arc<dyn EventRepository>,
        event_series_repo: Arc<dyn EventSeriesRepository>,
        recurring_event_service: Arc<RecurringEventService>,
        audit_service: Arc<AuditService>,
        integration_manager: Arc<IntegrationManager>,
    ) -> Self {
        Self {
            event_repo,
            event_series_repo,
            recurring_event_service,
            audit_service,
            integration_manager,
        }
    }

    /// Create an event. When `input.recurrence` is `Some`, materializes
    /// a recurring series and returns the anchor (first) occurrence;
    /// otherwise inserts a single event. In either case audits the
    /// action and — when visibility is not `AdminOnly` — dispatches
    /// `IntegrationEvent::EventPublished` for the resulting event.
    pub async fn create(
        &self,
        actor_id: Uuid,
        input: CreateEventInput,
    ) -> Result<Event> {
        let template = Event {
            id: Uuid::new_v4(),
            title: input.title,
            description: input.description,
            event_type: input.event_type,
            event_type_id: input.event_type_id,
            visibility: input.visibility,
            start_time: input.start_time,
            end_time: input.end_time,
            location: input.location,
            max_attendees: input.max_attendees,
            rsvp_required: input.rsvp_required,
            image_url: input.image_url,
            created_by: actor_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            series_id: None,
            occurrence_index: None,
        };
        let visibility_for_dispatch = template.visibility.clone();

        let event = if let Some(rule) = input.recurrence {
            // Series creation: materialize via RecurringEventService,
            // audit the series, return the anchor occurrence.
            let created = self.recurring_event_service
                .create_series_with_initial_materialization(
                    rule, template, input.recurrence_until, actor_id,
                )
                .await?;
            let first = created.occurrences.first().cloned().ok_or_else(|| {
                AppError::Internal("series materialized zero occurrences".to_string())
            })?;
            self.audit_service.log(
                Some(actor_id),
                "create_event_series",
                "event_series",
                &created.series.id.to_string(),
                None,
                Some(&first.title),
                None,
            ).await;
            first
        } else {
            // Single event.
            let created = self.event_repo.create(template).await?;
            self.audit_service.log(
                Some(actor_id),
                "create_event",
                "event",
                &created.id.to_string(),
                None,
                Some(&created.title),
                None,
            ).await;
            created
        };

        // Dispatch EventPublished unless AdminOnly. For a series we
        // emit one event for the anchor occurrence — Discord treats
        // each series as one announcement, not 52.
        if visibility_for_dispatch != EventVisibility::AdminOnly {
            self.integration_manager
                .handle_event(IntegrationEvent::EventPublished(event.clone()))
                .await;
        }

        Ok(event)
    }

    /// Update a single event row. Audits `update_event`. No
    /// integration dispatch — updates are silent per existing design.
    pub async fn update_one(
        &self,
        actor_id: Uuid,
        event_id: Uuid,
        input: UpdateEventInput,
    ) -> Result<Event> {
        let existing = self.event_repo.find_by_id(event_id).await?
            .ok_or_else(|| AppError::NotFound("Event not found".to_string()))?;

        let updated = Event {
            id: event_id,
            title: input.title,
            description: input.description,
            event_type: input.event_type,
            event_type_id: input.event_type_id,
            visibility: input.visibility,
            start_time: input.start_time,
            end_time: input.end_time,
            location: input.location,
            max_attendees: input.max_attendees,
            rsvp_required: input.rsvp_required,
            image_url: input.image_url,
            created_by: existing.created_by,
            created_at: existing.created_at,
            updated_at: Utc::now(),
            series_id: existing.series_id,
            occurrence_index: existing.occurrence_index,
        };

        let result = self.event_repo.update(event_id, updated).await?;

        self.audit_service.log(
            Some(actor_id),
            "update_event",
            "event",
            &event_id.to_string(),
            None,
            Some(&result.title),
            None,
        ).await;

        Ok(result)
    }

    /// Apply the editable subset of `input` to every occurrence in
    /// `series_id` whose `start_time >= from`. Returns the count of
    /// affected rows. Audits `update_event_series`.
    pub async fn update_series_from(
        &self,
        actor_id: Uuid,
        series_id: Uuid,
        from: DateTime<Utc>,
        input: UpdateEventInput,
    ) -> Result<u64> {
        // The repo helper reads from the template Event but only
        // applies the editable subset — id/created_*/series_id are
        // ignored. We still need a placeholder Event to pass through.
        let template = Event {
            id: Uuid::new_v4(),
            title: input.title,
            description: input.description,
            event_type: input.event_type,
            event_type_id: input.event_type_id,
            visibility: input.visibility,
            start_time: from,
            end_time: input.end_time,
            location: input.location,
            max_attendees: input.max_attendees,
            rsvp_required: input.rsvp_required,
            image_url: input.image_url,
            created_by: actor_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            series_id: Some(series_id),
            occurrence_index: None,
        };

        let count = self.event_repo
            .update_series_occurrences_from(series_id, from, &template)
            .await?;

        self.audit_service.log(
            Some(actor_id),
            "update_event_series",
            "event_series",
            &series_id.to_string(),
            None,
            Some(&count.to_string()),
            None,
        ).await;

        Ok(count)
    }

    /// Delete a single event row. Audits `delete_event`.
    pub async fn delete_one(&self, actor_id: Uuid, event_id: Uuid) -> Result<()> {
        self.event_repo.delete(event_id).await?;
        self.audit_service.log(
            Some(actor_id),
            "delete_event",
            "event",
            &event_id.to_string(),
            None,
            None,
            None,
        ).await;
        Ok(())
    }

    /// End a series after `after`: hard-delete every later occurrence
    /// and cap the series' `until_date` so the horizon job doesn't
    /// re-materialize. Audits `end_series` with the deleted count.
    /// Set-until-date failure is logged but does not fail the call —
    /// the primary delete already succeeded.
    pub async fn end_series(
        &self,
        actor_id: Uuid,
        series_id: Uuid,
        after: DateTime<Utc>,
    ) -> Result<u64> {
        let count = self.event_repo
            .delete_series_occurrences_after(series_id, after).await?;
        if let Err(e) = self.event_series_repo.set_until_date(series_id, after).await {
            tracing::error!("set_until_date failed for series {}: {}", series_id, e);
        }
        self.audit_service.log(
            Some(actor_id),
            "end_series",
            "event_series",
            &series_id.to_string(),
            None,
            Some(&count.to_string()),
            None,
        ).await;
        Ok(count)
    }

    /// Cascade-delete a series: drops the series row and (via FK
    /// ON DELETE CASCADE) every occurrence. Audits
    /// `delete_event_series`.
    pub async fn delete_series(&self, actor_id: Uuid, series_id: Uuid) -> Result<()> {
        self.event_series_repo.delete(series_id).await?;
        self.audit_service.log(
            Some(actor_id),
            "delete_event_series",
            "event_series",
            &series_id.to_string(),
            None,
            None,
            None,
        ).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{EventType, EventVisibility, Recurrence, WeekdayCode, CreateMemberRequest},
        integrations::IntegrationManager,
        repository::{
            MemberRepository, SqliteEventRepository, SqliteEventSeriesRepository,
            SqliteMemberRepository,
        },
    };
    use chrono::{Datelike, Duration, Weekday};
    use sqlx::{Executor, SqlitePool};

    /// Next Saturday at 18:00 UTC strictly after `now + 1 day`. Used as
    /// the start time for single-event tests that don't care about the
    /// weekday — just need a valid future timestamp.
    fn next_saturday_anchor() -> DateTime<Utc> {
        let now = Utc::now();
        let start = now + Duration::days(1);
        let days_until_sat = (Weekday::Sat.num_days_from_monday() as i64
            - start.weekday().num_days_from_monday() as i64).rem_euclid(7);
        let date = start.date_naive() + Duration::days(days_until_sat);
        date.and_hms_opt(18, 0, 0).unwrap().and_utc()
    }

    /// Next Tuesday at 18:00 UTC strictly after `now + 1 day`. Used as
    /// the start time for recurring-Tuesday tests where the weekly rule
    /// requires the anchor BE a Tuesday.
    fn next_tuesday_anchor() -> DateTime<Utc> {
        let now = Utc::now();
        let start = now + Duration::days(1);
        let days_until_tue = (Weekday::Tue.num_days_from_monday() as i64
            - start.weekday().num_days_from_monday() as i64).rem_euclid(7);
        let date = start.date_naive() + Duration::days(days_until_tue);
        date.and_hms_opt(18, 0, 0).unwrap().and_utc()
    }

    /// `anchor` shifted forward by `weeks` whole weeks. Use to compute
    /// `until_date` values relative to the test's local anchor.
    fn weeks_after(anchor: DateTime<Utc>, weeks: i64) -> DateTime<Utc> {
        anchor + Duration::weeks(weeks)
    }

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
        sqlx::migrate!("./migrations").run(&pool).await.expect("migrate");
        pool
    }

    fn make_service(pool: SqlitePool) -> EventAdminService {
        let event_repo: Arc<dyn EventRepository> =
            Arc::new(SqliteEventRepository::new(pool.clone()));
        let series_repo: Arc<dyn EventSeriesRepository> =
            Arc::new(SqliteEventSeriesRepository::new(pool.clone()));
        let recurring = Arc::new(RecurringEventService::new(
            event_repo.clone(), series_repo.clone(), pool.clone(),
        ));
        let audit = Arc::new(AuditService::new(pool.clone()));
        let integrations = Arc::new(IntegrationManager::new());

        EventAdminService::new(
            event_repo,
            series_repo,
            recurring,
            audit,
            integrations,
        )
    }

    async fn make_actor(pool: &SqlitePool) -> Uuid {
        let repo = SqliteMemberRepository::new(pool.clone());
        let m = repo.create(CreateMemberRequest {
            email: format!("a-{}@example.com", Uuid::new_v4()),
            username: format!("u_{}", Uuid::new_v4().simple()),
            full_name: "Test Admin".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
        }).await.unwrap();
        m.id
    }

    async fn audit_count(pool: &SqlitePool, action: &str, entity_id: &str) -> i64 {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM audit_logs WHERE action = ? AND entity_id = ?"
        )
        .bind(action)
        .bind(entity_id)
        .fetch_one(pool).await.unwrap();
        count.0
    }

    fn single_input(start: DateTime<Utc>, visibility: EventVisibility) -> CreateEventInput {
        CreateEventInput {
            title: "Test Event".to_string(),
            description: "A test event".to_string(),
            event_type: EventType::Meeting,
            event_type_id: None,
            visibility,
            start_time: start,
            end_time: None,
            location: None,
            max_attendees: None,
            rsvp_required: false,
            image_url: None,
            recurrence: None,
            recurrence_until: None,
        }
    }

    #[tokio::test]
    async fn create_single_event_emits_full_chain() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let start = next_saturday_anchor();
        let input = single_input(start, EventVisibility::MembersOnly);

        let event = svc.create(actor, input).await.unwrap();

        // Repo touched — event exists and matches input.
        let fetched = svc.event_repo.find_by_id(event.id).await.unwrap().unwrap();
        assert_eq!(fetched.title, "Test Event");
        assert_eq!(fetched.visibility, EventVisibility::MembersOnly);
        assert!(fetched.series_id.is_none(), "non-recurring create should not set series_id");

        // Audit row inserted.
        assert_eq!(audit_count(&pool, "create_event", &event.id.to_string()).await, 1);

        // No series row created (single insert).
        let series_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM event_series")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(series_count.0, 0);

        // Integration dispatch fired — IntegrationManager has no
        // registered integrations in this test so the call is a no-op
        // but reaching here without panic confirms the chain ran.
    }

    #[tokio::test]
    async fn create_recurring_series_materializes_and_audits() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let start = next_tuesday_anchor();
        let until = weeks_after(start, 8);
        let mut input = single_input(start, EventVisibility::MembersOnly);
        input.recurrence = Some(Recurrence::WeeklyByDay {
            interval: 1,
            weekdays: vec![WeekdayCode::Tue],
        });
        input.recurrence_until = Some(until);

        let anchor = svc.create(actor, input).await.unwrap();

        // Anchor is an actual occurrence row.
        assert!(anchor.series_id.is_some());
        let series_id = anchor.series_id.unwrap();

        // Multiple occurrences materialized.
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events WHERE series_id = ?")
            .bind(series_id.to_string())
            .fetch_one(&pool).await.unwrap();
        assert!(count.0 > 1, "expected multiple occurrences, got {}", count.0);

        // Series row exists.
        let series_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM event_series WHERE id = ?")
            .bind(series_id.to_string())
            .fetch_one(&pool).await.unwrap();
        assert_eq!(series_count.0, 1);

        // Audit row uses create_event_series with series_id as the entity_id.
        assert_eq!(
            audit_count(&pool, "create_event_series", &series_id.to_string()).await,
            1,
        );
        // And NOT a per-occurrence create_event audit row.
        assert_eq!(audit_count(&pool, "create_event", &anchor.id.to_string()).await, 0);
    }

    #[tokio::test]
    async fn create_admin_only_event_skips_integration_dispatch() {
        // No external observability of the integration_manager call
        // beyond "did the method return Ok"; this test ensures the
        // AdminOnly branch persists the event + audit row without
        // panicking and asserts the visibility was preserved end-to-end.
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let start = next_saturday_anchor();
        let input = single_input(start, EventVisibility::AdminOnly);

        let event = svc.create(actor, input).await.unwrap();
        assert_eq!(event.visibility, EventVisibility::AdminOnly);
        assert_eq!(audit_count(&pool, "create_event", &event.id.to_string()).await, 1);
    }

    fn update_input_from(event: &Event) -> UpdateEventInput {
        UpdateEventInput {
            title: event.title.clone(),
            description: event.description.clone(),
            event_type: event.event_type.clone(),
            event_type_id: event.event_type_id,
            visibility: event.visibility.clone(),
            start_time: event.start_time,
            end_time: event.end_time,
            location: event.location.clone(),
            max_attendees: event.max_attendees,
            rsvp_required: event.rsvp_required,
            image_url: event.image_url.clone(),
        }
    }

    #[tokio::test]
    async fn update_one_writes_audit() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let start = next_saturday_anchor();
        let event = svc.create(
            actor, single_input(start, EventVisibility::MembersOnly),
        ).await.unwrap();

        let mut input = update_input_from(&event);
        input.title = "Renamed".to_string();
        let result = svc.update_one(actor, event.id, input).await.unwrap();

        assert_eq!(result.title, "Renamed");
        assert_eq!(audit_count(&pool, "update_event", &event.id.to_string()).await, 1);
    }

    #[tokio::test]
    async fn update_series_from_audits_with_count() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let start = next_tuesday_anchor();
        let until = weeks_after(start, 8);
        let mut input = single_input(start, EventVisibility::MembersOnly);
        input.recurrence = Some(Recurrence::WeeklyByDay {
            interval: 1,
            weekdays: vec![WeekdayCode::Tue],
        });
        input.recurrence_until = Some(until);

        let anchor = svc.create(actor, input).await.unwrap();
        let series_id = anchor.series_id.unwrap();

        let mut update = update_input_from(&anchor);
        update.title = "Renamed Series".to_string();
        let count = svc.update_series_from(
            actor, series_id, anchor.start_time, update,
        ).await.unwrap();
        assert!(count >= 1);

        assert_eq!(
            audit_count(&pool, "update_event_series", &series_id.to_string()).await,
            1,
        );

        // Anchor row reflects the new title.
        let fetched = svc.event_repo.find_by_id(anchor.id).await.unwrap().unwrap();
        assert_eq!(fetched.title, "Renamed Series");
    }

    #[tokio::test]
    async fn delete_one_writes_audit() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let start = next_saturday_anchor();
        let event = svc.create(
            actor, single_input(start, EventVisibility::MembersOnly),
        ).await.unwrap();

        svc.delete_one(actor, event.id).await.unwrap();
        assert!(svc.event_repo.find_by_id(event.id).await.unwrap().is_none());
        assert_eq!(audit_count(&pool, "delete_event", &event.id.to_string()).await, 1);
    }

    #[tokio::test]
    async fn end_series_caps_and_audits() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let start = next_tuesday_anchor();
        let until = weeks_after(start, 8);
        let mut input = single_input(start, EventVisibility::MembersOnly);
        input.recurrence = Some(Recurrence::WeeklyByDay {
            interval: 1,
            weekdays: vec![WeekdayCode::Tue],
        });
        input.recurrence_until = Some(until);

        let anchor = svc.create(actor, input).await.unwrap();
        let series_id = anchor.series_id.unwrap();

        let before: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events WHERE series_id = ?"
        )
        .bind(series_id.to_string())
        .fetch_one(&pool).await.unwrap();

        // End the series at anchor.start_time — should delete all
        // strictly-later occurrences and leave anchor.
        let removed = svc.end_series(actor, series_id, anchor.start_time).await.unwrap();
        assert!(removed > 0);

        let after: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events WHERE series_id = ?"
        )
        .bind(series_id.to_string())
        .fetch_one(&pool).await.unwrap();
        assert_eq!(after.0, before.0 - removed as i64);

        // Anchor still there.
        assert!(svc.event_repo.find_by_id(anchor.id).await.unwrap().is_some());

        // until_date set on the series row.
        let series = svc.event_series_repo.find_by_id(series_id).await.unwrap().unwrap();
        assert_eq!(series.until_date, Some(anchor.start_time));

        assert_eq!(
            audit_count(&pool, "end_series", &series_id.to_string()).await,
            1,
        );
    }

    #[tokio::test]
    async fn delete_series_cascades_and_audits() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let start = next_tuesday_anchor();
        let until = weeks_after(start, 8);
        let mut input = single_input(start, EventVisibility::MembersOnly);
        input.recurrence = Some(Recurrence::WeeklyByDay {
            interval: 1,
            weekdays: vec![WeekdayCode::Tue],
        });
        input.recurrence_until = Some(until);

        let anchor = svc.create(actor, input).await.unwrap();
        let series_id = anchor.series_id.unwrap();

        svc.delete_series(actor, series_id).await.unwrap();

        // Series row gone.
        assert!(svc.event_series_repo.find_by_id(series_id).await.unwrap().is_none());
        // All occurrences cascade-deleted.
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events WHERE series_id = ?"
        )
        .bind(series_id.to_string())
        .fetch_one(&pool).await.unwrap();
        assert_eq!(count.0, 0);

        assert_eq!(
            audit_count(&pool, "delete_event_series", &series_id.to_string()).await,
            1,
        );
    }
}
