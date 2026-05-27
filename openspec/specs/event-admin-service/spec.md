# event-admin-service Specification

## Purpose
TBD - created by archiving change a08-lift-event-admin-orchestration. Update Purpose after archive.
## Requirements
### Requirement: EventAdminService is the single entrypoint for admin-driven event mutations

The system SHALL expose an `EventAdminService` at `src/service/event_admin_service.rs` that owns the full side-effect chain (validation, repo update, audit log, integration dispatch) for every admin-driven event mutation. Admin event handlers SHALL call this service rather than invoking the event repository, audit service, or integration manager directly.

#### Scenario: Handlers call the service, not the repo + collaborators

- **WHEN** an admin POSTs to an event-mutation route (`/portal/admin/events/new`, `/portal/admin/events/:id/update`, `/portal/admin/events/:id/delete`, or any series variant)
- **THEN** the handler SHALL call exactly one `EventAdminService` method to perform the operation; the handler SHALL NOT call `event_repo.{create,update,delete}`, `audit_service.log`, or `integration_manager.handle_event` directly for that flow

#### Scenario: Adding a new admin event action extends the service

- **WHEN** a contributor adds a new admin event mutation
- **THEN** the new method SHALL be added to `EventAdminService` with the audit + integration + (where applicable) integration-event dispatch baked into the method body; handlers calling the method inherit all side-effects

### Requirement: Every mutation method takes an explicit actor_id

`EventAdminService` mutation methods SHALL take `actor_id: Uuid` as a required parameter, passed to `audit_service.log` so audit-row provenance cannot be omitted by the caller.

#### Scenario: Audit row carries actor

- **WHEN** an admin invokes any `EventAdminService` mutation
- **THEN** the resulting `audit_logs` row SHALL have `actor_id = <admin's member uuid>`

### Requirement: EventPublished integration dispatch happens in the service

The service SHALL dispatch `IntegrationEvent::EventPublished` when creating an event whose visibility is not `AdminOnly`. The dispatch SHALL be part of the create method's body; handlers SHALL NOT issue this dispatch separately.

#### Scenario: Members-visible event creation dispatches the event

- **WHEN** an admin creates an event with visibility `Members` or `Public`
- **THEN** `EventAdminService::create` SHALL emit `IntegrationEvent::EventPublished(event)` via `integration_manager.handle_event(...)` after the persist

#### Scenario: Admin-only event creation does not dispatch

- **WHEN** an admin creates an event with visibility `AdminOnly`
- **THEN** the service SHALL NOT dispatch `EventPublished`; the audit row is still written

### Requirement: Service inherits existing failure semantics

`EventAdminService` SHALL preserve the failure semantics established by `PaymentService` and `MemberService`:

- Audit-log insert failure: logged via `tracing`, swallowed.
- Integration dispatch failure: per-integration failures logged inside `IntegrationManager`; the call returns success.
- Repository failure: propagated as `AppError`.

#### Scenario: Integration dispatch failure does not roll back the event

- **WHEN** `EventAdminService::create` succeeds at the repo insert but Discord rejects the `EventPublished` event
- **THEN** the method SHALL return `Ok(event)`; the integration failure SHALL be logged inside the integration layer

### Requirement: EventAdminService supports per-occurrence exceptions

`EventAdminService` SHALL expose three methods for managing per-occurrence exceptions to a recurring series:

- `cancel_event_occurrence(actor_id, series_id, occurrence_index, reason)` — cancels a single occurrence. Inserts an exception row with `kind = 'cancelled'` and DELETEs the corresponding `events` row. Idempotent.
- `override_event_occurrence(actor_id, series_id, occurrence_index, overrides, reason)` — overrides selected fields on a single occurrence. Inserts an exception row with `kind = 'overridden'` and `override_payload` JSON, then UPDATEs the `events` row with the override values. Returns the updated event.
- `restore_event_occurrence(actor_id, series_id, occurrence_index)` — removes the exception. For cancelled: re-creates the event row from the series template. For overridden: resets the event row to the series template. DELETEs the exception row.

