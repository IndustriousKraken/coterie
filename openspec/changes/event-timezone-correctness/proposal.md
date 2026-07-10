# event-timezone-correctness

## Why

Event times are off by the org's UTC offset everywhere except the admin
portal. An admin enters an event at **7:00 PM**; the public marketing site
and any calendar app show **3:00 PM** (a 4-hour shift in US Eastern
daylight time).

Root cause is a naive-time-mislabeled-as-UTC bug:

1. The admin form submits a naive local wall-clock (`datetime-local` gives
   `2026-07-23T19:00`, no zone).
2. The handler parses it straight into `DateTime<Utc>` and the row stores
   `2026-07-23 19:00:00` — i.e. 7 PM is recorded **as if it were UTC**.
3. `/public/events` serializes that as `2026-07-23T19:00:00Z` and the iCal
   feed emits `20260723T190000Z` (`src/api/handlers/public.rs:490`).
4. The browser correctly converts UTC to the viewer's local zone, so 19:00Z
   becomes 15:00 (3 PM) for an Eastern viewer.

Coterie has **no timezone model** and no org-timezone setting. For an org
with **Remote and Streaming members in other timezones** this is a real
correctness problem — those members need the event rendered in *their* local
time.

## Storage model: local wall-clock + IANA zone, NOT a frozen UTC instant

Storing a **UTC instant** for a *future* local event is lossy. The
conversion has to pick an offset at creation time; if a government changes
the rules before the event happens — permanent DST, an offset change, a
DST-date shift (all of which happen on short notice) — the frozen UTC
instant now decodes to the wrong wall-clock, and the organizer's "7 PM at
the venue" silently moves. This is worst for recurring series, which extend
indefinitely into the future where such changes are eventually guaranteed.

Therefore an event's authoritative stored time SHALL be the **local
wall-clock plus the IANA zone name** it was scheduled in. The UTC instant is
a **derived value computed at read time** from that pair using the current
tz database, so a later rule change is absorbed automatically on the next tz
database update. UTC is never the stored source of truth for a future event.

This also means the values already in `events` — naive local wall-clocks —
are *correct as stored*; they were only being *interpreted* wrong. So there
is **no lossy value-shifting backfill**; we annotate the rows with their zone
and fix the read path.

## What Changes

- Add an **org timezone** setting (`org.timezone`, an IANA name such as
  `America/New_York`, default `UTC`) in the `organization` settings
  category, editable from the portal settings page.
- Record each event's time as a **naive local wall-clock plus an IANA zone
  name** (a per-event `timezone` column, defaulted from `org.timezone` at
  creation so an event's zone is frozen at creation and a later org-setting
  change does not reinterpret old events).
- **Derive UTC at read time.** `/public/events` JSON and the iCal feed
  compute the UTC instant from (local, zone) using the current tz database
  and emit it, so every consumer renders the correct local time — a Pacific
  member sees 4 PM — and a future rule change is picked up automatically.
- **Admin I/O is the stored wall-clock** — no conversion, no loss. The form
  shows and accepts 7 PM directly.
- **Recurrence is computed on the wall-clock** (in the event's zone), so a
  "weekly at 7 PM" series stays 7 PM local forever, across every DST
  transition and rule change, because occurrences are stored as local times,
  not frozen UTC.
- **Query paths derive UTC on demand.** "Upcoming" filtering and time
  sorting convert (local, zone) → UTC via the tz database rather than
  comparing the stored naive value. An optional denormalized `start_utc`
  cache MAY back an index, but it is derived and refreshable, never
  authoritative.

## Impact

- **Spec:** new capability `event-timezone` — the setting, the local+zone
  storage model, UTC-derived-at-read, admin wall-clock I/O, wall-clock
  recurrence, and correct public/iCal output.
- **Code:** add an IANA tz dependency (e.g. `chrono-tz`);
  `src/service/settings_service.rs` (`org.timezone`, validated);
  a `timezone` column on `events` (and the series template), defaulted from
  `org.timezone`, backfilled to the current org zone for existing rows (a
  pure annotation, no time shift); the domain `Event` time representation
  (carry the zone; expose a `start_utc()` derivation);
  `src/api/handlers/public.rs` (derive UTC for JSON + iCal);
  `src/service/recurring_event_service.rs` and
  `compute_occurrence_start_time` (wall-clock recurrence); the "upcoming"
  filter/sort in the public and admin queries (derive UTC to compare).
- **Only existing data touch** is annotating rows with the org zone — no
  value shift, so nothing can be double-corrected. Event reminders ("N hours
  before start") become correct for free.
- **Out of scope (follow-up):** announcement scheduled-publish
  (`announcements.publish_at`) has the same latent mislabel and deserves the
  same local+zone treatment in a separate change.
