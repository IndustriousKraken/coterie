## Why

`require_setup` (in `src/api/middleware/setup.rs`) is the outer middleware on every request. After path-prefix shortcuts for `/setup`, `/static`, `/assets`, `/favicon`, it runs:

```sql
SELECT 1 FROM members WHERE is_admin = 1 LIMIT 1
```

…on every non-static request, every time, forever. Once the org has completed first-boot setup and an admin exists, this query's result is **`Ok(Some(_))` for the rest of the process lifetime** — but the middleware re-runs the query on every request anyway. The reason it can't flip back to `false` in normal operation: there's no admin handler that deletes a member, no handler that clears `is_admin = 1` (the closest paths — suspend, expire — leave the flag alone), and the setup flow is single-flight via `setup_lock`. The only way to reach a "no admin exists" state again is direct DB manipulation outside the app.

So the middleware is doing one DB query per request to confirm a value that's been "yes" since the first hit after setup. For an instance handling typical portal traffic (a few requests per second per active user), that's millions of redundant point queries per day per org, at the front of the request chain (every request has to wait for it).

The fix is a process-local cache: the first time the middleware observes `has_admin = true`, set an `Arc<AtomicBool>` on `AppState` and never query again for the rest of the process lifetime. Until then, the query runs (so the setup-redirect still fires correctly during first-boot). This is a single-line check at the top of the middleware and a single-line store after a positive lookup — small change, large reduction in DB load.

## What Changes

- **Add `admin_exists_observed: Arc<AtomicBool>` field to `AppState`**. Initialized to `false` in `AppState::new`.
- **Update `require_setup` middleware** to short-circuit when the flag is `true`:
  ```rust
  if state.admin_exists_observed.load(Ordering::Relaxed) {
      return next.run(request).await;
  }
  let has_admin = check_admin_exists(&state).await;
  if has_admin {
      state.admin_exists_observed.store(true, Ordering::Relaxed);
      return next.run(request).await;
  }
  Redirect::to("/setup").into_response()
  ```
- **Update the setup-wizard handler** (`src/web/templates/setup.rs`) to set the flag immediately after creating the first admin. This proactively avoids one final round-trip on the next request after setup completes.
- **Document the lifecycle** in the middleware's doc comment: once `true`, the flag is sticky for the process lifetime; deleting the last admin via direct SQL requires a server restart to re-trigger the setup-redirect path.
- **Out of scope**: removing the middleware entirely on a "post-setup" deployment, or using a `OnceCell` (more ceremony for the same outcome). An `Arc<AtomicBool>` is the simplest shape that fits.
- **Out of scope**: caching the negative case (`has_admin = false`). The query is cheap, and during first-boot the setup wizard runs in seconds — a per-request query during that window is fine.

## Capabilities

### New Capabilities

(None — internal optimization. No new capability spec.)

### Modified Capabilities
- `routing-architecture`: adds an internal-state requirement that `AppState` tracks an `admin_exists_observed: Arc<AtomicBool>` flag, and that `require_setup` short-circuits when the flag is `true`. The first-boot redirect contract (anonymous browser sees `/setup` until an admin exists) is preserved.

## Impact

- **Code**:
  - `src/api/state.rs`: ~3 lines added (field declaration, initializer, the `Arc<AtomicBool>` import).
  - `src/api/middleware/setup.rs`: ~6 lines added (load/store + the early return).
  - `src/web/templates/setup.rs`: ~1 line added (proactive `store(true, ...)` after admin creation).
  - Net: ~10 lines added; no lines removed.
- **Wire shape**: zero change. Setup-needing browsers still get redirected to `/setup`; admins-already-exist instances continue to forward through the chain.
- **Performance**: after first request post-setup, every subsequent request avoids one DB round-trip. For a portal at typical traffic (~1–10 rps), that's ~85k–850k fewer queries per day per instance.
- **Tests**: existing tests cover the redirect-during-first-boot and forward-when-admin-exists paths via integration tests that boot a fresh app. They continue to pass without modification — both paths still work; the cache is invisible at the wire layer. Add one unit test that asserts the second invocation of the middleware (after a positive first lookup) does not hit the DB.
- **Risk**: very low. The cache is correct as long as the underlying invariant holds (`is_admin = true` is monotonic post-setup in normal operation). Direct SQL that unsets the flag for every admin would leave the cache stale until restart — flagged in the doc comment, not a real-world workflow.
