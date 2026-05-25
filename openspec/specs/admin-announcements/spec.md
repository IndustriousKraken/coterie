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

### Requirement: Admin announcement form accepts optional scheduled publish time

The new-announcement form (`POST /portal/admin/announcements/new`) and edit-announcement form (`POST /portal/admin/announcements/:id/update`) SHALL each accept an optional `scheduled_publish_at` form field. The field SHALL be rendered as an HTML `datetime-local` input. Empty input means "no schedule." A non-empty input parses as a `DateTime<Utc>` (treating the form value as UTC for v1; per-timezone handling is a future change).

The admin detail page SHALL display the scheduled time if set, alongside the existing status indicator.

#### Scenario: Form submission with schedule

- **WHEN** an admin submits the new-announcement form with `scheduled_publish_at = "2026-06-01T09:00"`
- **THEN** the resulting `CreateAnnouncementInput` carries `scheduled_publish_at = Some(2026-06-01T09:00 UTC)`; the row is saved as Draft with that timestamp; `publish_now` is implicitly false

#### Scenario: Form submission without schedule

- **WHEN** the form omits the field or submits empty
- **THEN** the resulting input carries `scheduled_publish_at = None`; behavior matches today (Draft if `publish_now` is false; Published if true)

#### Scenario: Form combining publish_now and schedule

- **WHEN** the form has both `publish_now = true` AND `scheduled_publish_at = <future>`
- **THEN** `publish_now` wins (the row goes Published immediately); the schedule field is dropped. This is the simpler precedence; alternative would be to reject the combo, but the current shape favors "publish now, don't get clever."

