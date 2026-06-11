## Context

`require_setup` is wired as the outermost middleware (after CSRF) on the merged app router in `src/main.rs`:

```rust
let app = api_app
    .merge(web_app)
    .layer(from_fn_with_state(app_state.clone(), require_setup))
    .layer(from_fn_with_state(app_state, csrf_protect_unless_exempt));
```

…which means every non-CSRF-rejected request goes through it. The middleware's job: if no admin exists yet, redirect to `/setup` so the operator can create one. Once at least one admin exists, just forward.

Today it does this by running a SQL query *every* request:

```rust
async fn check_admin_exists(state: &AppState) -> bool {
    let result: Result<Option<(i64,)>, _> = sqlx::query_as(
        "SELECT 1 as exists_flag FROM members WHERE is_admin = 1 LIMIT 1",
    )
    .fetch_optional(&state.service_context.db_pool)
    .await;
    // ... map to bool
}
```

The query is cheap (indexed lookup, returns ≤1 row, point read), but it's still a DB round-trip on every request. At idle that's nothing; under any meaningful traffic it's measurable contention against a SQLite write-ahead log that has only one writer slot.

The deeper observation: after the org's first-boot setup completes, the answer to "does any admin exist?" is "yes" for the rest of the process lifetime. The codebase has no admin handler, no service method, no public API path that flips `is_admin = 1` → `0` for the last remaining admin. (There are paths to suspend/expire members, but those don't touch `is_admin`. There's no `DELETE FROM members` admin handler.) So the value is *monotonic* post-setup.

The fix is the simplest cache shape that exploits monotonicity: an `AtomicBool` that starts `false` and never reverts after being set to `true`. Once observed-true, the middleware skips the query forever.

## Goals / Non-Goals

**Goals:**
- Eliminate the per-request `SELECT 1 FROM members WHERE is_admin = 1 LIMIT 1` query in steady-state operation.
- Preserve the first-boot redirect: a fresh-install instance with no admin still redirects to `/setup`.
- Preserve the post-setup forward: a normal instance with an admin still forwards every request through.
- Single small change: one new `AppState` field, one branch in the middleware, one proactive store at the end of the setup-wizard handler.

**Non-Goals:**
- Caching the negative case (`has_admin = false`). The first-boot window is measured in minutes at most; per-request queries during that window are fine and safer (no risk of stale-no when the wizard creates the admin).
- Replacing the middleware with a "compile-out post-setup" mechanism that fully removes it from the layer chain on production deployments. Adds operational complexity (which deployment? which restart strategy?) for the same payoff.
- Using `tokio::sync::OnceCell` or `once_cell::sync::OnceCell`. `AtomicBool` with `Ordering::Relaxed` does the same job with less ceremony for a single-bool flag.
- Tracking the admin's id, count, or any other admin-related state. We only care about `>= 1` admin exists.
- Handling the "operator manually deleted all admins via direct SQL" scenario gracefully (without restart). That's a sysadmin workflow with a sysadmin recovery: restart the server.

## Decisions

### D1. `Arc<AtomicBool>` over `OnceCell<()>` or `RwLock<bool>`

```rust
use std::sync::atomic::{AtomicBool, Ordering};

pub struct AppState {
    // ... existing fields
    pub admin_exists_observed: Arc<AtomicBool>,
}
```

The semantic is "once-true-forever-true." `AtomicBool` matches exactly: load → branch → maybe-store. No unwrapping ceremony, no async lock acquisition, no allocation per check. `Ordering::Relaxed` is appropriate because the value's truthiness is the only thing that matters; ordering relative to other memory operations isn't meaningful here.

`OnceCell<()>` would also work but adds the awkwardness of "the unit value carries the meaning." `RwLock<bool>` is overbuilt — atomics are the right shape for monotonic flags.

### D2. The Arc wrapper is for `Clone`, not for shared mutability

`AppState` derives `Clone` and is cloned per-request via `State<AppState>`. `AtomicBool` doesn't implement `Clone`; wrapping in `Arc` makes the field cloneable. This is the same pattern used by `setup_lock: Arc<AsyncMutex<()>>` already on `AppState`.

### D3. Middleware shape: load → fast path → query → maybe-store → branch

