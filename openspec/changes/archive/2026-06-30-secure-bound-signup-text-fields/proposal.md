# secure-bound-signup-text-fields

## Why

`POST /public/signup` (`src/api/handlers/public.rs::signup`,
lines 97-131) is an **unauthenticated** endpoint that persists three
free-text fields — `email`, `username`, `full_name` — with essentially
no input bounding. The only validation is:

```rust
if !request.email.contains('@') {
    return Err(AppError::BadRequest("Invalid email format".to_string()));
}
```

`username` and `full_name` get **no** length or non-empty check, and
`email` gets **no** length cap, before `member_repo.create(...)` writes
the row. The sibling public-donate handler in the same file
(`src/api/handlers/public.rs:608-621`) already bounds the equivalent
fields — `email.len() <= 254`, `name.len() <= 200`, and rejects empty
values — so the signup path is the inconsistent one.

- **Trigger:** unauthenticated POST to `/public/signup` (gated only by
  the bot challenge; deliberately **not** behind `money_limiter` per the
  public-signup spec) with megabyte-sized `email` / `username` /
  `full_name` values.
- **Harm:** unbounded row growth from unauthenticated input — a
  storage-abuse / resource-exhaustion vector. These values are also
  later emitted verbatim in the admin CSV export.

This adds a new validation invariant at the signup HTTP boundary —
oversized/empty fields previously **accepted** (member created) are now
**rejected** (`400`) — so it changes an observable contract and lands in
the spec lane.

## What Changes

- In `signup`, before creating the member, trim and bound the text
  fields, mirroring the donate handler's caps:
  - `email`: non-empty, contains `@`, `len() <= 254`.
  - `full_name`: non-empty after trim, `len() <= 200`.
  - `username`: non-empty after trim, `len() <= 100`.
  Reject with `AppError::BadRequest` and a clear message on violation.
- No change to the existing bot-challenge gate, membership-type
  resolution, or the UNIQUE-violation handling.

## Impact

- Spec: `public-signup` — `ADDED` requirement "Signup bounds and
  validates its input fields".
- Code: `src/api/handlers/public.rs` (`signup`).
- Tests: handler-level tests asserting oversized/empty fields are
  rejected with `400`.
- No database schema, wire-format, or API-shape changes. The specific
  caps (254 / 200 / 100) match the existing donate handler and are
  implementation guidance; the binding invariant is that the signup
  text fields are non-empty and length-bounded.
