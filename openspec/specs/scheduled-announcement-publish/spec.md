# scheduled-announcement-publish Specification

## Purpose
TBD - created by archiving change a11-scheduled-announcement-publish. Update Purpose after archive.
## Requirements
### Requirement: Announcements can carry a future publish time

The `announcements` table SHALL include a nullable `scheduled_publish_at`
column holding a **local wall-clock** (a naive timestamp), paired with a
`scheduled_publish_timezone` IANA zone frozen from `org.timezone` at
scheduling. A Draft announcement with `scheduled_publish_at` set is
considered "scheduled." A Draft without it is just a Draft. A Published row's
`scheduled_publish_at` is irrelevant (the runner clears it on transition).

The new-announcement and edit-announcement admin forms SHALL accept an
optional `scheduled_publish_at` input (HTML `datetime-local`), interpreted as
a wall-clock in the org timezone; empty input persists as `None`. The true
UTC instant SHALL be derived from (wall-clock, zone) at compare time, so a
later change to the zone's rules does not move the intended publish time.

#### Scenario: Admin creates a scheduled Draft in the org timezone

- **WHEN** an admin submits the new-announcement form with a
  `scheduled_publish_at` value of "next Tuesday at 09:00" and the org timezone
  is `America/New_York`
- **THEN** the announcement is created with `status = Draft`, the stored
  wall-clock `09:00`, the frozen zone `America/New_York`, and a derived
  instant of `13:00Z` (in EDT)

#### Scenario: Admin clears a schedule on edit

- **WHEN** an admin edits a scheduled Draft and submits the form with an empty
  `scheduled_publish_at` input
- **THEN** the announcement's `scheduled_publish_at` is set to `None`; the row
  stays Draft

### Requirement: A background runner publishes scheduled announcements at their time

`AnnouncementAdminService::publish_scheduled()` SHALL be called from
`BillingRunner::run_cycle`. The method SHALL find all Draft announcements
whose **derived UTC instant** (from the stored wall-clock and frozen zone) is
`<= now` and, for each, atomically flip the row to Published, audit-log
`auto_publish_announcement` with `actor_id = None`, and dispatch
`IntegrationEvent::AnnouncementPublished(announcement)`. The SQL query SHALL
compare a widened coarse bound (the raw wall-clock against `now` plus the
widest IANA offset), with the exact `derived_utc <= now` test performed in
Rust, because SQLite cannot do the timezone math.

Precision is bounded by the runner tick interval (currently ~1 hour). A
scheduled announcement fires in the first tick on or after its true instant,
so it publishes at the intended org-local time rather than offset-hours early.

#### Scenario: Past-due Draft fires on the next tick

- **WHEN** a Draft's derived UTC instant is `(now - 5 minutes)`
- **THEN** the next runner tick SHALL flip it to Published, write an audit row,
  and dispatch `AnnouncementPublished`

#### Scenario: A non-UTC-org Draft does not publish early

- **WHEN** an `America/New_York` org has a Draft scheduled for `09:00` local,
  whose true instant `13:00Z` is still in the future
- **THEN** the runner SHALL NOT publish it until `now >= 13:00Z`, not at
  `09:00Z` (four hours early)

#### Scenario: Manual publish before scheduled time wins

- **WHEN** an admin manually publishes a scheduled Draft via
  `/portal/admin/announcements/:id/publish` before its scheduled time arrives
- **THEN** the row is Published with `actor_id = <admin>` on the audit row;
  when the scheduled time later arrives, the runner's atomic conditional
  UPDATE matches zero rows (status is already Published) and no second event
  is dispatched

#### Scenario: System-initiated audit row has no actor

- **WHEN** the runner auto-publishes a scheduled announcement
- **THEN** the resulting `audit_logs` row SHALL have `actor_id = NULL`

### Requirement: The Draft→Published transition is atomic

The repository method that the runner uses SHALL execute a conditional UPDATE that flips status only when the row is still Draft. Two concurrent runner ticks (e.g., across a server restart that overlaps with the next tick) SHALL NOT both dispatch the integration event.

#### Scenario: Conditional update prevents double-dispatch

- **WHEN** two concurrent calls to `mark_published_now(id)` run against the same Draft row
- **THEN** exactly one SHALL return true (the winner does the audit + dispatch); the other SHALL return false (and skip the dispatch)