```rust
pub async fn require_setup(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();

    // Path prefixes that always pass through (existing behavior).
    if path.starts_with("/setup")
        || path.starts_with("/static")
        || path.starts_with("/assets")
        || path.starts_with("/favicon")
    {
        return next.run(request).await;
    }

    // Fast path: once we've ever observed an admin, never query again.
    if state.admin_exists_observed.load(Ordering::Relaxed) {
        return next.run(request).await;
    }

    // Cold path: query the DB. If admin exists, set the flag
    // (write order doesn't matter — concurrent requests racing here
    //  all observe the same DB result and converge).
    let has_admin = check_admin_exists(&state).await;
    if has_admin {
        state.admin_exists_observed.store(true, Ordering::Relaxed);
        return next.run(request).await;
    }

    // Still pre-setup; redirect.
    Redirect::to("/setup").into_response()
}
```

### D4. Setup-wizard handler proactively sets the flag after creating the admin

Inside `src/web/templates/setup.rs`, after `member_repo.create(create_request).await` succeeds and the admin is promoted via `set_admin(member.id, true)`:

```rust
state.admin_exists_observed.store(true, Ordering::Relaxed);
```

This is belt-and-suspenders: even without it, the next request would query, observe `true`, and set the flag. With it, that one query is skipped. The cost is one line; the benefit is principled "the system knows it just created an admin, so it shouldn't have to ask the DB."

### D5. The race between concurrent first-boot requests is benign

Two concurrent requests during first-boot can both reach the query. Both will read the same DB state (either pre-admin or post-admin). If both read pre-admin, both redirect to `/setup` — correct. If both read post-admin, both store `true` (idempotent) and forward — correct. The `setup_lock` on the wizard handler is what serializes admin *creation*; the cache is only about reading the existence bit, which is naturally race-tolerant.

### D6. Doc comment on the middleware describes the cache lifecycle

The doc comment notes:
- The flag is set on the first observed `true` and never cleared for the rest of the process lifetime.
- If an operator removes admin status from every member via direct SQL, the middleware will continue to forward (cache is stale); a server restart re-arms the setup-redirect path.
- This is acceptable because the codebase has no application-level path that demotes the last admin.

### D7. Test coverage: assert no second query

Add a unit test that:
1. Constructs an `AppState` with an in-memory SQLite pool seeded with one admin row.
2. Invokes `require_setup` against a non-static, non-setup path (e.g., `/portal/dashboard`).
3. Confirms the response forwards (not a redirect).
4. Truncates the `members` table — no admins now exist in the DB.
5. Invokes `require_setup` again against the same path.
6. Confirms the response *still* forwards (cache says "yes admin exists" even though the DB now disagrees).

The truncation step is the assertion that the cache is real — without it, the second invocation would re-query and redirect.

## Risks / Trade-offs

- **Risk**: an operator manually deletes all admins via direct SQL and expects the next request to redirect to `/setup`. → **Mitigation**: documented in the middleware's doc comment; the recovery is a server restart. This workflow doesn't exist in any application-level path.
- **Risk**: a future contributor adds an admin-demotion handler (e.g., `POST /portal/admin/members/:id/demote`) that removes `is_admin` from the last admin without invalidating the cache. → **Mitigation**: any future path that can demote the last admin must invalidate the cache (`store(false, ...)`). Worth flagging in the doc comment so a reviewer notices.
- **Trade-off**: the cache is process-local. A multi-process deployment (e.g., systemd unit with multiple workers) would have each process query independently the first time. That's fine — each process independently caches.
- **Trade-off**: `Ordering::Relaxed` is the loosest memory ordering. Any reasonable architecture handles a monotonic bool correctly under relaxed ordering. If a future contributor wonders "should this be `Acquire`/`Release`?", the doc comment can clarify the answer is no.

## Migration Plan

Single PR. Pure-internal optimization, no wire-shape change.

1. Add `admin_exists_observed: Arc<AtomicBool>` to `AppState` with `false` as the initializer.
2. Add the load/store branches in `require_setup`.
3. Add the proactive `store(true, ...)` at the end of the successful setup-wizard path.
4. Add the unit test from D7.
5. `cargo build` + `cargo test --features test-utils`.
6. Deploy normally; no flags, no migrations. `git revert` is the rollback.
