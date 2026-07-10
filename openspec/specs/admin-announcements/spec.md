# admin-announcements Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Admin manages announcements with publish state

Admin announcement management SHALL be available at `/portal/admin/announcements` with:
- `GET /portal/admin/announcements` — listing.
- `GET /portal/admin/announcements/new` and `POST /portal/admin/announcements/new` — create.
- `GET /portal/admin/announcements/:id` — detail.
- `POST /portal/admin/announcements/:id/update` — update.
- `POST /portal/admin/announcements/:id/delete` — delete.
- `POST /portal/admin/announcements/:id/publish` — set published.
- `POST /portal/admin/announcements/:id/unpublish` — set unpublished.

Announcements SHALL have a publish state that controls whether they appear in `/public/announcements` (when also marked public) and `/public/feed/rss`.

#### Scenario: Unpublished announcement is invisible to public feeds

- **WHEN** an admin saves an announcement as a draft (unpublished)
- **THEN** it SHALL NOT appear in any public read or RSS feed

#### Scenario: Publishing emits an audit-log entry and integration event from the handler

- **WHEN** an admin publishes an announcement
- **THEN** the handler SHALL call `audit_service.log` (recording actor, target, transition) AND dispatch `IntegrationEvent::AnnouncementPublished`. Audit/integration emission are handler-owned for announcements.

### Requirement: User-supplied announcement content is escaped on render

Templates rendering announcement content SHALL escape HTML by default. Any opt-in to render-as-HTML SHALL be limited to admin-curated, sanitized content (not free-form user input) so a stored XSS via announcements is prevented.

#### Scenario: Script tag in body is escaped

- **WHEN** an admin saves an announcement whose body contains `<script>alert(1)</script>`
- **THEN** rendered pages SHALL display the literal text, not execute the script

### Requirement: Announcement-admin handlers route through AnnouncementAdminService

Admin announcement mutation handlers SHALL parse the wire shape (form, path params, current user) and call `AnnouncementAdminService` for the actual mutation work. Handlers SHALL NOT call `announcement_repo`, `audit_service.log`, or `integration_manager.handle_event` directly for these flows.

Wire shape (URLs, form bodies, HTMX response fragments) is unchanged.

#### Scenario: admin_create_announcement routes through the service

- **WHEN** an admin submits the new-announcement form
- **THEN** the handler SHALL build a `CreateAnnouncementInput` from the parsed form (including the `publish_now` flag) and call `AnnouncementAdminService::create(current_user.id, input)`

#### Scenario: admin_publish_announcement routes through the service

- **WHEN** an admin clicks Publish on a Draft announcement
- **THEN** the handler SHALL call `AnnouncementAdminService::publish(current_user.id, announcement_id)`; the integration dispatch happens inside the service

### Requirement: Admin announcement form accepts optional scheduled publish time

The new-announcement form (`POST /portal/admin/announcements/new`) and edit-announcement form (`POST /portal/admin/announcements/:id/update`) SHALL each accept an optional `scheduled_publish_at` form field. The field SHALL be rendered as an HTML `datetime-local` input. Empty input means "no schedule." A non-empty input SHALL be interpreted as a **wall-clock in the organization timezone** and stored as a naive wall-clock paired with a `scheduled_publish_timezone` IANA zone frozen from `org.timezone` at submission; the true UTC instant is derived from (wall-clock, zone) at compare time (see the `scheduled-announcement-publish` capability).

The admin detail page SHALL display the scheduled time in the org timezone with its zone abbreviation (e.g. "9:00 AM EDT"), alongside the existing status indicator.

#### Scenario: Form submission with schedule

- **WHEN** an admin submits the new-announcement form with `scheduled_publish_at = "2026-06-01T09:00"` and the org timezone is `America/New_York`
- **THEN** the resulting `CreateAnnouncementInput` carries the wall-clock `2026-06-01T09:00` and zone `America/New_York` (derived instant `2026-06-01T13:00Z`); the row is saved as Draft with that wall-clock and zone; `publish_now` is implicitly false

#### Scenario: Form submission without schedule

- **WHEN** the form omits the field or submits empty
- **THEN** the resulting input carries `scheduled_publish_at = None`; behavior matches today (Draft if `publish_now` is false; Published if true)

#### Scenario: Form combining publish_now and schedule

- **WHEN** the form has both `publish_now = true` AND `scheduled_publish_at = <future>`
- **THEN** `publish_now` wins (the row goes Published immediately); the schedule field is dropped. This is the simpler precedence; alternative would be to reject the combo, but the current shape favors "publish now, don't get clever."

### Requirement: Announcement list preview tolerates multi-byte bodies

The admin announcements list (`GET /portal/admin/announcements`) SHALL build each row's content preview without panicking, regardless of the announcement body's length or UTF-8 content. The preview truncation SHALL cut on a UTF-8 character boundary, never on a raw byte index.

#### Scenario: Announcement body with a multi-byte character at the truncation boundary renders safely

- **GIVEN** an announcement whose body is longer than the preview limit and contains a multi-byte UTF-8 character (e.g. an emoji) straddling the limit
- **WHEN** an admin loads `GET /portal/admin/announcements`
- **THEN** the request SHALL complete without panicking and the preview SHALL be truncated on a character boundary with an ellipsis appended

#### Scenario: Short ASCII bodies are shown in full

- **WHEN** an announcement body is plain ASCII at or below the preview limit
- **THEN** the preview SHALL equal the full body with no ellipsis appended

