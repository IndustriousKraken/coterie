## Why

`EmailTokenService` lives in a confused middle ground. It's a struct with two factory constructors (`verification(pool)` and `password_reset(pool)`) and two methods (`create`, `consume`). `ServiceContext` holds one Arc'd instance (the verification variant) and exposes it via dependency injection. But callers don't actually use the injected version — every caller constructs an ad-hoc local instance per request:

- `src/web/templates/reset.rs:81, :197` — `EmailTokenService::password_reset(db_pool.clone())`
- `src/web/templates/verify.rs:39` — `EmailTokenService::verification(db_pool.clone())`
- `src/api/handlers/public.rs:186` — `EmailTokenService::verification(db_pool.clone())`

The result: we pay the cost of a service abstraction (struct definition, ServiceContext field, Arc plumbing, FromRef impl after `a05`) without the benefit (per-request construction is the dominant pattern). An architectural reviewer flagged this as not pulling its weight.

Two coherent shapes exist; the current state is neither:
- **A. Inject properly**: every caller uses the Arc; ServiceContext holds one Arc per token variant (verification + password-reset); no ad-hoc construction.
- **B. Strip to free functions**: drop the struct entirely; expose `create_verification_token`, `consume_verification_token`, `create_password_reset_token`, `consume_password_reset_token` as free functions taking `&SqlitePool`. ServiceContext loses the field; ad-hoc construction is the only path because there's nothing to construct.

This change picks **B** (free functions) for reasons spelled out in design.md. The current "service" has no state beyond the pool, no trait, no testing seam — it's a glorified namespace for two SQL queries. Free functions match what it actually is.

## What Changes

- **Delete `pub struct EmailTokenService`** from `src/auth/email_tokens.rs`.
- **Replace with four free functions**:
  - `pub async fn create_verification_token(pool: &SqlitePool, member_id: Uuid, ttl: Duration) -> Result<CreatedToken>`
  - `pub async fn consume_verification_token(pool: &SqlitePool, token: &str) -> Result<Option<ConsumedToken>>`
  - `pub async fn create_password_reset_token(pool: &SqlitePool, member_id: Uuid, ttl: Duration) -> Result<CreatedToken>`
  - `pub async fn consume_password_reset_token(pool: &SqlitePool, token: &str) -> Result<Option<ConsumedToken>>`
- The private `INSERT` and `SELECT … DELETE WHERE` SQL is factored into private `insert_token(pool, table, …)` and `consume_token(pool, table, …)` helpers; the public functions are 2-line wrappers passing the right table name.
- **Remove `email_token_service: Arc<EmailTokenService>` from `ServiceContext`** and its construction in `ServiceContext::new`.
- **Remove the `FromRef<AppState> for Arc<EmailTokenService>` impl** in `src/api/state.rs` (added by `a05`).
- **Remove the `MemberService::resend_verification` dependency on `Arc<EmailTokenService>`** — the service receives `&SqlitePool` (which it already needs for its other work) and calls the free function directly.
- **Update every caller**:
  - `src/web/templates/reset.rs` — two sites; replace `EmailTokenService::password_reset(db_pool.clone())` + `.create(...)` / `.consume(...)` with the free-function calls.
  - `src/web/templates/verify.rs` — one site; same shape.
  - `src/api/handlers/public.rs::send_verification_email` — one site; same shape.
  - `src/service/member_service.rs::resend_verification` — one site; same shape.
- **Out of scope**: the underlying token storage (still two tables: `email_verification_tokens`, `password_reset_tokens`). No migration. The hash/random-generation logic stays.

## Capabilities

### New Capabilities

(None — internal refactor.)

### Modified Capabilities
- `email-tokens`: shape requirement updates — the API surface is free functions, not a struct, and ServiceContext does not hold an Arc'd instance.

## Impact

- **Code**: `src/auth/email_tokens.rs` shrinks (lose the struct definition + impl block; gain four 2-line public wrappers + two private helpers). Net: ~10 lines smaller. `src/service/mod.rs` loses ~4 lines (no `email_token_service` field and its construction). `src/api/state.rs` loses the FromRef impl (~5 lines). ~5 caller sites get shorter.
- **Wire shape**: zero change. Same tables, same token format, same TTLs, same audit behavior.
- **Tests**: existing `tests/email_token_test.rs` (recently archived as `tests-error-paths-in-email-token-service`) test the SQL behavior directly — they continue to work; they construct the test scenario with a pool, and the assertions are on the database side. The minor change is replacing `let svc = EmailTokenService::verification(pool); svc.create(...)` with `email_tokens::create_verification_token(&pool, ...)`.
- **Risk**: low. Pure mechanical refactor; no behavior change.
