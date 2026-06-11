## ADDED Requirements

### Requirement: Admin can create, view, update, and delete events

Admin event management SHALL be available at `/portal/admin/events` with the routes:
- `GET /portal/admin/events` — listing.
- `GET /portal/admin/events/new` and `POST /portal/admin/events/new` — create.
- `GET /portal/admin/events/:id` — detail.
- `POST /portal/admin/events/:id/update` — update.
- `POST /portal/admin/events/:id/delete` — delete.

Forms SHALL accept `multipart/form-data` to permit image uploads. CSRF MUST be the first multipart field so the top-level layer can validate before buffering large bodies.

#### Scenario: Event create with image succeeds

- **WHEN** an admin submits an event-create form with a JPEG image and `csrf_token` as the first field
- **THEN** the CSRF middleware SHALL validate the token, the handler SHALL persist the event via `event_repo`, then the handler SHALL call `audit_service.log` and dispatch `IntegrationEvent::EventPublished` (audit and integration emission are handler-owned for events)

#### Scenario: Multipart over the size cap is rejected with 400

- **WHEN** a multipart body exceeds 12MB
- **THEN** the CSRF middleware SHALL return a 400 "Request body too large"

### Requirement: Recurring events are managed as series

Events with a recurrence pattern SHALL be stored as a series with a generator that produces concrete event instances. The recurring-event service SHALL be the only entry point for materializing instances. Editing the series SHALL update future occurrences without rewriting historical ones.

#### Scenario: Editing series updates only future instances

- **WHEN** an admin edits the series description after some past occurrences exist
- **THEN** historical instances SHALL keep their original description; future instances SHALL adopt the edit

#### Scenario: Deleting series cancels future instances

- **WHEN** an admin deletes a series
- **THEN** future materialized instances SHALL be removed; past instances SHALL be retained for audit
