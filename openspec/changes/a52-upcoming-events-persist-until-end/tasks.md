# Tasks

## 1. The predicate

- [ ] 1.1 `src/domain/event.rs`: add a named constant for the grace period —
  two hours — with a comment recording why it exists (a missing `end_time` means
  the end is unknown, not zero-length) and why it is a constant rather than a
  setting (the field that answers "how long does this run" is `end_time`).
- [ ] 1.2 Add the predicate as a free function over **instants**, not over
  `&Event`: `is_upcoming(start_utc, end_utc: Option<..>, now) -> bool`, returning
  `end_utc.unwrap_or(start_utc + GRACE) > now`. Take `now` as a parameter rather
  than reading the clock inside — every caller already has one, and a
  clock-reading predicate is untestable at a boundary.
- [ ] 1.3 Add a thin `Event::is_upcoming(&self, now)` that passes
  `self.start_utc()` / `self.end_utc()` into it, for callers holding an event
  whose fields still hold the stored wall-clock. This is a convenience wrapper
  only — the derivation must not be duplicated inside it.
- [ ] 1.4 Do not add a `has_ended` twin. One predicate, negated where needed.

## 2. Call sites

- [ ] 2.1 `src/repository/event_repository.rs::list_upcoming` — replace
  `events.retain(|e| e.start_utc() > now)` with the predicate. The SQL
  pre-filter's 15-hour margin was sized for the timezone offset alone; widen it to
  also cover the grace period so an event whose row falls outside the coarse
  bound cannot be dropped before the exact test runs. Keep the sort on
  `start_utc()` — an in-progress event belongs at the head of the list.
- [ ] 2.2 `src/repository/event_repository.rs::count_members_only_upcoming` —
  same predicate, same widened margin, so the teaser count matches the list it
  teases.
- [ ] 2.3 `src/api/handlers/public/mod.rs` — the `None => events.retain(|e|
  e.start_time > now)` branch. **Call the instant-level function here, passing
  `e.start_time` and `e.end_time` directly. Do NOT call `Event::is_upcoming`.**
  `derive_utc_instants` runs immediately above (line ~209) and has already
  overwritten both fields with the derived instant; the `Event` wrapper would
  apply `wall_clock_to_utc` to an instant that is already UTC, shifting it a
  second time and putting the public feed and the iCal output off by the org's
  offset — four or five hours for `America/New_York`. This is the one call site
  where the convenient method is the wrong one. The `Some((from, to))` range
  branch is unchanged, and already compares derived values for the same reason.
- [ ] 2.4 Leave `src/service/series_enrollment_service.rs:141` start-based. It
  decides whether a class can still be bought; letting someone buy a pass during
  the final session is a pricing decision, not a listing one, and is out of scope.
- [ ] 2.5 Leave `src/web/templates/class_register.rs:70` start-based for the same
  reason — it counts sessions a buyer would still receive.
- [ ] 2.6 `src/web/portal/admin/events/single.rs:142-146` is the admin event
  list's time-filter dropdown, and it has TWO arms: `"upcoming" => e.start_utc() >
  now` and `"past" => e.start_utc() <= now`. Move both together, keeping them
  exact complements — `"past"` becomes `!is_upcoming(now)`. Moving only the
  `"upcoming"` arm would list an in-progress event under BOTH filters, which is a
  worse bug than the one being fixed.
- [ ] 2.7 Do NOT touch the occurrence cancel/override controls on the series
  detail page. `admin-events` canon fixes those at `start_time < now` — "exceptions
  only apply to the future" — and that is a different question from what a listing
  shows. Cancelling an occurrence that is currently happening is not something
  this change decides.
- [ ] 2.8 Reminder scheduling is untouched. `event-reminders` canon states a past
  event is not reminded on its **start** time, which stays true. Confirm by
  inspection that `list_pending_reminders` does not route through any of the
  above, and say so here rather than leaving it implied.

## 3. Tests

- [ ] 3.1 `is_upcoming` unit tests at the boundaries: one second before the end
  instant, one second after; with and without `end_time`; and for an event in a
  non-UTC zone where a naive wall-clock comparison would give the wrong answer.
  Derive the instants from the event under test rather than hardcoding absolute
  dates — a calendar-relative bound is what keeps these from becoming
  day-of-week flakes.
- [ ] 3.2 `list_upcoming` includes an event that started an hour ago and ends an
  hour from now, and excludes one that ended a minute ago.
- [ ] 3.3 `list_upcoming` sorts the in-progress event ahead of one starting later
  today.
- [ ] 3.4 An event with no `end_time` that started 30 minutes ago is included;
  one that started 3 hours ago is not.
- [ ] 3.5 `count_members_only_upcoming` returns the same count as the number of
  members-only entries `list_upcoming` yields for the same fixture, including
  while one of them is in progress. This is the assertion that stops the two
  implementations drifting again.
- [ ] 3.6 `/public/events` with no range includes an in-progress event; the same
  request with `format=ical` includes its VEVENT. Use a **non-UTC** org zone for
  this fixture. A UTC fixture cannot detect the double-conversion in 2.3, because
  the offset it would add is zero — the bug would ship green.
- [ ] 3.7 Double-conversion guard: for an event in a non-UTC zone, assert the
  instant-level predicate applied to post-`derive_utc_instants` values agrees with
  `Event::is_upcoming` applied to the same event before derivation.
- [ ] 3.8 `/public/events` still excludes an event that has ended, and the
  `from`/`to` range still returns it.
- [ ] 3.9 The admin list's `upcoming` and `past` filters partition the fixture:
  every event appears under exactly one of them, including one in progress.
- [ ] 3.10 Assert no call site outside the domain compares a start instant to
  `now` to decide upcoming-ness — a grep-style assertion over `src/`, allowing the
  deliberate exceptions in 2.4, 2.5, and 2.7. The defect class here is a fourth
  copy of the rule, not the three that exist.
