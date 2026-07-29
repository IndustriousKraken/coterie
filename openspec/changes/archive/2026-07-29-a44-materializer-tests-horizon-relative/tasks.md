# Tasks

Spec-only change. The triggering defect is already fixed; these tasks confirm the
rule is enforceable and that no other test carries the same hazard.

## 1. Verify the already-applied fix satisfies the rule

- [x] 1.1 Confirm `tests/series_pass_test.rs::horizon_roll_forward_seats_enrollees_on_new_occurrences`
  derives both `until_date` and the `extend_horizon` target from `DEFAULT_HORIZON`
  (currently `+90d` and `+60d` respectively), not from absolute day counts.
  Confirmed: `series_pass_test.rs:733` and `:762`.
- [x] 1.2 Confirm the comment there records the failure mode, so the bounds are not
  "simplified" back to absolute counts by a later contributor.
  Confirmed: `series_pass_test.rs:704-717` names the clamp, the one-day window and
  the Monday/Tuesday CI split; the assertion message at `:767` restates the bounds.

## 2. Audit the remaining extension tests

- [x] 2.1 `tests/recurring_event_test.rs::extend_horizon_adds_missing_tail` — verify
  it stays safe. It caps at 13 weeks then clears `until_date` before extending to
  52 weeks, so its window is months wide.
  Confirmed safe: both bounds derive from the same `next_tuesday_anchor()`, so the
  ~39-week window is weekday-independent and `added > 30` has ~9 weeks of slack.
- [x] 2.2 `tests/recurring_event_test.rs::extend_horizon_respects_until_date` —
  verify it stays safe. It asserts `added == 0`, so a narrow window cannot make it
  flake; only a *false* non-zero would, which the cap prevents.
  Confirmed safe.
- [x] 2.3 Grep for any other caller of `extend_horizon` or
  `extend_horizon_for_active_series` in tests and apply the rule to each.
  Six other callers, all anchor-relative:
  `recurring_event_test.rs::extend_horizon_for_active_series_processes_every_series`
  (cap cleared, ~44-week window) and four in
  `src/service/event_admin_service_tests.rs`
  (`extend_horizon_after_cancelling_{boundary,first}_occurrence_keeps_indices_aligned`,
  `extend_horizon_with_no_cancellations_assigns_contiguous_indices`,
  `materializer_re_applies_override_on_extend`) — all safe.
  One defect found and fixed:
  `event_admin_service_tests.rs::cancelled_occurrence_does_not_reappear_after_materializer_run`
  extended a series still capped at `anchor + 12 weeks`, so `extend_horizon`
  clamped the target back to `materialized_through` and the "materializer run" was
  a zero-width no-op — the assertion could never fail. Now clears `until_date`
  first and asserts `added > 0` so the window cannot silently close again
  (verified: reverting the cap-clear makes the new assertion fire).

## 3. Guard against reintroduction

- [x] 3.1 Run the suite on a weekday other than the one the fix was verified on
  (verified on a Tuesday), or temporarily shift the anchor weekday, to confirm the
  window holds across all seven.
  Both: the suite was re-run on Wednesday 2026-07-29, and a temporary sweep drove
  the same bounds through all seven recurrence weekdays (shifting the recurrence
  weekday by k days is equivalent to running the suite k days later in the week,
  since only the window-to-interval alignment matters). Current bounds:
  `added` = 8,8,8,9,9,9,9 for Mon..Sun. The same sweep against the OLD bounds
  (`until_date = now + 365d`) gives 0,0,0,1,0,0,0 — the one-in-seven flake
  reproduced, which confirms the sweep would have caught it. Sweep was temporary
  and has been removed; the permanent test keeps its single-weekday form.
