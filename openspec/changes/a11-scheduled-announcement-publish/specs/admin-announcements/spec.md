## ADDED Requirements

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
