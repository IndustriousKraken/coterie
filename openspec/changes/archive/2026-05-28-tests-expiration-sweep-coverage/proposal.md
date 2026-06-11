## Why

`src/service/billing_service/expiration.rs::check_expired_members` is
the daily sweep that flips Active members past dues + grace period to
`Expired`, kills their live sessions, and dispatches
`IntegrationEvent::MemberExpired` so integrations (Discord role swap,
etc.) can react. It is invoked from the daily background job in
`main.rs` — there is no admin handler that calls it. Despite being a
load-bearing piece of the member lifecycle, the function has **zero**
test coverage:

- No file under `tests/` invokes `check_expired_members` or
  `expiration::Expiration` (grep confirms — only HTML golden files
  with "expired" in the name match, and those are template
  snapshots, not function tests).
- No inline `#[cfg(test)] mod tests` block exists in
  `expiration.rs`.

The function has several behaviors worth locking in:

1. **Bypass-dues members are NOT swept** (line 60: `AND bypass_dues =
   0`). A regression that drops this clause would silently expire
   honorary/founder members.
2. **The grace period is respected** (line 59: `date(dues_paid_until,
   '+' || ? || ' days') < date('now')`). A member 1 day past dues but
   inside the 3-day grace window should NOT flip.
3. **Live sessions are killed** (line 80: `DELETE FROM sessions
   WHERE member_id IN ...`). A member whose status flips to Expired
   should not retain a valid `sessions` row.
4. **`MemberExpired` is dispatched per affected member** (line 102).
5. **Session-delete failure does not roll back the status flip**
   (the `tracing::warn!` + continue branch at line 86): the SQL DELETE
   error is logged but the function returns the count of expired
   members anyway. This is a deliberate "session cleanup is best-
   effort" choice that should be locked in by test.

The grace-period default is `3` (line 123: `unwrap_or(3)`), but if the
`membership.grace_period_days` setting is unset or unreadable the
default applies — also untested.

## What Changes

Add a new `tests/expiration_test.rs` integration test file with
five `#[tokio::test]` cases that drive `check_expired_members`
against an in-memory SQLite pool and a `RecordingIntegration`
(co-located, same shape as the one in
`tests/auto_renew_alert_test.rs`).

## Impact

- New file `tests/expiration_test.rs` (no existing file to extend —
  no expiration tests anywhere in the repo today). Uses
  `mod common; use common::fresh_pool;` to share the
  in-memory-SQLite-with-migrations harness, matching the convention
  in `tests/auto_renew_alert_test.rs`.
- No production code change. No existing tests are modified or
  deleted.
