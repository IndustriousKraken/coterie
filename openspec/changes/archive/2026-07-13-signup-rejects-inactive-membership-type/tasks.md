# Tasks

## 1. Reject inactive types at signup

- [x] 1.1 In `src/api/handlers/public.rs::signup` slug resolution, after
  resolving the type via `get_by_slug`, reject with
  `AppError::BadRequest` (mirroring the unknown-slug 400) when the resolved
  type's `is_active` is false, BEFORE creating the member. Keep the
  unknown-slug and omitted-slug (org-default) behavior unchanged. (May add a
  `MembershipTypeService::get_active_by_slug` helper to keep the check in one
  place.)

## 2. Tests

- [x] 2.1 Signup with a slug that exists but is inactive → `400`; assert no
  member row is created.
- [x] 2.2 Signup with an active slug still succeeds (Pending member created);
  signup with an omitted slug still takes the org default. Unknown slug still
  `400`.
