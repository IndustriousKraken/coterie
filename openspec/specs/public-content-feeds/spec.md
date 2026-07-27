# public-content-feeds Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
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

`GET /public/events` SHALL combine `event_repo.list_public()` AND `event_repo.list_members_only()` into a single response, serialized as a PUBLIC PROJECTION — NOT the raw `Event` struct. The projection SHALL expose only fields the public marketing surface needs: `id`, `title`, `description`, `event_type`, `visibility`, `start_time`, `end_time`, `timezone`, `location`, `image_url`, `max_attendees`, `rsvp_required`, `registration_url`, `guest_price_cents`. Internal fields SHALL NOT be exposed to anonymous callers — in particular `created_by` (the organizer's member id), `created_at`, `updated_at`, `event_type_id`, `series_id`, and `occurrence_index` SHALL be omitted for every event, public or members-only.

`registration_url` and `guest_price_cents` SHALL both be non-null exactly when the event is publicly registerable (`visibility = Public` AND `guest_registration_enabled`), and SHALL both be null otherwise. They SHALL be populated together or not at all. The price SHALL NOT be part of the registerability test: a free event that requires registration is registerable and SHALL carry a `registration_url` with a `guest_price_cents` of `0`.

`registration_url` SHALL be the absolute URL of the Coterie-hosted public registration page for that event. Emitting a resolved URL — rather than the ingredients a caller would need to decide registerability and construct a link — is deliberate: whether an event may be publicly registered is a server-side authorization rule, and a consumer that re-derived it from prices and visibility flags would duplicate that rule and drift from it. A consumer SHALL be able to decide whether to offer registration by testing `registration_url` for presence and nothing more, and SHALL NOT infer registerability from price, `rsvp_required`, or `visibility`.

Most events SHALL have a null `registration_url`: the ordinary recurring talk or open night that anyone may simply attend has no guest registration enabled, and a consumer SHALL render no registration affordance for it. This is the common case, not the exception, and a consumer SHOULD treat the presence of a registration URL as the unusual condition worth surfacing.

Members-only events SHALL additionally be sanitized so that no private data leaks:

- `title` SHALL be replaced with `"Members-Only Event"`.
- `description` SHALL be replaced with `"This event is for members only. Log in to the portal to see details."`.
- `location` SHALL be set to `None`.
- `image_url` SHALL be set to `None`.

The real `start_time`/`end_time` pass through for both public and members-only events. **By default** — and always for the iCal format — the result SHALL be filtered to upcoming events (derived UTC instant after `now()`), sorted ascending by start time, and truncated to the configured `limit` (default 50). **When both `from` and `to` query parameters are supplied as valid RFC 3339 instants**, spanning no more than a bounded maximum window, the JSON result SHALL instead include every event whose derived UTC instant falls within `[from, to)` — **including past events** — sorted ascending by start time, still projected, members-only sanitized, AdminOnly excluded, and `limit`-bounded. A missing, malformed, or over-wide range SHALL fall back to the default upcoming-only behavior rather than erroring.

#### Scenario: Members-only event title is sanitized

- **WHEN** a members-only event "Annual Members Dinner" is in the database
- **THEN** `/public/events` SHALL include an entry whose `title = "Members-Only Event"` and whose location/image_url are null; the start/end times SHALL be the real values

#### Scenario: Past events are excluded by default

- **WHEN** an event's derived instant is in the past AND no `from`/`to` range is supplied
- **THEN** the event SHALL NOT appear in `/public/events` regardless of public/members-only

#### Scenario: A date range returns past events

- **WHEN** an anonymous caller fetches `/public/events?from=<start>&to=<end>` (valid RFC 3339, within the maximum span) and a past event's derived instant falls within `[from, to)`
- **THEN** that past event SHALL appear in the JSON response, projected and — if members-only — sanitized like any other

#### Scenario: A malformed or over-wide range falls back to upcoming-only

- **WHEN** `from`/`to` are missing one side, unparseable, or span more than the maximum window
- **THEN** the response SHALL be the default upcoming-only list (the range is ignored), never an error that breaks the marketing calendar

#### Scenario: Internal identifiers are not exposed to anonymous callers

- **WHEN** an anonymous caller fetches `/public/events` (public or members-only events present)
- **THEN** no entry SHALL carry `created_by`, `created_at`, `updated_at`, `event_type_id`, `series_id`, or `occurrence_index`

#### Scenario: An ordinary show-up event carries no registration URL

- **WHEN** an anonymous caller fetches `/public/events` and an event has guest registration disabled (the common case — a recurring talk or open night anyone may attend)
- **THEN** that entry's `registration_url` and `guest_price_cents` SHALL both be null

#### Scenario: A paid registerable event carries a resolved URL and its price

- **WHEN** an event is `Public`, has guest registration enabled, and has a guest price above zero
- **THEN** that entry SHALL carry a non-null absolute `registration_url` pointing at its Coterie-hosted registration page, and a `guest_price_cents` equal to that price

#### Scenario: A free registration-required event also carries a registration URL

- **WHEN** an event is `Public` with guest registration enabled and a guest price of `0` — a free workshop with limited seats
- **THEN** that entry SHALL carry a non-null `registration_url` and a `guest_price_cents` of `0`; a zero price SHALL NOT suppress the registration URL

#### Scenario: A members-only event never advertises registration

- **WHEN** a members-only event is projected into `/public/events`
- **THEN** its `registration_url` and `guest_price_cents` SHALL be null alongside the other sanitized fields

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

