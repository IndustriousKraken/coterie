## 1. AppState plumbing

- [ ] 1.1 In `src/api/state.rs`, add `use std::sync::atomic::AtomicBool;` to the imports.
- [ ] 1.2 Add a new field to `AppState`:
  ```rust
  /// Process-local cache for "has any admin been observed in the DB?".
  /// Set to true on the first positive lookup and never cleared. See
  /// `require_setup` for the lifecycle rationale.
  pub admin_exists_observed: Arc<AtomicBool>,
  ```
- [ ] 1.3 In `AppState::new`, initialize the field to `Arc::new(AtomicBool::new(false))` alongside the existing `setup_lock` initialization.
- [ ] 1.4 `cargo build` — clean (no callers reference the new field yet).

## 2. Middleware short-circuit

- [ ] 2.1 In `src/api/middleware/setup.rs`, add `use std::sync::atomic::Ordering;`.
- [ ] 2.2 After the existing path-prefix shortcuts (`/setup`, `/static`, `/assets`, `/favicon`), add the cache check:
  ```rust
  if state.admin_exists_observed.load(Ordering::Relaxed) {
      return next.run(request).await;
  }
  ```
- [ ] 2.3 In the existing positive-lookup branch (`if has_admin { ... next.run(request).await }`), add a `store` immediately before forwarding:
  ```rust
  let has_admin = check_admin_exists(&state).await;
  if has_admin {
      state.admin_exists_observed.store(true, Ordering::Relaxed);
      return next.run(request).await;
  }
  Redirect::to("/setup").into_response()
  ```
  (Adjust the existing control flow shape minimally; the function ends with the redirect on the negative path as today.)
- [ ] 2.4 Update the doc comment on `require_setup` to describe the cache lifecycle (sticky once-true, restart-required to re-arm if admins are manually removed via direct SQL).
- [ ] 2.5 `cargo build` — clean.

## 3. Setup-wizard proactive store

- [ ] 3.1 In `src/web/templates/setup.rs`, after the admin is created and promoted (after `set_admin(...)` succeeds, before the `SetupResponse` is built), add:
  ```rust
  state.admin_exists_observed.store(true, Ordering::Relaxed);
  ```
- [ ] 3.2 Add `use std::sync::atomic::Ordering;` to the file's imports if not already present.
- [ ] 3.3 `cargo build` — clean.

## 4. Test

- [ ] 4.1 Add a unit test (in `src/api/middleware/setup.rs` or a sibling test file) that:
  - Constructs an `AppState` with an in-memory SQLite pool seeded with one admin row.
  - Invokes `require_setup` against a non-static, non-setup-prefix path; asserts the response is a forward (not a redirect).
  - Truncates the `members` table.
  - Invokes `require_setup` again against the same path; asserts the response is *still* a forward (cache persists despite empty table).
- [ ] 4.2 Add a complementary unit test that confirms the negative case (no admin, no cache): construct an `AppState` with an empty `members` table, invoke `require_setup`, assert a redirect to `/setup`, assert `admin_exists_observed` is still `false`.
- [ ] 4.3 `cargo test --features test-utils` — all pass.

## 5. Integration test of the wizard → cache transition

Section 4's unit tests already prove the middleware caches a positive lookup and survives a post-cache DB truncation. This section adds one router-level integration test for the wizard-arms-cache path so the spec scenario about proactive store at the end of setup is exercised end-to-end without any manual steps.

- [ ] 5.1 Add a test (in `src/api/middleware/setup.rs` test module or a new `tests/setup_redirect_test.rs`) that boots a `Router` via `coterie::api::create_app` + `coterie::web::create_web_routes` against an in-memory SQLite pool with migrations applied but no admin row.
- [ ] 5.2 Drive a request to a non-static, non-setup path through `tower::ServiceExt::oneshot`. Assert: response is a redirect to `/setup`, and `state.admin_exists_observed.load(Ordering::Relaxed)` is still `false`.
- [ ] 5.3 Drive a POST to `/setup` (the wizard handler) with valid form data via `oneshot`. Assert: response is success, and `state.admin_exists_observed.load(Ordering::Relaxed)` is now `true` (this proves the proactive store in `src/web/templates/setup.rs` actually fires).
- [ ] 5.4 Drive a follow-up request to the same non-static path. Assert: response forwards through (no redirect), and the test asserts via sqlx query logging or a wrapper counter that `check_admin_exists` was NOT invoked on this third request. If counting calls is awkward, settle for asserting the forward response — the unit tests in Section 4 already prove the no-query-when-cached path.

## 6. Spec sync

- [ ] 6.1 Confirm the change's delta spec (`openspec/changes/cache-has-admin-flag/specs/routing-architecture/spec.md`) matches the implemented behavior.
- [ ] 6.2 At archive time (`opsx:archive`), the new requirement about the process-cached setup-redirect check merges into `openspec/specs/routing-architecture/spec.md`.
