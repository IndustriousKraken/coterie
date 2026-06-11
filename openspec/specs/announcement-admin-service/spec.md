# announcement-admin-service Specification

## Purpose
TBD - created by archiving change a09-lift-announcement-admin-orchestration. Update Purpose after archive.
## Requirements
### Requirement: AnnouncementAdminService is the single entrypoint for admin-driven announcement mutations

The system SHALL expose an `AnnouncementAdminService` at `src/service/announcement_admin_service.rs` owning the full side-effect chain (validation, repo update, audit log, integration dispatch) for every admin-driven announcement mutation. Admin announcement handlers SHALL call this service rather than invoking the announcement repository, audit service, or integration manager directly.

#### Scenario: Handlers call the service, not the repo + collaborators

- **WHEN** an admin POSTs to an announcement-mutation route
- **THEN** the handler SHALL call exactly one `AnnouncementAdminService` method; it SHALL NOT call `announcement_repo`, `audit_service.log`, or `integration_manager.handle_event` directly

### Requirement: Publish path centralizes the AnnouncementPublished dispatch

`AnnouncementAdminService::publish` and the publish-now variant of `AnnouncementAdminService::create` SHALL each dispatch `IntegrationEvent::AnnouncementPublished(announcement)` after the persist. The unpublish path SHALL NOT dispatch any integration event.

#### Scenario: create with publish_now dispatches the integration event

- **WHEN** an admin creates an announcement with `publish_now=true` on the form
- **THEN** the service SHALL mark the row Published, write the audit row, AND dispatch `AnnouncementPublished`

#### Scenario: explicit publish dispatches the integration event

- **WHEN** an admin transitions a Draft announcement to Published via the publish action
- **THEN** the service SHALL update status, write the audit row, AND dispatch `AnnouncementPublished`

#### Scenario: unpublish is silent on the integration channel

- **WHEN** an admin unpublishes a Published announcement
- **THEN** the service SHALL update status and write the audit row but SHALL NOT dispatch any integration event

### Requirement: Every mutation method takes an explicit actor_id

The mutation methods SHALL take `actor_id: Uuid` as a required parameter for audit-row provenance.

#### Scenario: Audit row carries actor

- **WHEN** any service mutation method runs
- **THEN** the resulting `audit_logs` row SHALL have `actor_id = <admin's member uuid>`

### Requirement: AnnouncementAdminService returns typed NotFound for unknown ids

`AnnouncementAdminService::update`, `::delete`, `::publish`, and `::unpublish` SHALL each return `AppError::NotFound("Announcement not found")` (NOT a panic, NOT `AppError::Internal`) when invoked against an `announcement_id` that does not exist in the `announcements` table. The methods SHALL short-circuit BEFORE writing an audit row, so a 404 on the wire MUST NOT produce a phantom audit entry for the non-existent id.

#### Scenario: update against missing id returns 4xx and writes no audit

- **WHEN** a handler invokes `svc.update(actor, missing_id, input)` against an id that does not exist
- **THEN** the method SHALL return `Err(AppError::NotFound(msg))` where `msg.contains("Announcement not found")` AND `SELECT COUNT(*) FROM audit_logs WHERE action = 'update_announcement' AND entity_id = missing_id` SHALL be `0`

#### Scenario: delete against missing id returns 4xx and writes no audit

- **WHEN** a handler invokes `svc.delete(actor, missing_id)` against an id that does not exist
- **THEN** the method SHALL return `Err(AppError::NotFound(_))` AND no `delete_announcement` audit row SHALL be written for that id

#### Scenario: publish against missing id returns 4xx and writes no audit

- **WHEN** a handler invokes `svc.publish(actor, missing_id)` against an id that does not exist
- **THEN** the method SHALL return `Err(AppError::NotFound(_))` AND no `publish_announcement` audit row SHALL be written for that id

#### Scenario: unpublish against missing id returns 4xx and writes no audit

- **WHEN** a handler invokes `svc.unpublish(actor, missing_id)` against an id that does not exist
- **THEN** the method SHALL return `Err(AppError::NotFound(_))` AND no `unpublish_announcement` audit row SHALL be written for that id

