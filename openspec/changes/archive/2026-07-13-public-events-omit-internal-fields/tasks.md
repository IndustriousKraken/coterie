# Tasks

## 1. Public projection

- [x] 1.1 Add a `PublicEvent` serialize struct in
  `src/api/handlers/public.rs` (ToSchema) exposing only: `id`, `title`,
  `description`, `event_type`, `visibility`, `start_time`, `end_time`,
  `timezone`, `location`, `image_url`, `max_attendees`, `rsvp_required`. Omit
  `created_by`, `created_at`, `updated_at`, `event_type_id`, `series_id`,
  `occurrence_index`.
- [x] 1.2 In `list_events`, keep combining `list_public()` + sanitized
  `list_members_only()`, keep `derive_utc_instants` + upcoming-filter + sort +
  limit, then map each `Event` to `PublicEvent` for the JSON response. iCal
  path unchanged.
- [x] 1.3 Register `PublicEvent` in `src/api/docs.rs`; update the `list_events`
  response body schema to `[PublicEvent]`.

## 2. Tests

- [x] 2.1 Assert the JSON `/public/events` response for a PUBLIC event omits
  `created_by`, `created_at`, `updated_at`, `event_type_id`, `series_id`,
  `occurrence_index`.
- [x] 2.2 Assert a MEMBERS-ONLY event still has the sanitized
  title/description and null location/image_url, real start/end times, AND
  omits the internal fields.
