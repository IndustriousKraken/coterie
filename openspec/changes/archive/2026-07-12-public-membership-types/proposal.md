# public-membership-types

## Why

The marketing site's join form must let a visitor choose a membership type,
and `POST /public/signup` 400s on an unknown `membership_type_slug` — but
there is no unauthenticated way to discover valid types. `public_routes`
(`src/api/mod.rs:168-182`) exposes events/announcements/feeds/signup/donate
only; membership types are reachable solely through admin-gated portal
routes. Today the join form hardcodes a slug, which silently drifts from
the DB (the live form offers "standard", which is not a configured type —
signups through it fail).

`MembershipTypeService::list` already exists (`membership_type_service.rs:22`)
— the gap is one thin public read endpoint.

## What Changes

- Add `GET /public/membership-types`: returns the active membership types
  (slug, name, description, fee, billing period) ordered by `sort_order`,
  for unauthenticated callers. Inactive types are excluded.
- Document it in the OpenAPI spec (`src/api/docs.rs`) like the other
  public endpoints.
- Read-only, non-money-moving: same gate class as `/public/events` (CORS
  allowlist for browsers; no bot challenge, no money limiter).

## Impact

- Spec: new capability `public-membership-types` (ADDED requirement).
- Code: `src/api/handlers/public.rs` (or a sibling handler) + route in
  `src/api/mod.rs` + `src/api/docs.rs` entry.
- Tests: handler test asserting active-only, ordering, and response shape.
- No schema changes. Fee and billing period are already non-secret (they
  are shown on the public join page by design).
