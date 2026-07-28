## 1. Make the admin-existence check fail closed

- [x] 1.1 In `src/web/templates/setup.rs::check_admin_exists`, change the
  `Err(e)` arm to log and return `true` ("assume an admin exists") instead
  of `false`. Update the doc comment on the function to state that an
  unreadable answer is treated as "an admin exists", so the unauthenticated
  wizard can never be opened by a database error — and note that this now
  matches `src/api/middleware/setup.rs::check_admin_exists`.
- [x] 1.2 In `src/web/templates/setup.rs::setup_handler`, before acquiring
  `setup_lock`, return the existing "Setup has already been completed"
  `400` response when `admin_exists_observed.load(Ordering::Relaxed)` is
  `true`. `setup_handler` already receives
  `State(admin_exists_observed): State<Arc<AtomicBool>>`, so no signature
  change is needed. Keep the in-lock `check_admin_exists` re-check as well
  — the flag is a fast path, not a replacement.

## 2. Regression tests

- [x] 2.1 Add a test `setup_refuses_when_admin_check_errors` in
  `src/web/templates/setup.rs`'s test module: build the setup route with a
  `SqlitePool` whose underlying connection is unusable (e.g. connect to a
  migrated in-memory DB, then `pool.close().await` so every query returns
  `Err`), POST a valid `SetupRequest`, and assert the response status is
  `400` and the body's `error` is `Some`. Guards the fail-closed direction.
- [x] 2.2 Add a test `setup_refuses_when_admin_already_observed`: build the
  route with a working migrated pool that contains NO admin row but with
  `admin_exists_observed` pre-set to `true`, POST a valid `SetupRequest`,
  and assert `400` and that `SELECT COUNT(*) FROM members` is still `0`.
- [x] 2.3 Add (or extend) a happy-path test `setup_creates_first_admin` on
  a fresh migrated pool with `admin_exists_observed = false`: assert `200`,
  that exactly one member row exists with `is_admin = 1` and
  `status = 'Active'`, and that `admin_exists_observed` is `true`
  afterwards. Proves the fix did not break first-boot.
