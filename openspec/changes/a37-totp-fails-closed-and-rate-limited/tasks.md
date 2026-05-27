## 1. Fail-closed TOTP enrollment check — JSON

- [ ] 1.1 In `src/api/handlers/auth.rs::login`, replace:
  ```rust
  let totp_enabled = totp_service.is_enabled(member.id).await.unwrap_or(false);
  ```
  with:
  ```rust
  let totp_enabled = totp_service
      .is_enabled(member.id)
      .await
      .context("failed to query TOTP enrollment status")?;
  ```
- [ ] 1.2 Add `use anyhow::Context;` if not already present in the file's `use` block.
- [ ] 1.3 Verify the surrounding `Result<Response>` return type accepts `?` on the error; if the error type doesn't impl `From<anyhow::Error>`, convert via `.map_err(...)` to `AppError::Internal` so the existing `IntoResponse` impl produces a 500.

## 2. Fail-closed TOTP enrollment check — web

- [ ] 2.1 In `src/web/templates/auth.rs::login_handler`, make the same change as 1.1 to its `is_enabled` call site.
- [ ] 2.2 Match whatever error-surfacing pattern the web handler already uses (likely returning a 500 response directly rather than `?`-propagation). Mirror the file's existing convention.

## 3. Rate-limit the JSON second-factor endpoint

- [ ] 3.1 In `src/api/handlers/auth.rs::login_totp`, add `State(login_limiter): State<LoginLimiter>,` to the handler signature (alphabetized appropriately among the other `State(...)` extractors).
- [ ] 3.2 At handler entry (before the pending-token lookup), add:
  ```rust
  let ip = state::client_ip(&headers, settings.server.trust_forwarded_for());
  if !login_limiter.0.check_and_record(ip) {
      return Err(AppError::TooManyRequests);
  }
  ```
  Note: the handler currently takes `_headers: HeaderMap` (underscored); rename to `headers: HeaderMap` so it's used. The `settings` extractor is already in scope.

## 4. Rate-limit the web second-factor endpoint

- [ ] 4.1 In `src/web/templates/auth.rs::login_totp_handler`, add `State(login_limiter): State<LoginLimiter>,` to the handler signature.
- [ ] 4.2 The handler currently does NOT extract `HeaderMap`. Add `headers: HeaderMap,` to the signature.
- [ ] 4.3 At handler entry, add the same rate-limit check as 3.2, but return the web-shaped 429 response (match the existing `/login` handler's 429 pattern, which returns `Json(LoginResponse { success: false, error: Some("Too many login attempts. Please try again later."), .. })` with `StatusCode::TOO_MANY_REQUESTS`).

## 5. Tests — JSON

- [ ] 5.1 In `tests/api_login_totp.rs`, add `json_login_returns_500_if_totp_enrollment_check_errors`:
  - Build the harness with a `TotpService` impl that returns `Err(...)` from `is_enabled`. (If the existing harness constructs the real `TotpService`, introduce a fake or use a feature-gated `FailingTotpService` in `tests/common/mod.rs`.)
  - Submit valid email+password.
  - Assert status code is 500.
  - Assert no `session` cookie in the response.
  - Assert no `pending_login` cookie in the response.
- [ ] 5.2 Add `json_login_totp_returns_429_after_budget_exhausted`:
  - Provision a TOTP-enrolled member; complete a successful `/auth/login` to get a pending token.
  - Submit 5 invalid TOTP codes from the same IP to `/auth/login/totp` (each returns 401).
  - 6th submission to `/auth/login/totp` returns 429.

## 6. Tests — web

- [ ] 6.1 If `tests/web_login_totp.rs` (or a similarly-named test) doesn't exist, create one following the harness pattern in `tests/api_login_totp.rs` adapted to the web router.
- [ ] 6.2 Add `web_login_returns_500_if_totp_enrollment_check_errors` — same shape as 5.1 against `POST /login`.
- [ ] 6.3 Add `web_login_totp_returns_429_after_budget_exhausted` — same shape as 5.2 against `POST /login/totp`.

## 7. Spec deltas

- [ ] 7.1 OpenSpec applies the MODIFIED operations in this change's `specs/session-auth/spec.md` and `specs/rate-limiting/spec.md` to the canonical specs at archive time. No manual canonical-spec edits needed.

## 8. Validation

- [ ] 8.1 `cargo build` — clean.
- [ ] 8.2 `cargo test --features test-utils` — all tests pass including the new ones.
- [ ] 8.3 `cargo clippy --features test-utils -- --deny warnings` — clean.
- [ ] 8.4 `cargo fmt --check` — clean.
- [ ] 8.5 `openspec validate a37-totp-fails-closed-and-rate-limited` — clean.
