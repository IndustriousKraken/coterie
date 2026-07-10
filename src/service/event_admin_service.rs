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
    domain::{
        Event, EventType, EventVisibility, OccurrenceException, OccurrenceExceptionKind,
        OccurrenceOverride, Recurrence,
    },
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
    /// IANA zone the wall-clock `start_time`/`end_time` are in. Defaulted
    /// from `org.timezone` by the handler and frozen on the row.
    pub timezone: String,
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
    pub async fn create(&self, actor_id: Uuid, input: CreateEventInput) -> Result<Event> {
        let template = Event {
            id: Uuid::new_v4(),
            title: input.title,
            description: input.description,
            event_type: input.event_type,
            event_type_id: input.event_type_id,
            visibility: input.visibility,
            start_time: input.start_time,
            end_time: input.end_time,
            timezone: input.timezone,
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
            let created = self
                .recurring_event_service
                .create_series_with_initial_materialization(
                    rule,
                    template,
                    input.recurrence_until,
                    actor_id,
                )
                .await?;
            let first = created.occurrences.first().cloned().ok_or_else(|| {
                AppError::Internal("series materialized zero occurrences".to_string())
            })?;
            self.audit_service
                .log(
                    Some(actor_id),
                    "create_event_series",
                    "event_series",
                    &created.series.id.to_string(),
                    None,
                    Some(&first.title),
                    None,
                )
                .await;
            first
        } else {
            // Single event.
            let created = self.event_repo.create(template).await?;
            self.audit_service
                .log(
                    Some(actor_id),
                    "create_event",
                    "event",
                    &created.id.to_string(),
                    None,
                    Some(&created.title),
                    None,
                )
                .await;
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
        let existing = self
            .event_repo
            .find_by_id(event_id)
            .await?
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
            // Zone is frozen at creation; an edit never re-zones the event.
            timezone: existing.timezone,
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

        self.audit_service
            .log(
                Some(actor_id),
                "update_event",
                "event",
                &event_id.to_string(),
                None,
                Some(&result.title),
                None,
            )
            .await;

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
            // Placeholder: update_series_occurrences_from does not write
            // the zone (it's frozen per occurrence), so this is unused.
            timezone: String::new(),
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

        let count = self
            .event_repo
            .update_series_occurrences_from(series_id, from, &template)
            .await?;

        self.audit_service
            .log(
                Some(actor_id),
                "update_event_series",
                "event_series",
                &series_id.to_string(),
                None,
                Some(&count.to_string()),
                None,
            )
            .await;

        Ok(count)
    }

    /// Delete a single event row. Audits `delete_event`.
    pub async fn delete_one(&self, actor_id: Uuid, event_id: Uuid) -> Result<()> {
        self.event_repo.delete(event_id).await?;
        self.audit_service
            .log(
                Some(actor_id),
                "delete_event",
                "event",
                &event_id.to_string(),
                None,
                None,
                None,
            )
            .await;
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
        let count = self
            .event_repo
            .delete_series_occurrences_after(series_id, after)
            .await?;
        if let Err(e) = self
            .event_series_repo
            .set_until_date(series_id, after)
            .await
        {
            tracing::error!("set_until_date failed for series {}: {}", series_id, e);
        }
        self.audit_service
            .log(
                Some(actor_id),
                "end_series",
                "event_series",
                &series_id.to_string(),
                None,
                Some(&count.to_string()),
                None,
            )
            .await;
        Ok(count)
    }

    /// Cascade-delete a series: drops the series row and (via FK
    /// ON DELETE CASCADE) every occurrence. Audits
    /// `delete_event_series`.
    pub async fn delete_series(&self, actor_id: Uuid, series_id: Uuid) -> Result<()> {
        self.event_series_repo.delete(series_id).await?;
        self.audit_service
            .log(
                Some(actor_id),
                "delete_event_series",
                "event_series",
                &series_id.to_string(),
                None,
                None,
                None,
            )
            .await;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Per-occurrence exceptions
    //
    // These three methods own the "cancel / override / restore a single
    // occurrence of a recurring series" flow. Each writes an exception
    // row that the materializer consults on horizon-rolls, then mutates
    // the corresponding `events` row directly.

    /// Cancel a single occurrence in a series. Records the exception
    /// row (so the materializer never re-creates the occurrence) and
    /// hard-deletes the existing `events` row if present. Idempotent —
    /// calling on an already-cancelled `(series, index)` succeeds and
    /// emits a fresh audit row.
    pub async fn cancel_event_occurrence(
        &self,
        actor_id: Uuid,
        series_id: Uuid,
        occurrence_index: i32,
        reason: Option<String>,
    ) -> Result<()> {
        self.require_series_exists(series_id).await?;
        if occurrence_index < 1 {
            return Err(AppError::BadRequest(
                "occurrence_index must be >= 1".to_string(),
            ));
        }

        let existing = self
            .event_repo
            .find_by_series_and_index(series_id, occurrence_index)
            .await?;
        let (entity_id, old_value) = match existing.as_ref() {
            Some(e) => (e.id.to_string(), Some(e.title.clone())),
            None => (
                format!("{}#{}", series_id, occurrence_index),
                Some(occurrence_index.to_string()),
            ),
        };

        self.event_series_repo
            .insert_exception(OccurrenceException {
                series_id,
                occurrence_index,
                kind: OccurrenceExceptionKind::Cancelled,
                override_payload: None,
                created_at: Utc::now(),
                created_by: actor_id,
                audit_reason: reason,
            })
            .await?;

        if let Some(event) = existing {
            self.event_repo.delete(event.id).await?;
        }

        self.audit_service
            .log(
                Some(actor_id),
                "cancel_event_occurrence",
                "event",
                &entity_id,
                old_value.as_deref(),
                None,
                None,
            )
            .await;
        Ok(())
    }

    /// Override selected fields on a single occurrence. Records the
    /// exception row (so the materializer re-applies the overrides on
    /// future horizon-rolls) and updates the corresponding `events`
    /// row in place. Returns the updated event.
    pub async fn override_event_occurrence(
        &self,
        actor_id: Uuid,
        series_id: Uuid,
        occurrence_index: i32,
        overrides: OccurrenceOverride,
        reason: Option<String>,
    ) -> Result<Event> {
        self.require_series_exists(series_id).await?;
        if occurrence_index < 1 {
            return Err(AppError::BadRequest(
                "occurrence_index must be >= 1".to_string(),
            ));
        }

        let payload = serde_json::to_string(&overrides)
            .map_err(|e| AppError::Internal(format!("override serialize: {}", e)))?;

        self.event_series_repo
            .insert_exception(OccurrenceException {
                series_id,
                occurrence_index,
                kind: OccurrenceExceptionKind::Overridden,
                override_payload: Some(payload),
                created_at: Utc::now(),
                created_by: actor_id,
                audit_reason: reason,
            })
            .await?;

        let mut event = self
            .event_repo
            .find_by_series_and_index(series_id, occurrence_index)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "occurrence {} of series {} not found",
                    occurrence_index, series_id,
                ))
            })?;
        let event_id = event.id;
        overrides.apply(&mut event);
        event.updated_at = Utc::now();
        let updated = self.event_repo.update(event_id, event).await?;

        self.audit_service
            .log(
                Some(actor_id),
                "override_event_occurrence",
                "event",
                &event_id.to_string(),
                None,
                Some(&updated.title),
                None,
            )
            .await;
        Ok(updated)
    }

    /// Reverse an exception. For `Cancelled` the materializer re-creates
    /// the row from the series template (returns `Some(event)`). For
    /// `Overridden` the existing row is reset to the template (returns
    /// `None` — the event_id is unchanged). No-op + audit when no
    /// exception exists.
    pub async fn restore_event_occurrence(
        &self,
        actor_id: Uuid,
        series_id: Uuid,
        occurrence_index: i32,
    ) -> Result<Option<Event>> {
        let series = self
            .event_series_repo
            .find_by_id(series_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("series {} not found", series_id)))?;
        if occurrence_index < 1 {
            return Err(AppError::BadRequest(
                "occurrence_index must be >= 1".to_string(),
            ));
        }

        let exception = self
            .event_series_repo
            .find_exception(series_id, occurrence_index)
            .await?;

        let Some(exception) = exception else {
            // Audit the no-op so operator actions remain traceable.
            self.audit_service
                .log(
                    Some(actor_id),
                    "restore_event_occurrence",
                    "event",
                    &format!("{}#{}", series_id, occurrence_index),
                    None,
                    None,
                    None,
                )
                .await;
            return Ok(None);
        };

        let result = match exception.kind {
            OccurrenceExceptionKind::Cancelled => {
                // Drop the exception FIRST so the materializer doesn't
                // re-skip the index on the single-occurrence path.
                self.event_series_repo
                    .delete_exception(series_id, occurrence_index)
                    .await?;
                let event = self
                    .recurring_event_service
                    .materialize_single_occurrence(&series, occurrence_index)
                    .await?;
                Some(event)
            }
            OccurrenceExceptionKind::Overridden => {
                // Reset the events row by recomputing the would-be
                // template values + start_time from the series rule.
                self.event_series_repo
                    .delete_exception(series_id, occurrence_index)
                    .await?;
                self.reset_overridden_occurrence(&series, occurrence_index)
                    .await?;
                None
            }
        };

        self.audit_service
            .log(
                Some(actor_id),
                "restore_event_occurrence",
                "event",
                &format!("{}#{}", series_id, occurrence_index),
                None,
                None,
                None,
            )
            .await;
        Ok(result)
    }

    async fn require_series_exists(&self, series_id: Uuid) -> Result<()> {
        if self
            .event_series_repo
            .find_by_id(series_id)
            .await?
            .is_none()
        {
            return Err(AppError::NotFound(format!(
                "series {} not found",
                series_id
            )));
        }
        Ok(())
    }

    /// Reset an overridden occurrence's `events` row to match the
    /// series template (start_time + fields). The row's identity
    /// (event_id) is preserved so attendance and integration handles
    /// remain valid.
    async fn reset_overridden_occurrence(
        &self,
        series: &crate::domain::EventSeries,
        occurrence_index: i32,
    ) -> Result<()> {
        let existing = self
            .event_repo
            .find_by_series_and_index(series.id, occurrence_index)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "overridden occurrence {} of series {} has no events row",
                    occurrence_index, series.id,
                ))
            })?;

        let template_start = self
            .recurring_event_service
            .compute_occurrence_start_time(series, occurrence_index)
            .await?;

        // Use any other existing occurrence as the source of template
        // fields. Picking the earliest occurrence (other than this one,
        // if possible) gives a stable reference even after multiple
        // overrides.
        let mut prototype_index = 1_i32;
        if prototype_index == occurrence_index {
            prototype_index = 2;
        }
        let prototype = self
            .event_repo
            .find_by_series_and_index(series.id, prototype_index)
            .await?;
        let prototype = match prototype {
            Some(p) => p,
            None => existing.clone(), // fall back to current row's template-ish fields
        };

        let duration = prototype.end_time.map(|e| e - prototype.start_time);

        let reset = Event {
            id: existing.id,
            title: prototype.title.clone(),
            description: prototype.description.clone(),
            event_type: prototype.event_type.clone(),
            event_type_id: prototype.event_type_id,
            visibility: prototype.visibility.clone(),
            start_time: template_start,
            end_time: duration.map(|d| template_start + d),
            timezone: existing.timezone.clone(),
            location: prototype.location.clone(),
            max_attendees: prototype.max_attendees,
            rsvp_required: prototype.rsvp_required,
            image_url: prototype.image_url.clone(),
            created_by: existing.created_by,
            created_at: existing.created_at,
            updated_at: Utc::now(),
            series_id: existing.series_id,
            occurrence_index: existing.occurrence_index,
        };
        self.event_repo.update(existing.id, reset).await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "event_admin_service_tests.rs"]
mod tests;
