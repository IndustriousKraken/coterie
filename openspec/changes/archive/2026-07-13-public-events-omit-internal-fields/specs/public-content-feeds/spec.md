# public-content-feeds Specification

## MODIFIED Requirements

### Requirement: Members-only events appear in /public/events with sanitized fields

`GET /public/events` SHALL combine `event_repo.list_public()` AND `event_repo.list_members_only()` into a single response, serialized as a PUBLIC PROJECTION — NOT the raw `Event` struct. The projection SHALL expose only fields the public marketing surface needs: `id`, `title`, `description`, `event_type`, `visibility`, `start_time`, `end_time`, `timezone`, `location`, `image_url`, `max_attendees`, `rsvp_required`. Internal fields SHALL NOT be exposed to anonymous callers — in particular `created_by` (the organizer's member id), `created_at`, `updated_at`, `event_type_id`, `series_id`, and `occurrence_index` SHALL be omitted for every event, public or members-only.

Members-only events SHALL additionally be sanitized so that no private data leaks:

- `title` SHALL be replaced with `"Members-Only Event"`.
- `description` SHALL be replaced with `"This event is for members only. Log in to the portal to see details."`.
- `location` SHALL be set to `None`.
- `image_url` SHALL be set to `None`.

The real `start_time`/`end_time` pass through for both public and members-only events. The result SHALL be filtered to upcoming events (`start_time > now()`), sorted ascending by start time, and truncated to the configured `limit` (default 50).

#### Scenario: Members-only event title is sanitized

- **WHEN** a members-only event "Annual Members Dinner" is in the database
- **THEN** `/public/events` SHALL include an entry whose `title = "Members-Only Event"` and whose location/image_url are null; the start/end times SHALL be the real values

#### Scenario: Past events are excluded

- **WHEN** an event's `start_time` is in the past
- **THEN** the event SHALL NOT appear in `/public/events` regardless of public/members-only

#### Scenario: Internal identifiers are not exposed to anonymous callers

- **WHEN** an anonymous caller fetches `/public/events` (public or members-only events present)
- **THEN** no entry SHALL carry `created_by`, `created_at`, `updated_at`, `event_type_id`, `series_id`, or `occurrence_index`
