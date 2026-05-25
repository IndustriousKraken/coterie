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

