## ADDED Requirements

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