Each method SHALL emit an audit row (`cancel_event_occurrence`, `override_event_occurrence`, `restore_event_occurrence`) per the `audit-logging` capability contract.

#### Scenario: Cancel a single occurrence

- **WHEN** an admin calls `cancel_event_occurrence(admin_id, series_id, 5, Some("holiday"))` on a series whose occurrence 5 currently exists
- **THEN** `event_series_exceptions` SHALL contain a row `(series_id, 5, 'cancelled', NULL, …)`; the `events` row for occurrence 5 SHALL be deleted; an audit row with `action = "cancel_event_occurrence"` SHALL be present

#### Scenario: Override a single occurrence's location

- **WHEN** an admin calls `override_event_occurrence(admin_id, series_id, 7, OccurrenceOverride { location: Some("Conference Room B"), .. })` on a series whose occurrence 7 currently exists
- **THEN** `event_series_exceptions` SHALL contain a row `(series_id, 7, 'overridden', '{"location":"Conference Room B"}', …)`; the `events` row for occurrence 7 SHALL have `location = 'Conference Room B'`; other fields unchanged

#### Scenario: Restore a cancelled occurrence

- **WHEN** an admin calls `restore_event_occurrence(admin_id, series_id, 5)` and occurrence 5 was previously cancelled
- **THEN** the exception row SHALL be deleted; a new `events` row for occurrence 5 SHALL be created from the series template (start_time + template fields recomputed via the recurrence rule)

#### Scenario: Idempotent cancel

- **WHEN** an admin calls `cancel_event_occurrence` twice on the same `(series_id, occurrence_index)` pair
- **THEN** the second call SHALL succeed without error; the exception row remains; the events row remains absent; a second audit row IS emitted

### Requirement: Materializer respects per-occurrence exceptions

The recurring-event materializer (both initial materialization on series creation and the daily horizon-roll) SHALL consult `event_series_exceptions` for each `(series_id, occurrence_index)` pair it would otherwise create:

- If a `cancelled` exception exists → no `events` row is created for that index.
- If an `overridden` exception exists → the `events` row is created from the series template, then the `override_payload`'s non-null fields are applied on top.
- If no exception exists → the `events` row is created from the series template as before.

This guarantees that:
- A cancelled occurrence does NOT reappear on the next horizon-roll.
- An overridden occurrence's overrides do NOT get clobbered when materialization re-runs.

#### Scenario: Cancelled occurrence stays cancelled across horizon-roll

- **WHEN** an occurrence is cancelled via `cancel_event_occurrence`, then the daily materializer runs (`now + 52 weeks` extends the horizon past the cancelled occurrence's date)
- **THEN** the materializer SHALL NOT recreate an `events` row for that occurrence index; the cancellation persists

#### Scenario: Overridden occurrence overrides survive series re-edit

- **WHEN** occurrence 7 has an `overridden` exception (location = "Room B"), then `update_series` is called with a cutoff before occurrence 7 (forcing re-materialization)
- **THEN** the `events` row for occurrence 7 SHALL be re-created with the series's updated template fields AND the override's location = "Room B" applied on top

### Requirement: OccurrenceOverride permits a documented subset of fields

The `OccurrenceOverride` struct SHALL permit overriding the following fields on a per-occurrence basis: `title`, `description`, `start_time`, `end_time`, `location`, `max_attendees`, `rsvp_required`, `image_url`. `null` for a field means "use the series template value."

`event_type` and `visibility` are series-level concerns and SHALL NOT be overridable per-occurrence in this iteration.

#### Scenario: Override with only location set leaves other fields from series

- **WHEN** `override_event_occurrence` is called with `OccurrenceOverride { location: Some("Room B"), .. defaults }` (all other fields `None`)
- **THEN** the resulting `events` row's `location` is "Room B"; `title`, `start_time`, etc. match the series template

