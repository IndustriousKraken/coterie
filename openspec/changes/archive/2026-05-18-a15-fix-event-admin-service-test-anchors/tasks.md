## 1. Add helpers to the test module

- [x] 1.1 Inside `src/service/event_admin_service.rs`'s existing `#[cfg(test)] mod tests` block, near the top alongside `fresh_pool` / `make_service`, add the three helper functions:
  ```rust
  /// Next Saturday at 18:00 UTC strictly after `now + 1 day`. Used as
  /// the start time for single-event tests that don't care about the
  /// weekday — just need a valid future timestamp.
  fn next_saturday_anchor() -> DateTime<Utc> {
      let now = Utc::now();
      let start = now + chrono::Duration::days(1);
      let days_until_sat = (chrono::Weekday::Sat.num_days_from_monday() as i64
          - start.weekday().num_days_from_monday() as i64).rem_euclid(7);
      let date = start.date_naive() + chrono::Duration::days(days_until_sat);
      date.and_hms_opt(18, 0, 0).unwrap().and_utc()
  }

  /// Next Tuesday at 18:00 UTC strictly after `now + 1 day`. Used as
  /// the start time for recurring-Tuesday tests where the weekly rule
  /// requires the anchor BE a Tuesday.
  fn next_tuesday_anchor() -> DateTime<Utc> {
      let now = Utc::now();
      let start = now + chrono::Duration::days(1);
      let days_until_tue = (chrono::Weekday::Tue.num_days_from_monday() as i64
          - start.weekday().num_days_from_monday() as i64).rem_euclid(7);
      let date = start.date_naive() + chrono::Duration::days(days_until_tue);
      date.and_hms_opt(18, 0, 0).unwrap().and_utc()
  }

  /// `anchor` shifted forward by `weeks` whole weeks. Use to compute
  /// `until_date` values relative to the test's local anchor.
  fn weeks_after(anchor: DateTime<Utc>, weeks: i64) -> DateTime<Utc> {
      anchor + chrono::Duration::weeks(weeks)
  }
  ```
  Note: import any missing `use chrono::Weekday;` at the module level if not already in scope.

## 2. Replace single-event anchors (Saturday flavor)

- [x] 2.1 Find and replace all 4 occurrences of `Utc.with_ymd_and_hms(2026, 8, 1, 18, 0, 0).unwrap()` with `next_saturday_anchor()`. Tests touched: `create_single_event_emits_full_chain`, `create_admin_only_event_skips_integration_dispatch`, `update_one_writes_audit`, `delete_one_writes_audit`. Verify count via `grep -c "2026, 8, 1," src/service/event_admin_service.rs` returns 0 after the sweep.

## 3. Replace recurring-Tuesday anchors

- [x] 3.1 Find and replace all 5 occurrences of `Utc.with_ymd_and_hms(2026, 8, 4, 18, 0, 0).unwrap()` with `next_tuesday_anchor()`. Tests touched: `create_recurring_series_materializes_and_audits`, `update_series_from_audits_with_count`, `end_series_caps_and_audits`, `delete_series_cascades_and_audits`, plus one more — confirm via grep that all are addressed.

## 4. Replace until-date values

- [x] 4.1 Find and replace all 3 occurrences of `Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()` with `weeks_after(start, 8)`, where `start` is the local anchor variable in each test (e.g., `let start = next_tuesday_anchor(); let until = weeks_after(start, 8);`).
- [x] 4.2 If any test uses a different variable name for the anchor (`anchor` instead of `start`, or similar), use that name in the `weeks_after(...)` call. Match the local convention.

## 5. Verify

- [x] 5.1 `cargo test --features test-utils --lib event_admin_service` — every test in the module passes.
- [x] 5.2 If any test fails on an exact occurrence-count assertion that's now off by 1, widen the assertion to a tolerance range (e.g., `(7..=9).contains(&count)` for an expected-8 case). Same shape as `a14`'s `(50..=53)` fix. This is the same risk noted in `a14` — calendar alignment occasionally shifts the count by 1.
- [x] 5.3 `cargo test --features test-utils` — full suite passes.
- [x] 5.4 Grep verify: `grep -n "with_ymd_and_hms(2026" src/service/event_admin_service.rs` returns nothing.

## 6. Confirm scope discipline

- [x] 6.1 The helpers stay inside `src/service/event_admin_service.rs`'s test module. No shared helper module is created.
- [x] 6.2 No other file is touched. Specifically: `tests/recurring_event_test.rs` (already fixed by `a14`), `tests/member_template_snapshots.rs` (formatting fixtures, not drift candidates per autocoder report), and `src/web/templates/filters.rs` (same — formatting fixtures) are all unchanged.
