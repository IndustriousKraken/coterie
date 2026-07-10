# admin-announcements Specification (delta)

## MODIFIED Requirements

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
