## Why

`src/web/templates/auth.rs:285-295` (the web/HTMX `login_handler`) calls:

```rust
let (_session, token) = auth_service
    .create_session(member.id, ...)
    .await
    .unwrap();
```

`create_session` returns `Result<_, AppError>` because it performs a SQLite `INSERT` (see `src/auth/session.rs:37-74`). Any DB-side error — write lock contention, disk-full, or the transient `database is locked` SQLite returns under load — turns a successful password verification into a panic inside the request handler. Axum maps the panic into a dropped connection rather than a clean 500.

Reachability: any user with valid credentials (no TOTP enrolled) while the DB is momentarily unavailable. The path is exercised on every successful non-TOTP web login.

Harm: an unwind through the handler discards in-flight state and produces a confusing user experience under load; the parallel paths in the same file at lines 244-258, 331-345, and 346-356 (cookie/redirect-header construction) AND the matching JSON handler in `src/api/handlers/auth.rs:131-132` already use proper `match`/`?` to surface a clean `500` instead of panicking. This is the only remaining `unwrap` on a fallible operation on this hot path.

The JSON `/auth/login` sibling (`src/api/handlers/auth.rs:131-132`) correctly uses `auth_service.create_session(...).await?` and returns a clean error — so the fix is a literal copy of that pattern across to the web handler.

## What Changes

Replace the `.unwrap()` with a `match` that returns `StatusCode::INTERNAL_SERVER_ERROR` plus a generic "Login failed. Please try again." `LoginResponse`, matching the shape used at `src/web/templates/auth.rs:218-229` (which already does this for the pending-login-mint failure). Log the error at `error` level via `tracing` so an operator can correlate with the underlying DB failure.

## Impact

- `src/web/templates/auth.rs` — single-handler change, ~15 lines added inside `login_handler`. No new dependencies, no new state on the handler signature.
- `openspec/specs/session-auth/spec.md` — add a scenario under the existing "Login creates a session row" requirement asserting the handler surfaces 500 on `create_session` failure rather than panicking.
- Tests: add a test that drives `login_handler` with a poisoned `SqlitePool` (pool closed, or `member_repo` faked to return a wrapped error) and asserts the response is `500 Internal Server Error` with the generic error body.
