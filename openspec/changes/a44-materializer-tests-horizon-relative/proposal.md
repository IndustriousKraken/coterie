# a44-materializer-tests-horizon-relative

## Why

`admin-events` already requires materializer tests to use **runtime-relative**
anchors instead of fixed calendar dates. On 2026-07-28 a test that fully complied
with that rule failed in CI anyway:

`series_pass_test::horizon_roll_forward_seats_enrollees_on_new_occurrences` set
`until_date = Utc::now() + 365 days` — properly runtime-relative — and then
asserted that `extend_horizon` materialized new occurrences. But
`DEFAULT_HORIZON` is 52 weeks (364 days), and `extend_horizon` caps its target at
`until_date`. So initial materialization already filled to day 364 and the
extension window was **one day wide**. A weekly-Tuesday series only has an
occurrence in a one-day window when that day happens to be a Tuesday — which
made the test pass only when CI ran on a Monday. It passed Monday 2026-07-27 and
failed Tuesday 2026-07-28.

Runtime-relative was necessary but not sufficient. A test that asserts on
horizon *extension* must position its bounds relative to `DEFAULT_HORIZON`,
because that constant — not the wall clock — is what decides how much room the
extension has to work in. Nothing in canon said so, so a compliant-looking test
shipped a one-in-seven flake.

This is a **change** rather than an issue: the code fix is already made, but the
rule that would have prevented it does not exist in canon and needs to.

## What Changes

- The existing `admin-events` requirement on materializer test anchors gains a
  second rule: a test asserting that a horizon extension **produced** occurrences
  SHALL derive both its `until_date` and its extension target from
  `DEFAULT_HORIZON`, and SHALL leave a window wide enough to contain at least one
  occurrence of the rule under test regardless of the weekday the suite runs on.
- The requirement also names the diagnostic signature, so the next person seeing
  it recognises the class immediately: an occurrence-count assertion that passes
  locally and fails in CI on some days is a horizon-window bug, not flaky
  infrastructure to be retried.

## Impact

- **Spec:** `admin-events` — 1 MODIFIED requirement (the existing runtime-relative
  rule, extended). No new capability, no other capability touched.
- **Code:** none required by this change. The triggering defect was already fixed
  in `tests/series_pass_test.rs`, which now derives both bounds from
  `DEFAULT_HORIZON` with a 60-day window and passes on every weekday (verified by
  running it on a Tuesday, the weekday that broke CI).
- **Follow-on for implementers:** the two other `extend_horizon` tests in
  `tests/recurring_event_test.rs` were audited and are already safe — they use
  deliberately short caps (8 and 13 weeks) well inside the horizon, so their
  extension windows are months wide. No further test changes are needed.
