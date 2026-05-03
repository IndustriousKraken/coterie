## ADDED Requirements

### Requirement: Public reads are unauthenticated, CORS-allowed, GET-only

The public read endpoints SHALL be:

- `GET /public/events` — JSON list, or iCal when `?format=ical` is passed.
- `GET /public/events/private-count` — count of currently-private events (for "N members-only events" displays).
- `GET /public/announcements` — JSON list of published public announcements.
- `GET /public/announcements/private-count` — count of private announcements.
- `GET /public/feed/rss` — RSS 2.0 feed.
- `GET /public/feed/calendar` — iCal feed.

These endpoints SHALL be GET-only and therefore not subject to CSRF. They SHALL be reachable cross-origin via the configured CORS allowlist and SHALL NOT require a session.

#### Scenario: Allowed origin can fetch /public/events from a browser

- **WHEN** a browser on an allowed origin issues `fetch('/public/events')`
- **THEN** the response SHALL include the appropriate `Access-Control-Allow-Origin` header for that origin

### Requirement: Members-only events appear in /public/events with sanitized fields

`GET /public/events` SHALL combine `event_repo.list_public()` AND `event_repo.list_members_only()` into a single response. Members-only events SHALL be sanitized so that no private data leaks:

- `title` SHALL be replaced with `"Members-Only Event"`.
- `description` SHALL be replaced with `"This event is for members only. Log in to the portal to see details."`.
- `location` SHALL be set to `None`.
- `image_url` SHALL be set to `None`.

Other fields (start_time, end_time, id) SHALL pass through. The result SHALL be filtered to upcoming events (`start_time > now()`), sorted ascending by start time, and truncated to the configured `limit` (default 50).

#### Scenario: Members-only event title is sanitized

- **WHEN** a members-only event "Annual Members Dinner" is in the database
- **THEN** `/public/events` SHALL include an entry whose `title = "Members-Only Event"` and whose location/image_url are null; the start/end times SHALL be the real values

#### Scenario: Past events are excluded

- **WHEN** an event's `start_time` is in the past
- **THEN** the event SHALL NOT appear in `/public/events` regardless of public/members-only

### Requirement: iCal format via query param

`GET /public/events?format=ical` SHALL return the same upcoming-events list as `text/calendar; charset=utf-8` content type, with members-only events sanitized identically to the JSON response.

#### Scenario: format=ical returns text/calendar

- **WHEN** `/public/events?format=ical` is requested
- **THEN** the response Content-Type SHALL be `text/calendar; charset=utf-8` and the body SHALL be a valid VEVENT-bearing iCal document

### Requirement: /public/feed/calendar is the dedicated iCal endpoint

`GET /public/feed/calendar` SHALL return an iCal feed of events. The endpoint SHALL exist alongside `/public/events?format=ical`; both serve iCal but the dedicated route SHALL be the documented "subscribe to calendar" URL.

#### Scenario: Dedicated calendar endpoint serves iCal

- **WHEN** `/public/feed/calendar` is fetched
- **THEN** the response Content-Type SHALL be `text/calendar` and the body SHALL be a valid iCal document with sanitized members-only events

### Requirement: /public/announcements returns published public announcements only

`GET /public/announcements` SHALL return only announcements that are public-flagged (via `list_public()`) AND have a non-NULL `published_at`. Drafts SHALL NOT appear.

#### Scenario: Draft announcement is excluded

- **WHEN** an admin saves an announcement as draft (no `published_at`)
- **THEN** the announcement SHALL NOT appear in `/public/announcements` even if public-flagged

### Requirement: /public/feed/rss returns public announcements

`GET /public/feed/rss` SHALL return an RSS 2.0 feed (`application/rss+xml; charset=utf-8`) of public-flagged announcements.

#### Scenario: RSS feed contains only public announcements

- **WHEN** `/public/feed/rss` is fetched
- **THEN** members-only announcements SHALL NOT appear in the feed

### Requirement: All /public/* endpoints documented in OpenAPI spec

Every `/public/*` endpoint SHALL be registered in `src/api/docs.rs` so the OpenAPI spec at `/api/docs/openapi.json` matches the implemented surface. Adding a `/public/*` endpoint without a `#[utoipa::path]` annotation AND a docs.rs registration SHALL be treated as incomplete.

#### Scenario: New /public/* endpoint must update docs.rs

- **WHEN** a new public endpoint is added
- **THEN** the change SHALL include a `#[utoipa::path]` annotation on the handler AND a corresponding registration in `src/api/docs.rs`
