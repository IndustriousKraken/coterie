# public-content-feeds Specification

## MODIFIED Requirements

### Requirement: Members-only events appear in /public/events with sanitized fields

`GET /public/events` SHALL combine `event_repo.list_public()` AND `event_repo.list_members_only()` into a single response, serialized as a PUBLIC PROJECTION — NOT the raw `Event` struct. The projection SHALL expose only fields the public marketing surface needs: `id`, `title`, `description`, `event_type`, `visibility`, `start_time`, `end_time`, `timezone`, `location`, `image_url`, `max_attendees`, `rsvp_required`. Internal fields SHALL NOT be exposed to anonymous callers — in particular `created_by` (the organizer's member id), `created_at`, `updated_at`, `event_type_id`, `series_id`, and `occurrence_index` SHALL be omitted for every event, public or members-only.

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
