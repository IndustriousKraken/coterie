## 1. Add the new public functions alongside the existing struct (incremental compile)

- [ ] 1.1 In `src/auth/email_tokens.rs`, refactor the existing `EmailTokenService::create` and `EmailTokenService::consume` bodies into two private free functions `async fn insert_token(pool: &SqlitePool, table: &'static str, member_id: Uuid, ttl: Duration) -> Result<CreatedToken>` and `async fn consume_token(pool: &SqlitePool, table: &'static str, token: &str) -> Result<Option<ConsumedToken>>`. The bodies are byte-identical to the existing struct-method bodies; they take the table as an explicit parameter instead of reading `self.table`.
- [ ] 1.2 Add the four public free functions, each a one-line wrapper:
  ```rust
  pub async fn create_verification_token(pool: &SqlitePool, member_id: Uuid, ttl: Duration) -> Result<CreatedToken> {
      insert_token(pool, "email_verification_tokens", member_id, ttl).await
  }
  pub async fn consume_verification_token(pool: &SqlitePool, token: &str) -> Result<Option<ConsumedToken>> {
      consume_token(pool, "email_verification_tokens", token).await
  }
  pub async fn create_password_reset_token(pool: &SqlitePool, member_id: Uuid, ttl: Duration) -> Result<CreatedToken> {
      insert_token(pool, "password_reset_tokens", member_id, ttl).await
  }
  pub async fn consume_password_reset_token(pool: &SqlitePool, token: &str) -> Result<Option<ConsumedToken>> {
      consume_token(pool, "password_reset_tokens", token).await
  }
  ```
- [ ] 1.3 Re-export the four new functions from `src/auth/mod.rs` (alongside the existing `pub use email_tokens::EmailTokenService;` line — keep the latter for now; it goes away in step 4).
- [ ] 1.4 `cargo build` — clean. The struct and the new functions coexist temporarily.

## 2. Migrate the five call sites

- [ ] 2.1 `src/web/templates/reset.rs:81` — replace `let service = EmailTokenService::password_reset(db_pool.clone()); … service.create(member.id, ttl).await` with `auth::email_tokens::create_password_reset_token(&db_pool, member.id, ttl).await`. Drop the `EmailTokenService` import if no longer needed in the file.
- [ ] 2.2 `src/web/templates/reset.rs:197` — same shape, but for `consume` (likely `consume_password_reset_token(&db_pool, token).await`).
- [ ] 2.3 `src/web/templates/verify.rs:39` — replace with `consume_verification_token(&db_pool, token).await`.
- [ ] 2.4 `src/api/handlers/public.rs:186` (inside `send_verification_email`) — replace with `create_verification_token(&db_pool, member.id, ttl).await`.
- [ ] 2.5 `src/service/member_service.rs::resend_verification` — change the dependency on `Arc<EmailTokenService>` to a `SqlitePool` (add the field to the struct if not already present; update `MemberService::new` constructor signature and every construction site). Body: `auth::email_tokens::create_verification_token(&self.db_pool, member.id, ttl).await`.
- [ ] 2.6 `cargo build` after each site — clean.

## 3. Verify no remaining struct usage

- [ ] 3.1 `grep -rn "EmailTokenService" src/` — confirm the only remaining references are in `email_tokens.rs` itself (the soon-to-be-deleted struct + impl).

## 4. Remove the old struct and its plumbing

- [ ] 4.1 Delete `pub struct EmailTokenService { … }` and `impl EmailTokenService { … }` from `src/auth/email_tokens.rs`. The private helpers and public free functions remain.
- [ ] 4.2 Delete `pub use email_tokens::EmailTokenService;` from `src/auth/mod.rs`.
- [ ] 4.3 In `src/service/mod.rs`, delete the `email_token_service: Arc<EmailTokenService>` field on `ServiceContext` and the line in `ServiceContext::new` that constructs it.
- [ ] 4.4 Delete `impl FromRef<AppState> for Arc<EmailTokenService>` from `src/api/state.rs` (added by `a05-add-fromref-impls-on-appstate`).
- [ ] 4.5 `cargo build --all-targets --features test-utils` — clean.

## 5. Test pass

- [ ] 5.1 `cargo test --features test-utils` — full suite passes. The existing email-token tests assert on database state, which is unchanged.
- [ ] 5.2 Manually verify the email-token integration test file (`tests/email_token_test.rs` or wherever it lives) was updated to call the free functions if it previously used the struct.
