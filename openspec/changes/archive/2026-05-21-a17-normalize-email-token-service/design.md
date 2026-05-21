## Context

`EmailTokenService` was authored as a struct with two factory constructors, presumably anticipating a future where each token kind grew distinct behavior. That future hasn't arrived — both kinds use identical SQL with just a different table name. The "service" stores nothing but the pool reference and a static `&'static str` table name. It's not a trait; there's no testing seam; it's not stateful across calls.

In parallel, the codebase grew the convention that token issuance happens in the request handler (signup, password-reset request, resend-verification). Those handlers are pre-auth-tier or simple, so they construct ad-hoc rather than pulling from DI. The Arc on `ServiceContext` is a vestige of the planned-but-never-realized injected pattern.

The reviewer's question — "is it doing enough as an injected abstraction to justify being one?" — is a clean one. The answer is no. The two paths to coherence are A (commit to injection) and B (commit to free functions). B matches the actual shape of the code.

## Goals / Non-Goals

**Goals:**
- One canonical way to issue/consume an email token: a free function taking `&SqlitePool` plus the kind-specific arguments.
- The `EmailTokenService` struct and its Arc on `ServiceContext` are removed.
- Every caller (5 sites) uses the new function shape.
- Behavior is byte-identical: same tables, same TTLs, same hash format.

**Non-Goals:**
- Changing the token format, hash algorithm, or TTL defaults.
- Migrating the token storage tables.
- Adding new token kinds (e.g., magic-login tokens). If that ever happens, it's a new function pair; the structure doesn't need to anticipate it.
- Introducing a trait for token operations. There's no test fakery need; the integration tests use a real in-memory pool.

## Decisions

### D1. Free functions, kind-specific names

```rust
pub async fn create_verification_token(pool: &SqlitePool, member_id: Uuid, ttl: Duration) -> Result<CreatedToken>;
pub async fn consume_verification_token(pool: &SqlitePool, token: &str) -> Result<Option<ConsumedToken>>;
pub async fn create_password_reset_token(pool: &SqlitePool, member_id: Uuid, ttl: Duration) -> Result<CreatedToken>;
pub async fn consume_password_reset_token(pool: &SqlitePool, token: &str) -> Result<Option<ConsumedToken>>;
```

Considered: a single `pub enum TokenKind { Verification, PasswordReset }` plus `create_token(pool, kind, ...)`. Rejected — the kinds aren't interchangeable at the call site (a handler that issues a verification token never wants to issue a password-reset token in the same request), so enum dispatch doesn't help. Two named functions per kind are honest about what each caller does.

### D2. Internal helpers handle the SQL

```rust
async fn insert_token(pool: &SqlitePool, table: &'static str, member_id: Uuid, ttl: Duration) -> Result<CreatedToken>;
async fn consume_token(pool: &SqlitePool, table: &'static str, token: &str) -> Result<Option<ConsumedToken>>;
```

Private to the module. Public functions are 2-line wrappers passing the right `&'static str` table name. The table name is module-internal — never user input — so the existing `format!("INSERT INTO {} …", table)` interpolation pattern is safe.

### D3. `CreatedToken` and `ConsumedToken` types stay

The two return types (`CreatedToken { token, expires_at }`, `ConsumedToken { member_id }`) are useful at call sites. They stay as public types in `auth::email_tokens`.

### D4. `MemberService` resend-verification: pass the pool, not the service

`MemberService::resend_verification` currently takes `Arc<EmailTokenService>` in its constructor. Change: it stops taking the service; instead it receives `db_pool: SqlitePool` (which it might already need for other operations — verify during implementation). The method body calls `auth::email_tokens::create_verification_token(&self.db_pool, ...)` directly.

If `MemberService` doesn't already hold a pool, add one — the cost is one Arc-equivalent. This is the same trade-off `Notifications` and other services already make.

### D5. ServiceContext field removal

`ServiceContext::new` currently constructs `let email_token_service = Arc::new(EmailTokenService::verification(db_pool.clone()));` and stores it on the struct. Both lines are removed. The verification-only-on-ServiceContext oddity (no password-reset Arc) also goes away naturally — neither variant is on ServiceContext anymore.

### D6. Remove the FromRef impl, but tolerate the breakage

`a05-add-fromref-impls-on-appstate` added `impl FromRef<AppState> for Arc<EmailTokenService>`. Remove it. No handler should be extracting it via DI (the audit in the proposal confirms ad-hoc construction is the dominant pattern); if any does, the compiler will catch it.

### D7. Spec delta location

The existing `email-tokens` capability spec describes the service's behavior (token lifecycle, hash storage, etc.). Update it to describe the function-based API instead of the struct-based one. The lifecycle requirements (TTL semantics, atomic consume, token entropy) are unchanged.

## Risks / Trade-offs

- **Risk**: a caller I missed via grep extracts `State<Arc<EmailTokenService>>`. → Mitigation: compiler catches it. The grep results in the proposal are exhaustive given the simple type name.
- **Risk**: a future trait need (e.g., to inject a fake for tests) would force re-introducing the struct. → Acceptable: that's a tomorrow problem to solve when it appears. Today's tests work fine against the real pool, and the cost of going back to a struct is small if it's ever needed.
- **Trade-off**: kind-specific function names are slightly verbose. `create_verification_token` is 27 characters; the previous `EmailTokenService::verification(pool).create(...)` was longer. The new form reads cleaner at the call site.
- **Trade-off**: `MemberService` carrying a `SqlitePool` is mildly redundant if the service is mostly DB-via-repo today. Inspect during implementation; if it already has the pool for another reason, no new field. If not, the field is one `Arc`-equivalent, which matches what the service was holding before via the Arc'd `EmailTokenService` (which itself held the pool).

## Migration Plan

Single PR.

1. Add the four public functions and two private helpers in `src/auth/email_tokens.rs`. Keep the struct + old methods initially (for green-build incrementality).
2. Migrate the five call sites one at a time, running `cargo build` after each.
3. Remove the struct + impl block, the `ServiceContext` field, the `FromRef` impl.
4. `cargo test --features test-utils` — full suite passes.
5. Grep verify: `grep -rn "EmailTokenService" src/` returns nothing except possibly historical comments.
