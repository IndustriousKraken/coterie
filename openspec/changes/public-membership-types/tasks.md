# Tasks

## 1. Endpoint

- [x] 1.1 Add `GET /public/membership-types` to `public_routes` in
  `src/api/mod.rs`, handled in `src/api/handlers/public.rs`.
- [x] 1.2 Response: JSON array of active types ordered by `sort_order`,
  each `{ slug, name, description, fee_cents, currency, billing_period }`.
  Exclude inactive types. Do not expose internal ids.
- [x] 1.3 Document the endpoint + response schema in `src/api/docs.rs`.

## 2. Tests

- [x] 2.1 Handler test: returns active types in `sort_order`, excludes an
  inactive type, and serializes the expected fields (no `id`, no
  `is_active`).
- [x] 2.2 Handler test: empty table returns `200 []` (not an error).
