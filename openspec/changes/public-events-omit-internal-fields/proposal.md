# public-events-omit-internal-fields

## Why

`GET /public/events` sanitizes members-only events by nulling four display
fields (title, description, location, image_url) but serializes the entire
`Event` struct, and canon (`public-content-feeds` → "Members-only events appear
in /public/events with sanitized fields") explicitly says "Other fields
(start_time, end_time, id) SHALL pass through." In practice that passes through
`created_by` — the organizer's internal member UUID — plus `created_at`,
`updated_at`, `event_type_id`, `series_id`, `occurrence_index`, and other
internal fields, to **anonymous** callers, for both members-only AND public
events. `created_by` is an internal identifier that should never reach the
unauthenticated marketing surface. The iCal path already emits only a minimal
projection; the JSON path over-shares.

Because canon mandates the pass-through, tightening it is a spec change, not an
issue.

## What Changes

- `/public/events` (JSON) SHALL return a purpose-built PUBLIC PROJECTION rather
  than the raw `Event` struct — the same fields the marketing site consumes
  (id, title, description, event_type, visibility, start_time, end_time,
  timezone, location, image_url, max_attendees, rsvp_required) and NOT the
  internal-only fields (`created_by`, `created_at`, `updated_at`,
  `event_type_id`, `series_id`, `occurrence_index`).
- Members-only sanitization (nulling title/description/location/image_url) is
  unchanged and applied on top of the projection. AdminOnly events remain
  excluded. The upcoming filter, sort, and limit are unchanged.
- The iCal projection is already safe and unchanged.

## Impact

- Spec: `public-content-feeds` — 1 MODIFIED requirement.
- Code: `src/api/handlers/public.rs` (`list_events` builds a
  `PublicEvent` projection; document it in `src/api/docs.rs`).
- Tests: assert the JSON response omits `created_by` (and the other internal
  fields) for both a public and a members-only event; existing
  sanitization/past-exclusion tests unchanged.
- Marketing site: no change — it reads none of the dropped fields. Verify the
  home/calendar JS doesn't reference `created_by` (it doesn't).
