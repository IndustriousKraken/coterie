# Tasks

## 1. Bound and validate signup text fields

- [x] 1.1 In `src/api/handlers/public.rs::signup`, after the
  bot-challenge gate and before `member_repo.create`, trim `email`,
  `username`, and `full_name` and validate:
  - `email`: non-empty, contains `@`, `len() <= 254` — else
    `AppError::BadRequest`.
  - `full_name`: non-empty after trim, `len() <= 200` — else
    `AppError::BadRequest`.
  - `username`: non-empty after trim, `len() <= 100` — else
    `AppError::BadRequest`.
  Use the trimmed values when building `CreateMemberRequest`.
- [x] 1.2 Keep the existing bot-challenge check, password validation,
  membership-type-slug resolution, and UNIQUE-violation mapping
  unchanged and in their current order.

## 2. Tests

- [x] 2.1 Add handler/integration tests for `signup` asserting `400`
  (`AppError::BadRequest`) when `full_name` exceeds 200 chars, when
  `username` exceeds 100 chars, when `email` exceeds 254 chars, and when
  `username` or `full_name` is empty/whitespace-only.
- [x] 2.2 Add a test asserting a normal-length, valid signup still
  succeeds (member created) — guarding against the bounds being too
  tight.
