## Why

`a14-fix-recurring-event-test-anchors` added an `admin-events` spec requirement that tests of the recurring-event materializer SHALL use anchors computed relative to `Utc::now()` at runtime. While implementing `a14`, the autocoder surfaced ~9 (actually 12, by exact grep) inline unit tests inside `src/service/event_admin_service.rs` that violate the same rule — they use hardcoded `2026-08-04` Tuesday anchors and `2026-10-01` untils, the same pattern just fixed in `tests/recurring_event_test.rs`.

Honest framing: not all of these tests will fail in the immediate-and-obvious way `weekly_creates_about_52_occurrences` did. The production materializer doesn't filter by "now" — it filters by `[anchor, target_horizon)` with `target_horizon = min(now+12mo, until_date)`. For tests with fixed anchor AND fixed until that bracket each other, the produced occurrence set may stay stable indefinitely. So "late summer 2026" failures are likely but not certain — depends on each test's specific assertions.

Regardless, the rule from `a14` is the rule. Hardcoded calendar timestamps in materializer tests are a defect *on principle* — they're a maintenance trap waiting for the next calendar-edge bug to bite. Fix the pattern now while the cost is small, in the same week as the original `a14` work, so the rule lands across both call sites.

## What Changes

- **Inside `src/service/event_admin_service.rs`'s `#[cfg(test)] mod tests`**, add helper functions equivalent to those in `tests/recurring_event_test.rs`:
  - `fn next_saturday_anchor() -> DateTime<Utc>` — next Saturday at 18:00 UTC, used for single-event tests where the start time has no recurring-rule constraint and just needs to be in the near future.
  - `fn next_tuesday_anchor() -> DateTime<Utc>` — next Tuesday at 18:00 UTC, used for recurring-Tuesday tests.
  - `fn weeks_after(anchor: DateTime<Utc>, weeks: i64) -> DateTime<Utc>` — relative until-date helper. Takes the anchor explicitly (cleaner than `weeks_after_anchor(N)` from `a14` because there are two anchor flavors here; making the anchor a parameter avoids guessing which one).
- **Replace 12 hardcoded timestamps** with calls to those helpers:
  - 4 occurrences of `2026-08-01` (single-event `start`) → `next_saturday_anchor()`.
  - 5 occurrences of `2026-08-04` (recurring-Tuesday `start`) → `next_tuesday_anchor()`.
  - 3 occurrences of `2026-10-01` (`until`) → `weeks_after(anchor, 8)` — 8 weeks past the Tuesday anchor matches the original ~8.5-week spread.
- **Scope discipline**: helpers stay local to this file's test module. They aren't shared with `tests/recurring_event_test.rs` because that file lives in a different compilation unit. If a future sweep finds drift in a third location, that's the right moment to extract a shared `src/service/test_helpers.rs`; doing it for one duplicate is premature.
- **Out of scope**:
  - Refactoring the test setup helpers (`fresh_pool`, `make_service`, `make_actor`, `audit_count`).
  - Adding new test coverage.
  - Touching any other file. The autocoder explicitly noted that `tests/member_template_snapshots.rs` and `src/web/templates/filters.rs` have hardcoded dates as formatting/snapshot fixtures, not materializer inputs — those are correct as-is.

## Capabilities

### New Capabilities

(None.)

### Modified Capabilities
- `admin-events`: clarify the existing `a14`-added requirement to make explicit that it covers `#[cfg(test)]` modules inside `src/` files, not only standalone test files under `tests/`. Pure clarification — the rule's intent already covered both; this surfaces it.

## Impact

- **Code**: `src/service/event_admin_service.rs` test module only. Adds ~25 lines of helpers, replaces 12 hardcoded-timestamp lines with helper calls.
- **Wire shape**: no production code changes.
- **Test outcome**: `cargo test --features test-utils --lib event_admin_service` continues to pass. Suite stays green now and remains green through late 2026 and beyond.
- **Risk**: very low. Pure test refactor. The helper outputs match what the materializer expects (Tuesday anchor, near-future, valid until-date).
- **Dependency**: none. The `a14` spec language is already in canonical specs (after archive); this change extends compliance to the second offending location.
