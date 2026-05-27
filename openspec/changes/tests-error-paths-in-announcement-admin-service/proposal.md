## Why

`src/service/announcement_admin_service.rs` exposes
`update(actor, id, input)`, `delete(actor, id)`, `publish(actor, id)`,
and `unpublish(actor, id)`. Each loads the announcement row via
`announcement_repo.find_by_id(id)` and unwraps with `.ok_or_else(||
AppError::NotFound("Announcement not found".to_string()))`. There are
three such call sites (lines 152, 209, 302).

The inline `#[cfg(test)] mod tests` block has seven `#[tokio::test]`
cases (create_draft, create_publish_now, update, delete, publish,
publish_is_idempotent_for_already_published, unpublish) — every one
covers the happy path. None passes a non-existent UUID. The behaviour
of "admin clicks Delete on a row that was concurrently deleted in
another tab" is therefore unverified: the code is intended to return
a typed 4xx, but no test prevents a regression to a panic or a
generic `Internal` error.

`publish` has an additional un-asserted invariant: when called on a
row whose `published_at` is already set, the method short-circuits
without writing an audit row (the `publish_is_idempotent_for_already_published`
test asserts the row's `published_at` doesn't change, but does NOT
assert that the audit table grew zero new rows). A regression that
added a phantom `publish_announcement` audit row would not be caught.

## What Changes

Add four new `#[tokio::test]` functions to the inline test module:
three NotFound assertions for update/delete/publish (and unpublish if
shape allows), plus one stricter assertion on the
already-published audit shape.

## Impact

- `src/service/announcement_admin_service.rs` — add four new
  `#[tokio::test]` functions to the existing inline test module.
  Reuses the existing helpers; no new infrastructure.
- No production code change. No existing tests are modified or
  deleted.
