## Why

`tests/recurring_event_test.rs::weekly_creates_about_52_occurrences` fails today. The test pins a fixed anchor at `2026-05-05` and asserts that the materializer produces 50–53 occurrences, but the materializer's horizon is `now + 12 months` — not `anchor + 12 months`. As real-world "now" advances past the anchor, the window grows beyond a single year from the anchor and the count drifts upward. Today (2026-05-18) the count is 54, just outside the test's tolerance.

The same pattern is a time-bomb in the other ten tests in the file that use the same fixed `2026-05-05` anchor. They currently pass because each one constrains the materializer with an explicit `until_date` like `2026-08-01`. But once "now" advances past those `until_date`s, `target_horizon` will resolve to a past timestamp and `generate_occurrences` will return an empty list. The first such failure is roughly two months out (`2026-08-01` untils), with cascade failures through end-of-2026 for the rest.

Fix once, end the drift permanently: anchor everything relative to "now" so the test inputs always sit in the same temporal position no matter when the suite runs.

## What Changes

- **Add a private test helper** in `tests/recurring_event_test.rs`:
  ```rust
  /// Returns the next Tuesday at 18:00 UTC strictly after `now + 1 day`.
  /// Used as the anchor for tests so all date computations are relative
  /// to when the suite runs.
  fn next_tuesday_anchor() -> DateTime<Utc> { … }

  /// Returns `next_tuesday_anchor()` shifted forward by `weeks` weeks.
  /// Used to compute `until_date` values that were previously
  /// hardcoded like `2026-08-01`.
  fn weeks_after_anchor(weeks: i64) -> DateTime<Utc> { … }
  ```
- **One test exception**: the "2nd Wednesday" test at line 407 anchors on a 2nd-Wednesday-of-some-month, not the next Tuesday. It gets its own helper `next_2nd_wednesday_anchor()` that returns the second Wednesday of the next month.
- **Replace all 11 occurrences** of `Utc.with_ymd_and_hms(2026, 5, 5, 18, 0, 0).unwrap()` with `next_tuesday_anchor()`.
- **Replace the one occurrence** of `Utc.with_ymd_and_hms(2026, 5, 13, 19, 0, 0).unwrap()` with `next_2nd_wednesday_anchor()`.
- **Replace the dependent `until_date` values** (`2026-08-01`, `2026-06-30`, `2026-07-01`, `2026-12-01`, `2026-12-31`, `2027-05-05`) with relative computations like `weeks_after_anchor(13)` (≈ 3 months) so the relationship `anchor + offset = until` stays semantically identical.
- **Out of scope**: changing the materializer's horizon logic. The bug is in the test inputs, not the production code. The production code's "now + 12 months" horizon is correct for its job (rolling forward as time passes); the tests just need to feed it relative anchors.
- **Out of scope**: other test files. A grep for `with_ymd_and_hms(2026` in `tests/` may surface similar hardcoded anchors elsewhere; if any exist, flag in design.md but address only the ones blocking `recurring_event_test`.

## Capabilities

### New Capabilities

(None — this is a test-only fix.)

### Modified Capabilities

(None — no production capability changes. The recurring-event spec is unchanged; the test was wrong, not the spec.)

## Impact

- **Code**: `tests/recurring_event_test.rs` only. Adds ~20 lines of helpers, replaces ~20 hardcoded-timestamp lines with helper calls. Net line count roughly neutral.
- **Wire shape**: no production code changes.
- **Risk**: very low. Pure test refactor; the helper outputs match the production materializer's expectations (Tuesday-at-18:00-UTC anchor, 12-month horizon).
- **Test outcome**: `cargo test --features test-utils --test recurring_event_test` returns to all-green. The full suite goes green for the first time since the May 2026 calendar drift began.
- **Dependency**: none. Independent of every other queued change.
