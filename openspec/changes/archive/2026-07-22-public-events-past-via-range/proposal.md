# public-events-past-via-range

## Why

The marketing calendar can't show past events: `/public/events` is canonically
upcoming-only (`public-content-feeds` → "Members-only events appear in
/public/events with sanitized fields": *"filtered to upcoming events"*, with a
"Past events are excluded" scenario). Members reasonably want the calendar to
show what happened on past days, not blank them out.

We keep the default upcoming-only (the marketing **home page** "Upcoming Events"
section and iCal subscriptions rely on it) and add an **opt-in date range**: when
the caller supplies `from` and `to`, the JSON feed returns events in that window,
**including past events**. The marketing calendar uses this to fetch the month it
is displaying, so navigating back a month loads that month's past events. This is
a behavior change to a canonical requirement, so it's a **change**.

## What Changes

- `GET /public/events` accepts optional `from` and `to` RFC 3339 instants.
  - **No range (default), and always for `format=ical`:** unchanged — upcoming
    only, sorted ascending, `limit`-bounded. The home page and calendar
    subscriptions are unaffected.
  - **Both `from` and `to` supplied and valid,** spanning no more than a bounded
    maximum window: the JSON result includes every event whose derived instant
    is in `[from, to)` — past events included — still projected, members-only
    sanitized, AdminOnly excluded, sorted ascending, `limit`-bounded.
  - A missing/malformed/over-wide range falls back to the default upcoming-only
    behavior (never an error — a bad range must not break the marketing site).
- Filtering compares the **derived UTC instant** (`start_utc`), not the naive
  wall-clock, consistent with the timezone model.

## Impact

- **Spec:** `public-content-feeds` — 1 MODIFIED requirement ("Members-only events
  appear in /public/events with sanitized fields"): the filtering paragraph gains
  the range opt-in; the "Past events are excluded" scenario becomes "…by default";
  two scenarios added (range returns past; bad range falls back). The iCal
  requirements are unchanged (iCal stays upcoming-only).
- **Code:** `src/api/handlers/public.rs` — `PublicEventsQuery` gains `from`/`to`;
  `list_events` branches on a valid bounded range vs the default upcoming filter.
  Update the `#[utoipa::path]`/`docs.rs` schema for the new params.
- **Tests:** range returns a past event (projected/sanitized); no-range still
  excludes past; malformed/over-wide range falls back to upcoming; `format=ical`
  ignores the range.
- **Companion:** the marketing repo's `calendar.js` fetches the displayed month's
  range and refetches on navigation (separate change,
  `calendar-shows-past-by-month`); the home page keeps requesting upcoming-only.
- **Bound:** maximum range span (implementation: ~400 days) prevents an unbounded
  scan from an anonymous endpoint.
