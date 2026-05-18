## Context

`a14-fix-recurring-event-test-anchors` archived a spec requirement under `admin-events`:

> Tests asserting on `RecurringEventService::create_series_with_initial_materialization` (or the materializer's output more generally) SHALL compute their input anchors and `until_date` values relative to `Utc::now()` at runtime. Fixed-calendar timestamps SHALL NOT be used as test inputs to the materializer.

That `a14` fix covered `tests/recurring_event_test.rs`. While implementing it, the autocoder did a `grep -rn "with_ymd_and_hms(202" src/ tests/` and found ~12 more sites inside `src/service/event_admin_service.rs::tests`. Those didn't get fixed in `a14` (scope discipline — `a14` was about the one explicitly-failing test plus its sibling pattern in the same file).

This change applies the same rule to `event_admin_service.rs`.

## Goals / Non-Goals

**Goals:**
- The `admin-events` spec requirement applies to inline `#[cfg(test)] mod tests` blocks in `src/` files, not only standalone `tests/` files.
- All 12 sites in `event_admin_service.rs` use runtime-relative anchors.
- Test suite stays green now and through future calendar-edge dates.

**Non-Goals:**
- Extracting helpers to a shared location. Premature for one duplicate; revisit if a third location appears.
- Changing the test-setup helpers (`fresh_pool`, etc.).
- Adding new tests or test coverage.
- Sweeping other files. The autocoder confirmed the snapshot/filter tests aren't drift candidates.

## Decisions

### D1. Helpers live in the same `#[cfg(test)] mod tests` block as the tests

Adding the helpers inline keeps the diff scope tight and ensures the helpers can use any test-local types (e.g., the `Helpers` struct from `build()`, if relevant — though for the simple date helpers, no test-local context is needed).

### D2. Two flavors of anchor helper

The original `a14` work used a single `next_tuesday_anchor()` because every test in `recurring_event_test.rs` used a Tuesday anchor. Here, the tests split:

- Single-event tests use `2026-08-01` (Saturday). They don't care about the weekday — they just need a valid future start_time. So `next_saturday_anchor()` matches the original semantics (any weekday in the near future).
- Recurring-Tuesday tests use `2026-08-04` (Tuesday). They need a Tuesday because the rule is `WeeklyByDay { weekdays: [Tue] }`. So `next_tuesday_anchor()` is required.

Could we just use `next_tuesday_anchor()` for both flavors? Technically yes — the single-event tests don't depend on the weekday. But preserving the original "Sat for single, Tue for recurring" distinction keeps the diff minimal and avoids subtly changing the test semantics.

### D3. `weeks_after(anchor, weeks)` takes the anchor explicitly

The `a14` version was `weeks_after_anchor(weeks)` — implicitly relative to the single Tuesday anchor used everywhere in that file. Here, with two anchors in play, an implicit version would be ambiguous. The explicit-anchor version makes the relationship clear at the call site:

```rust
let start = next_tuesday_anchor();
let until = weeks_after(start, 8);
```

Reads cleanly. Avoids the bug class where a contributor picks the wrong implicit anchor.

### D4. Offset of 8 weeks matches the original ~8.5-week spread

Original: anchor=`2026-08-04`, until=`2026-10-01`. That's 58 days = 8 weeks + 2 days. Round to 8 weeks for the helper. The recurring tests assert things like "the materializer produces 8 occurrences" — that's the count of Tuesdays in an 8-week window starting on a Tuesday (week 0 Tuesday + 7 more weekly Tuesdays = 8 occurrences total inclusive). Same count holds with the helper's 8-week offset.

If any test happens to assert exactly 8 or 9 occurrences and the count slips by 1 with the helper's offset, widen the assertion to a range — same fix shape as `a14`'s `(50..=53)` tolerance.

### D5. Spec delta is a clarification, not a new rule

The `a14` spec language ("tests asserting on the recurring-event materializer SHALL use runtime-relative anchors") already covers any test, including inline `#[cfg(test)]` tests in `src/`. This change adds a clarifying scenario that makes the inclusion explicit — useful for future readers who might assume the rule only applies to standalone test files.

## Risks / Trade-offs

- **Risk**: an assertion turns out to be 1-off-from-expected because the relative anchor lands in a slightly different calendar week than `2026-08-04` did. → **Mitigation**: widen the range with the `(N-1..=N+1).contains(&count)` tolerance, same as `a14` did with `(50..=53)`. Trivial fix during implementation.
- **Risk**: the helper extraction tempts a future contributor to also lift them to `src/service/test_helpers.rs` for "reuse," which then forces every test module that imports them to declare a `#[cfg(test)] use` chain. → **Mitigation**: design's non-goal explicitly says don't extract until there's a third caller. Documented.
- **Trade-off**: helpers are duplicated across `tests/recurring_event_test.rs` and `src/service/event_admin_service.rs`. ~25 lines of duplicate code. Acceptable for now; extract on the third caller.

## Migration Plan

Single PR.

1. Add the three helper functions (`next_saturday_anchor`, `next_tuesday_anchor`, `weeks_after`) inside the existing `#[cfg(test)] mod tests` block in `src/service/event_admin_service.rs`, near the top of the module (alongside `fresh_pool`, `make_service`).
2. Replace each `Utc.with_ymd_and_hms(2026, 8, 1, 18, 0, 0).unwrap()` with `next_saturday_anchor()`. Confirm by grep that all 4 occurrences are addressed.
3. Replace each `Utc.with_ymd_and_hms(2026, 8, 4, 18, 0, 0).unwrap()` with `next_tuesday_anchor()`. Confirm all 5 occurrences.
4. Replace each `Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()` with `weeks_after(start, 8)` (where `start` is the local anchor variable in each test). Confirm all 3 occurrences.
5. Run `cargo test --features test-utils --lib event_admin_service`. If any assertion is off by 1 from the expected occurrence count, widen the range per D4.
6. Run the full suite: `cargo test --features test-utils`.
7. Grep verify: `grep -n "with_ymd_and_hms(202" src/service/event_admin_service.rs` returns nothing.
