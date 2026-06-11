## Context

`src/service/recurring_event_service.rs::create_series_with_initial_materialization` computes:

```rust
let now = Utc::now();
let target_horizon = (now + DEFAULT_HORIZON)
    .min(until_date.unwrap_or(DateTime::<Utc>::MAX_UTC));
let occurrence_times = generate_occurrences(
    template.start_time,  // anchor — earliest occurrence
    &rule,
    template.start_time,  // from
    target_horizon,        // strict upper bound
);
```

`DEFAULT_HORIZON` is 12 months. The materializer generates occurrences in `[anchor, target_horizon)`. Production behavior is correct: the calendar always shows ~12 months from now.

The tests, however, pin both the anchor and (for most) the `until_date` to fixed calendar timestamps in May–December 2026. As "now" advances, the gap between the fixed anchor and the moving `target_horizon` changes — that's the drift.

Failure modes by test class:

| Test pattern                              | Drift behavior                                  |
|-------------------------------------------|-------------------------------------------------|
| `until_date = None`, count assertion      | Count grows as "now" advances past anchor. *Already failing.* |
| `until_date = Some(<future date>)`        | Works while `now < until_date`. Fails when `now >= until_date` because `target_horizon = until_date < now`, and `generate_occurrences(anchor, …, anchor, target_horizon)` returns empty if `anchor >= target_horizon`. Triggers ~2 months out for the `Aug 2026` cases, later for others. |

So the visible bug (line 123) is the first symptom of a wider pattern. The fix is to remove the fixed-calendar assumption from the test inputs entirely.

## Goals / Non-Goals

**Goals:**
- The recurring-event test suite passes today and continues to pass as wall-clock time advances.
- Anchors and `until_date`s are computed relative to "now" so the temporal relationship the tests assert remains stable.
- The fix is contained to `tests/recurring_event_test.rs`.

**Non-Goals:**
- Changing the production materializer's behavior. The "now + 12 months" horizon is the right shape for the production calendar; the bug is in the test inputs.
- Auditing every other test file for the same pattern. A grep should be done as part of this change to flag any other drift candidates, but addressing them is deferred unless they're blocking.
- Mocking `Utc::now()` in the materializer. Tempting (would make tests fully deterministic) but introduces a clock-injection surface that changes the production signature. Out of scope.
- Refactoring how `template()` is built or how `build()` sets up helpers.

## Decisions

### D1. Helper functions, not constants

`next_tuesday_anchor()` is a function (not a `const`) because it depends on `Utc::now()`. Same for `next_2nd_wednesday_anchor()`. Tests call them once each at the top of the test body — same shape as the existing `Utc.with_ymd_and_hms(...)` literal, just function-call form.

### D2. Anchors are always in the near future, not the past

The helper returns the next Tuesday after `now + 1 day`. The `+1 day` buffer is so the anchor is unambiguously in the future regardless of what time of day the test runs — without it, a test running on a Tuesday afternoon might get "today" as the anchor and the materializer might or might not include it depending on hour.

The 18:00 UTC time is preserved from the original test (matches the meaning: "evening club meeting").

### D3. `until_date` computations are relative to the new anchor

The existing tests have anchor=`2026-05-05` and various untils like `2026-08-01` (~13 weeks later), `2026-06-30` (~8 weeks), `2026-07-01` (~8 weeks), `2026-12-01` (~30 weeks), `2026-12-31` (~34 weeks), `2027-05-05` (~52 weeks).

The helper `weeks_after_anchor(weeks: i64) -> DateTime<Utc>` produces an `until_date` at the same relative offset. The test expectation (e.g., "until_date caps at ~13 weeks → ~13 weekly occurrences") stays accurate.

Some untils use specific times like `2026-08-01 00:00:00`. The helper uses the same time-of-day as the anchor by default (18:00 UTC); if a test cares about the time component matching midnight, it can do its own arithmetic. Likely none do — let me verify in the implementation.

### D4. The 2nd-Wednesday case gets its own helper

`next_2nd_wednesday_anchor()` returns the 2nd Wednesday of the next calendar month, at 19:00 UTC (matching the original test's hour). That test asserts the monthly-by-weekday-ordinal rule generator, so the anchor MUST be a 2nd Wednesday — the helper computes "month + 1, find Wednesdays, take the second."

### D5. Grep adjacent test files

The proposal notes that other test files may have the same hardcoded-anchor pattern. As part of implementation, a `grep -rn "with_ymd_and_hms(2026" tests/` documents the broader exposure. Any matches that look like the same drift pattern get a one-line comment in this change's commit: "next change should sweep <filename>." We don't fix them here, but we surface them.

### D6. Keep the proven `(50..=53).contains(...)` tolerance

Even with a relative anchor, the count for a "weekly Tuesday for 12 months" series isn't perfectly stable — it depends on what day of the year the anchor lands on. The existing 50–53 tolerance is well-chosen; keep it.

## Risks / Trade-offs

- **Risk**: a helper returns a date during a DST transition or other calendar oddity. → **Mitigation**: helpers work in UTC. No DST in UTC. Edge cases like leap days don't matter for "next Tuesday" computations.
- **Risk**: a test asserts a count that's sensitive to the calendar month the anchor lands in (e.g., "January has 4 or 5 Tuesdays depending on alignment"). → **Mitigation**: the existing tolerance ranges (50–53, etc.) absorb this. Confirm during implementation that no test asserts an exact count tied to a calendar-month property.
- **Trade-off**: tests are slightly harder to debug — failure messages will reference dates derived from "now," not nice round calendar dates. Acceptable; the failure messages already include the anchor in their output via `assert!`-with-message patterns where it matters.
- **Trade-off**: tests run different code paths in winter vs. summer (e.g., monthly-by-weekday-ordinal will pick a different month). Not a real risk if the rules are correctly month-agnostic.

## Migration Plan

Single PR.

1. Add the helpers `next_tuesday_anchor()`, `next_2nd_wednesday_anchor()`, `weeks_after_anchor(weeks)` near the top of `tests/recurring_event_test.rs`.
2. Replace the 11 occurrences of `Utc.with_ymd_and_hms(2026, 5, 5, 18, 0, 0).unwrap()` with calls to `next_tuesday_anchor()`. Store the result in `let anchor = …` at the start of each test (same shape as today).
3. Replace the 1 occurrence of `Utc.with_ymd_and_hms(2026, 5, 13, 19, 0, 0).unwrap()` with `next_2nd_wednesday_anchor()`.
4. Replace each hardcoded `until_date` with the appropriate `weeks_after_anchor(N)` call. Pick `N` from the table below to preserve the original temporal relationships:
   - `2026-08-01` ≈ anchor + 13 weeks → `weeks_after_anchor(13)`
   - `2026-06-30` ≈ anchor + 8 weeks → `weeks_after_anchor(8)`
   - `2026-07-01` ≈ anchor + 8 weeks → `weeks_after_anchor(8)`
   - `2026-12-01` ≈ anchor + 30 weeks → `weeks_after_anchor(30)`
   - `2026-12-31` ≈ anchor + 34 weeks → `weeks_after_anchor(34)`
   - `2027-05-05` ≈ anchor + 52 weeks → `weeks_after_anchor(52)`
5. Run `cargo test --features test-utils --test recurring_event_test`. All 11 tests in that file pass.
6. Run `cargo test --features test-utils` against the full suite. Confirm no regressions elsewhere.
7. Grep `tests/` for `with_ymd_and_hms(2026` to surface other hardcoded-date tests. If matches are found, leave a one-line note in the commit message; do not fix them in this change.
