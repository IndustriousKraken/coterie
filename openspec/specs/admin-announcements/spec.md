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

