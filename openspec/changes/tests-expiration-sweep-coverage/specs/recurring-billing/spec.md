## ADDED Requirements

### Requirement: check_expired_members respects bypass_dues and grace period

`Expiration::check_expired_members` SHALL flip a member's `status`
from `Active` to `Expired` if and only if ALL of the following hold:

- `status = 'Active'`
- `dues_paid_until IS NOT NULL`
- `date(dues_paid_until, '+<grace_days> days') < date('now')`
- `bypass_dues = 0`

A member with `bypass_dues = 1` SHALL NOT be expired regardless of
how far past `dues_paid_until` they are. A member within the grace
window SHALL NOT be expired.

#### Scenario: Member past grace is expired with MemberExpired dispatch

- **WHEN** a member has `dues_paid_until = now - 10 days`, status =
  Active, bypass_dues = 0, and the grace-period setting is `3`
- **THEN** the sweep SHALL flip status to `Expired`, return a count
  of `1`, AND dispatch exactly one `IntegrationEvent::MemberExpired`
  for that member

#### Scenario: Member within grace stays Active

- **WHEN** a member has `dues_paid_until = now - 1 day`, status =
  Active, and grace = `3`
- **THEN** the sweep SHALL return `0` and the member's status SHALL
  remain `Active`

#### Scenario: bypass_dues member is never expired

- **WHEN** a member has `bypass_dues = 1` and `dues_paid_until = now -
  999 days`
- **THEN** the sweep SHALL return `0` and the member's status SHALL
  remain `Active`

### Requirement: check_expired_members invalidates live sessions of expired members

When a member's status flips to `Expired`, the sweep SHALL DELETE
every row in `sessions` whose `member_id` matches. Session-delete
failure SHALL be logged via `tracing::warn` but SHALL NOT roll back
the status flip — the middleware still rejects `Expired` status, so
the member is bounced on the next request regardless.

#### Scenario: Expired member's sessions are removed

- **WHEN** an Active member past grace has an active `sessions` row,
  and the sweep runs
- **THEN** after the sweep the `sessions` row SHALL be gone AND the
  members row SHALL read `Expired`

### Requirement: check_expired_members uses default grace when setting is unset

The grace period SHALL be read from the
`membership.grace_period_days` setting via `SettingsService`. When
the setting is missing or unreadable, the sweep SHALL default to
`3` days.

#### Scenario: Unset grace-period setting falls back to 3 days

- **WHEN** the `membership.grace_period_days` setting has never been
  written, and a member sits at `dues_paid_until = now - 5 days`
- **THEN** the sweep SHALL expire the member (5 > 3-day default)
