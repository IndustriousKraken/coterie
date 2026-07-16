# Tasks

Behavior-preserving fix. The happy path (a successful `POST /setup` creates an
`Active`, `is_admin`, `bypass_dues` first admin, arms the cache, redirects to
`/login`) MUST stay identical — treat any change to it as a regression. The fix
only corrects the failure paths.

## 1. Stop swallowing the first-admin activation error

- [x] 1.1 In `src/web/templates/setup.rs::setup_handler`, replace the
  swallowing block at lines 188-190
  (`if let Err(e) = member_repo.update(member.id, update_request).await { tracing::error!(...) }`)
  with a fatal branch that mirrors the adjacent `set_admin` failure handling
  (lines 193-204): log the error and `return` a
  `StatusCode::INTERNAL_SERVER_ERROR` response with
  `SetupResponse { success: false, redirect: None, error: Some("Failed to activate admin user".into()) }`.
  Execution MUST NOT fall through to `set_admin`, to
  `admin_exists_observed.store(true, ...)` (line 210), or to the
  `success: true` return.
- [x] 1.2 Confirm by reading the surrounding code that on this new early
  return the cache flag is left `false` and no `HX-Redirect` / `success: true`
  is emitted.

## 2. Leave a retryable clean state on any post-create failure

- [x] 2.1 Add a small helper in `setup.rs` (e.g.
  `async fn delete_member_row(db_pool: &SqlitePool, id: Uuid)`) that runs a
  best-effort `DELETE FROM members WHERE id = ?` (bound parameter, `id` bound
  via `.bind(id)`), logging at `warn` on error and otherwise ignoring the
  result. The handler already has `db_pool` in scope (used by
  `check_admin_exists`).
- [x] 2.2 Call this cleanup on BOTH post-create failure paths before returning
  `500`: the new `update` failure branch from task 1, and the existing
  `set_admin` failure branch (lines 193-204). This removes the orphaned
  `Pending` member row so a subsequent `POST /setup` does not fail with a
  `UNIQUE` violation on email/username.
- [x] 2.3 Verify the cleanup is NOT run on the success path and does not touch
  any row other than the just-created `member.id`.

## 3. Regression test: setup yields an Active admin

- [x] 3.1 Add an integration test (new file
  `tests/setup_provisioning_test.rs`, following the real-SQLite-pool app
  harness pattern used by `tests/setup_redirect_test.rs` /
  `tests/create_admin_test.rs`) that issues a valid `POST /setup` against a
  fresh test app and then asserts the created member row has
  `status = Active`, `is_admin = 1`, and `bypass_dues = true`. This locks the
  post-condition so a future change cannot silently leave the first admin
  `Pending`.
- [x] 3.2 Assert the same request returns `200` with `success: true` and the
  `/login` redirect, so the test also guards the unchanged happy-path contract.

## 4. Verify

- [x] 4.1 Run `cargo test` (the existing `setup_redirect_test`,
  `create_admin_test`, and `csrf_coverage_test` suites plus the new test) and
  confirm all pass.
- [x] 4.2 Run `cargo clippy` on the touched file and confirm no new warnings.
