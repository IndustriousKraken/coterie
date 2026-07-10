# announcement-publish-timezone

## Why

Scheduled announcements publish at the wrong time — the same
naive-time-mislabeled-as-UTC bug that `event-timezone-correctness` fixed for
events, now in the announcement scheduled-publish path.

An admin schedules an announcement for **9:00 AM** via a `datetime-local`
input (naive local, no zone). It is stored as `09:00` in
`scheduled_publish_at` (a `NaiveDateTime`,
`src/repository/announcement_repository.rs:41`) and the runner compares it
directly against UTC `now` (`... AND scheduled_publish_at <= ?`,
`announcement_repository.rs:304`). For an `America/New_York` org, `09:00`
is treated as `09:00Z`, so the announcement publishes at **5:00 AM Eastern**
— four hours early. The admin surface also labels the scheduled time "UTC"
(`src/web/portal/admin/announcements.rs:352`), which is wrong now that the
value is a local wall-clock.

This was called out as an explicit follow-up during the
`event-timezone-correctness` review (it was out of scope there, which
targeted events).

## What Changes

- Reuse the **`org.timezone`** setting added by `event-timezone-correctness`.
- Treat `scheduled_publish_at` as a **local wall-clock plus a frozen IANA
  zone** (a `scheduled_publish_timezone` column defaulted from `org.timezone`
  at scheduling), mirroring the event model — so a later rule change does not
  move the intended publish time.
- The runner derives the **true UTC instant** from (wall-clock, zone) and
  compares *that* to `now`, so the announcement publishes at the intended
  local time. The SQL stays a coarse pre-filter widened by the widest IANA
  offset; the exact comparison happens in Rust (same pattern as
  `list_pending_reminders` / `list_upcoming`), because SQLite has no tz math.
- The admin surface renders the scheduled time in the org zone with its
  abbreviation (e.g. "9:00 AM EDT"), not a mislabeled "UTC".

## Impact

- **Spec:** `scheduled-announcement-publish` — MODIFIED the "future publish
  time" and "background runner publishes at their time" requirements to
  specify org-local semantics with UTC derived at compare time. And
  `admin-announcements` — MODIFIED "Admin announcement form accepts optional
  scheduled publish time" to interpret the `datetime-local` field as an
  org-timezone wall-clock (dropping the "treat as UTC for v1" clause), since
  the form-input parsing is pinned there too.
- **Code:** a `scheduled_publish_timezone` column on `announcements`
  (annotation migration, defaulted from `org.timezone`, no value shift);
  the announcement domain/service to carry the zone and derive the instant;
  `publish_scheduled()` / the repo query to widen + filter on the derived
  UTC; the admin form/list rendering to show the org zone abbr.
- **Testing:** offline; a non-UTC-org test that a Draft scheduled for a local
  time fires on its true instant (not the org-offset-early instant), mirroring
  the event reminder/list_upcoming tests.
- **Reuses** the `wall_clock_to_utc` derivation and `org.timezone` setting
  already shipped for events; no new dependency.
