# Tasks

## 1. Store publish time as a local wall-clock + zone

- [ ] 1.1 Migration: add a `scheduled_publish_timezone` TEXT column to
  `announcements`, defaulted to the current `org.timezone`. Backfill existing
  scheduled rows to the current org zone — a pure annotation, DO NOT shift
  any stored `scheduled_publish_at` value.
- [ ] 1.2 Carry the zone on the announcement domain/service and add a
  derivation that resolves the true UTC instant from
  (`scheduled_publish_at` wall-clock, zone) using the shared
  `wall_clock_to_utc` (reused from the event change), with the same DST
  gap/overlap handling.
- [ ] 1.3 On create/edit, store the naive `datetime-local` input as-is and
  freeze the zone from `org.timezone`.

## 2. Runner compares the derived instant

- [ ] 2.1 In the scheduled-publish query (`announcement_repository.rs:304`)
  and `AnnouncementAdminService::publish_scheduled()`, widen the SQL bound by
  the widest IANA offset (~15h) as a coarse pre-filter, then do the exact
  `derived_utc <= now` test in Rust before flipping each Draft. Keep the
  atomic conditional UPDATE, the `actor_id = None` audit, and the
  `AnnouncementPublished` dispatch unchanged.

## 3. Admin rendering

- [ ] 3.1 Render the scheduled time in the org zone with its abbreviation
  (e.g. "9:00 AM EDT") on the admin list/detail
  (`src/web/portal/admin/announcements.rs:352`), replacing the "UTC" label.

## 4. Tests (offline)

- [ ] 4.1 Non-UTC org: a Draft scheduled for a local time whose true instant
  is still in the future does NOT publish early; it fires on the true instant.
- [ ] 4.2 The annotation migration shifts no stored value (rendered local
  time identical before/after).
