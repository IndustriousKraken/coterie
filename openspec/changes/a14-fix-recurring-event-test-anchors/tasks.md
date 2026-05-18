## 1. Add the test helpers

- [ ] 1.1 Near the top of `tests/recurring_event_test.rs` (under the existing imports, before the `build()` function), add the three helper functions:
  ```rust
  /// Next Tuesday at 18:00 UTC strictly after `now + 1 day`. Used so
  /// test anchors are always in the near future regardless of when the
  /// suite runs.
  fn next_tuesday_anchor() -> DateTime<Utc> {
      let now = Utc::now();
      let start = now + chrono::Duration::days(1);
      let days_until_tue = (Weekday::Tue.num_days_from_monday() as i64
          - start.weekday().num_days_from_monday() as i64).rem_euclid(7);
      let date = start.date_naive() + chrono::Duration::days(days_until_tue);
      date.and_hms_opt(18, 0, 0).unwrap().and_utc()
  }

  /// `next_tuesday_anchor()` shifted forward by `weeks` whole weeks.
  fn weeks_after_anchor(weeks: i64) -> DateTime<Utc> {
      next_tuesday_anchor() + chrono::Duration::weeks(weeks)
  }

  /// The 2nd Wednesday of the calendar month following the current
  /// month, at 19:00 UTC. Used by tests of the MonthlyByWeekdayOrdinal
  /// rule where the anchor must be a "2nd Wednesday."
  fn next_2nd_wednesday_anchor() -> DateTime<Utc> {
      let now = Utc::now().date_naive();
      let next_month_first = if now.month() == 12 {
          chrono::NaiveDate::from_ymd_opt(now.year() + 1, 1, 1).unwrap()
      } else {
          chrono::NaiveDate::from_ymd_opt(now.year(), now.month() + 1, 1).unwrap()
      };
      let days_until_wed = (Weekday::Wed.num_days_from_monday() as i64
          - next_month_first.weekday().num_days_from_monday() as i64).rem_euclid(7);
      let first_wed = next_month_first + chrono::Duration::days(days_until_wed);
      let second_wed = first_wed + chrono::Duration::days(7);
      second_wed.and_hms_opt(19, 0, 0).unwrap().and_utc()
  }
  ```
- [ ] 1.2 Add a small unit test for the helpers themselves (in the same file, under `#[cfg(test)] mod helper_tests`) asserting each returns a future date with the expected weekday/time. Quick sanity check; trivial.

## 2. Replace anchors in each test

For each occurrence of `Utc.with_ymd_and_hms(2026, 5, 5, 18, 0, 0).unwrap()`, replace with `next_tuesday_anchor()`. Verify by line:

- [ ] 2.1 Line 123 — `weekly_creates_about_52_occurrences`
- [ ] 2.2 Line 161 — `until_date_caps_materialization`
- [ ] 2.3 Line 190 — (next test after 161; identify by name when implementing)
- [ ] 2.4 Line 231 — (identify by name)
- [ ] 2.5 Line 265 — (identify by name)
- [ ] 2.6 Line 309 — (identify by name)
- [ ] 2.7 Line 340 — `end_series_after_date_deletes_future_only`
- [ ] 2.8 Line 385 — (identify by name)
- [ ] 2.9 Line 431 — (identify by name)
- [ ] 2.10 Line 407 — replace with `next_2nd_wednesday_anchor()` (different test pattern).
- [ ] 2.11 Any other `with_ymd_and_hms(2026, 5, 5, ...)` matches surfaced by `grep` after the replacements above. The grep before edits identified ~11 matches; verify all are addressed.

## 3. Replace `until_date` values

For each hardcoded `until_date` in the file, replace with `weeks_after_anchor(N)` per the design's mapping table:

- [ ] 3.1 `2026-08-01` → `weeks_after_anchor(13)` (appears twice; check both)
- [ ] 3.2 `2026-06-30` → `weeks_after_anchor(8)` (appears in two tests)
- [ ] 3.3 `2026-07-01` → `weeks_after_anchor(8)` (appears twice)
- [ ] 3.4 `2026-12-01` → `weeks_after_anchor(30)`
- [ ] 3.5 `2026-12-31` → `weeks_after_anchor(34)`
- [ ] 3.6 `2027-05-05` → `weeks_after_anchor(52)`

After this section, no `Utc.with_ymd_and_hms(2026, ...)` or `with_ymd_and_hms(2027, ...)` should remain in `tests/recurring_event_test.rs`.

## 4. Verify

- [ ] 4.1 `cargo test --features test-utils --test recurring_event_test` — all tests in the file pass.
- [ ] 4.2 `cargo test --features test-utils` — full suite passes.
- [ ] 4.3 Grep verify: `grep -n "with_ymd_and_hms" tests/recurring_event_test.rs` returns nothing (the file should have no remaining hardcoded calendar timestamps).

## 5. Surface broader exposure (no fixes here)

- [ ] 5.1 Run `grep -rn "with_ymd_and_hms(202" tests/ src/` (note: searches `2020`-`2029`). Catalog any other test files with the same hardcoded-date pattern in the implementing PR's commit message. Do NOT fix them in this change — scope discipline. A follow-up change can sweep them if the pattern shows the same drift symptom.
