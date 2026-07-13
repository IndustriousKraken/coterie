# signup-rejects-inactive-membership-type

## Why

`POST /public/signup` resolves `membership_type_slug` via
`MembershipTypeService::get_by_slug` → `find_by_slug`, which filters on `slug`
only and does NOT check `is_active`. So a signup that supplies a KNOWN but
DEACTIVATED membership-type slug is accepted and creates a Pending member on
that type. Inactive types are deliberately excluded from the public
`GET /public/membership-types` listing, but signup will still accept one that
an operator retired (or that an attacker guesses from a stale link). This is not
a payment bypass — the member is still Pending and activation is gated on the
Stripe webhook — but signup should not accept a membership type the org has
turned off.

Canon does not currently specify signup behavior for inactive slugs (only
"unknown slug fails loudly" is implied), so rejecting them is new behavior.

## What Changes

- `POST /public/signup` SHALL reject a `membership_type_slug` that resolves to
  an INACTIVE membership type with `400`, the same way an unknown slug is
  rejected — a deactivated type is not signup-able.
- An omitted slug continues to take the org default (the first active type by
  sort_order), and a known ACTIVE slug is unchanged.

## Impact

- Spec: `public-signup` — 1 ADDED requirement.
- Code: `src/api/handlers/public.rs::signup` slug resolution — after
  `get_by_slug`, reject when `!is_active` (or add an
  `MembershipTypeService::get_active_by_slug` helper). Do not weaken the
  unknown-slug 400.
- Tests: signup with an inactive slug → `400`, no member created; active slug
  and omitted slug unchanged.
