## ADDED Requirements

### Requirement: Announcements can carry a future publish time

The `announcements` table SHALL include a nullable `scheduled_publish_at: Option<DateTime<Utc>>` column. A Draft announcement with `scheduled_publish_at` set is considered "scheduled." A Draft without it is just a Draft. A Published row's `scheduled_publish_at` is irrelevant (the runner clears it on transition).

The new-announcement and edit-announcement admin forms SHALL accept an optional `scheduled_publish_at` input (HTML `datetime-local`). Empty input persists as `None`.

#### Scenario: Admin creates a scheduled Draft

- **WHEN** an admin submits the new-announcement form with a `scheduled_publish_at` value of "next Tuesday at 09:00 UTC"
- **THEN** the announcement is created with `status = Draft` and `scheduled_publish_at = <that timestamp>`

#### Scenario: Admin clears a schedule on edit

- **WHEN** an admin edits a scheduled Draft and submits the form with an empty `scheduled_publish_at` input
- **THEN** the announcement's `scheduled_publish_at` is set to `None`; the row stays Draft

### Requirement: A background runner publishes scheduled announcements at their time

`AnnouncementAdminService::publish_scheduled()` SHALL be called from `BillingRunner::run_cycle`. The method SHALL find all Draft announcements whose `scheduled_publish_at <= now` and, for each, atomically flip the row to Published, audit-log `auto_publish_announcement` with `actor_id = None`, and dispatch `IntegrationEvent::AnnouncementPublished(announcement)`.

Precision is bounded by the runner tick interval (currently ~1 hour). A scheduled announcement fires in the first tick on or after its scheduled time.

#### Scenario: Past-due Draft fires on the next tick

- **WHEN** a Draft has `scheduled_publish_at = (now - 5 minutes)`
- **THEN** the next runner tick SHALL flip it to Published, write an audit row, and dispatch `AnnouncementPublished`

#### Scenario: Future Draft is not touched

- **WHEN** a Draft has `scheduled_publish_at = (now + 2 hours)`
- **THEN** the next runner tick (within the next hour) SHALL NOT touch it

#### Scenario: Manual publish before scheduled time wins

- **WHEN** an admin manually publishes a scheduled Draft via `/portal/admin/announcements/:id/publish` before its scheduled time arrives
- **THEN** the row is Published with `actor_id = <admin>` on the audit row; when the scheduled time later arrives, the runner's atomic conditional UPDATE matches zero rows (status is already Published) and no second event is dispatched

#### Scenario: System-initiated audit row has no actor

- **WHEN** the runner auto-publishes a scheduled announcement
- **THEN** the resulting `audit_logs` row SHALL have `actor_id = NULL` (matching the existing audit-log spec's "Option<UUID> — NULL for system-initiated entries")

### Requirement: The Draft→Published transition is atomic

The repository method that the runner uses SHALL execute a conditional UPDATE that flips status only when the row is still Draft. Two concurrent runner ticks (e.g., across a server restart that overlaps with the next tick) SHALL NOT both dispatch the integration event.

#### Scenario: Conditional update prevents double-dispatch

- **WHEN** two concurrent calls to `mark_published_now(id)` run against the same Draft row
- **THEN** exactly one SHALL return true (the winner does the audit + dispatch); the other SHALL return false (and skip the dispatch)
