# Tasks

Spec-only change. The triggering defect is already fixed; these tasks confirm the
rule is enforceable and that no other test carries the same hazard.

## 1. Verify the already-applied fix satisfies the rule

- [ ] 1.1 Confirm `tests/series_pass_test.rs::horizon_roll_forward_seats_enrollees_on_new_occurrences`
  derives both `until_date` and the `extend_horizon` target from `DEFAULT_HORIZON`
  (currently `+90d` and `+60d` respectively), not from absolute day counts.
- [ ] 1.2 Confirm the comment there records the failure mode, so the bounds are not
  "simplified" back to absolute counts by a later contributor.

## 2. Audit the remaining extension tests

- [ ] 2.1 `tests/recurring_event_test.rs::extend_horizon_adds_missing_tail` — verify
  it stays safe. It caps at 13 weeks then clears `until_date` before extending to
  52 weeks, so its window is months wide.
- [ ] 2.2 `tests/recurring_event_test.rs::extend_horizon_respects_until_date` —
  verify it stays safe. It asserts `added == 0`, so a narrow window cannot make it
  flake; only a *false* non-zero would, which the cap prevents.
- [ ] 2.3 Grep for any other caller of `extend_horizon` or
  `extend_horizon_for_active_series` in tests and apply the rule to each.

## 3. Guard against reintroduction

- [ ] 3.1 Run the suite on a weekday other than the one the fix was verified on
  (verified on a Tuesday), or temporarily shift the anchor weekday, to confirm the
  window holds across all seven.
