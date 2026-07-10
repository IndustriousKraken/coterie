# Tasks

## 1. Org timezone setting

- [x] 1.1 Add an IANA timezone dependency (e.g. `chrono-tz`).
- [x] 1.2 In `src/service/settings_service.rs`, add an `org.timezone`
  constant (category `organization`, default `UTC`). Validate on write that
  the value parses as an IANA zone; reject unknown names.
- [x] 1.3 Surface it on the settings page (a dropdown of common zones beats a
  free-text field). Add a `settings.org_timezone() -> Tz` helper that falls
  back to `UTC`.

## 2. Local + zone storage model

- [x] 2.1 Migration: add a `timezone` TEXT column to `events` (and the series
  template table), defaulted to the current `org.timezone`. Backfill existing
  rows to the current org zone — this is a pure annotation; DO NOT shift any
  stored time value.
- [x] 2.2 Change the domain `Event` representation so its time is understood
  as a naive local wall-clock plus an IANA zone, not a `DateTime<Utc>`.
  Provide a derivation, e.g. `start_utc()` / `end_utc()`, that resolves the
  UTC instant from (local, zone) using the current tz database, handling the
  DST gap/overlap cases with a defined rule rather than a panic.

## 3. Admin I/O is the stored wall-clock

- [x] 3.1 In `src/web/portal/admin/events/single.rs`, store the form's naive
  input as-is together with the event zone (defaulted from `org.timezone`);
  no conversion on save.
- [x] 3.2 Pre-fill the edit form and render admin lists/detail from the stored
  wall-clock directly (optionally annotate the zone abbreviation, e.g.
  "7:00 PM EDT").

## 4. Derive UTC at read time (public + iCal)

- [x] 4.1 In `src/api/handlers/public.rs`, serialize `/public/events`
  `start_time`/`end_time` and the iCal `DTSTART`/`DTEND` from `start_utc()` /
  `end_utc()` (still `…Z`). Keep the format; change only the source to the
  derived instant.
- [x] 4.2 Update the "upcoming" filter and the sort (currently
  `e.start_time > now` / `sort_by start_time`, `public.rs:288,292`) to compare
  derived UTC instants.

## 5. Wall-clock recurrence

- [x] 5.1 In `compute_occurrence_start_time`
  (`src/service/event_admin_service.rs`) and
  `src/service/recurring_event_service.rs`, advance occurrences on the
  wall-clock in the event's zone and persist each as a local wall-clock (not
  a frozen UTC instant), so the series keeps its local time across DST and
  rule changes.
- [x] 5.2 Test: a weekly evening series spanning a DST boundary keeps a
  constant local time; its derived UTC instants differ by an hour across the
  boundary.

## 6. Optional derived-UTC index (only if needed for scale)

- [x] 6.1 If query performance needs it, add a denormalized `start_utc`
  column populated from (local, zone) on write, used only for indexing/sort.
  It is derived and refreshable (recompute after a tz database update); it is
  never authoritative. Skip unless a measured need appears.
  (Intentionally skipped: no measured perf need. `start_utc()` derives
  on read; series/list volumes are small. Add later if sort/filter is hot.)

## 7. Tests

- [x] 7.1 Round-trip: with `org.timezone=America/New_York`, an event entered
  at 7 PM stores local `19:00` + zone, the admin form re-renders `19:00`, and
  `/public/events` derives `23:00:00Z` in July.
- [x] 7.2 Rule-change resilience: given a stored local `19:00` + zone, the
  derived UTC follows the tz database — i.e. the wall-clock is preserved and
  the instant is recomputed, never frozen.
- [x] 7.3 The annotation migration shifts no time values (a row's rendered
  local time is identical before and after).
- [x] 7.4 Unknown IANA name is rejected by the setting validator.
