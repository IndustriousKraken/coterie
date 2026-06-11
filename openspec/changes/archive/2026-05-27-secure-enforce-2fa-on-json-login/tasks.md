## 1. Gate JSON login on TOTP enrollment

- [x] 1.1 In `src/api/handlers/auth.rs::login`, after `verify_password` succeeds and the member's status is accepted, add a call to `totp_service.is_enabled(member.id)`. Inject `Arc<TotpService>` and `Arc<PendingLoginService>` via `State` extractors the same way `src/web/templates/auth.rs::login_handler` does.
- [x] 1.2 If `is_enabled` returns `true`, do NOT call `invalidate_all_sessions`, do NOT call `create_session`, and do NOT set a `session` cookie. Instead, mint a `pending_login` via `pending_login_service.create(member.id, ...)`, attach the `pending_login` cookie to the response jar via `crate::auth::pending_login::create_cookie(...)`, and return `StatusCode::ACCEPTED` with a JSON body like `{"message": "2fa_required", "pending_token": "<token>"}` so JSON clients that don't carry cookies can still complete the flow.
- [x] 1.3 Preserve the existing behavior for members WITHOUT TOTP enrolled: still call `invalidate_all_sessions` and `create_session`, still set the `session` cookie, still return `200 OK` with the existing `LoginResponse` body.

## 2. Add JSON second-step endpoint

- [x] 2.1 Add `pub async fn login_totp(...)` in `src/api/handlers/auth.rs`. Inject `Arc<AuthService>`, `Arc<Settings>`, `Arc<TotpService>`, `Arc<PendingLoginService>`, `SqlitePool`, `HeaderMap`, `CookieJar`, and a `Json<LoginTotpRequest>` body (fields: `code: String`, optional `pending_token: Option<String>` for non-cookie clients).
- [x] 2.2 Read the pending token from the cookie (`crate::auth::pending_login::COOKIE_NAME`) or fall back to `req.pending_token`. If absent or `pending_login_service.find(token)` returns `None` / expired, return `AppError::Unauthorized` and clear any stale `pending_login` cookie.
- [x] 2.3 Look up the member by `pending.member_id`. Try `totp_service.verify_for_member(member.id, &req.code).await`; if false, try `totp_service.consume_recovery_code(member.id, &req.code).await`. If neither succeeds, return `AppError::Unauthorized` (the pending row stays so the client may retry until expiry).
- [x] 2.4 On success: `pending_login_service.consume(token)` atomically, `pending_login_service.delete_for_member(member.id)`, then run the same session-fixation invalidation + session create as the password-only handler. Return the session cookie AND a clear-`pending_login` cookie on the jar, plus `200 OK` with the existing `LoginResponse` body.
- [x] 2.5 Add route `.route("/auth/login/totp", post(handlers::auth::login_totp))` in `src/api/mod.rs::create_app`, next to the existing `/auth/login`.

## 3. Tests

- [x] 3.1 Add an integration test `tests/api_login_totp.rs::json_login_for_totp_enrolled_returns_202_no_session` that creates a member, enrolls them in TOTP, POSTs valid credentials to `/auth/login`, and asserts: response is `202`, no `session` cookie is set, response body contains `pending_token`, and `auth_service.list_active_sessions(member_id)` is empty.
- [x] 3.2 Add `tests/api_login_totp.rs::json_login_totp_with_valid_code_creates_session` that completes the flow: POST `/auth/login`, capture pending cookie, POST `/auth/login/totp` with a valid TOTP code, assert `200 OK`, a `session` cookie is set, and the `pending_login` row is gone.
- [x] 3.3 Add `tests/api_login_totp.rs::json_login_totp_with_wrong_code_returns_unauthorized` that asserts a wrong code yields `401`, no session is created, and the pending row is still consumable on a subsequent retry until it expires.
- [x] 3.4 Add `tests/api_login_totp.rs::json_login_no_totp_still_returns_200` to confirm members without TOTP enrolled keep the existing 1-step behavior.

## 4. Spec

- [x] 4.1 Update `openspec/specs/session-auth/spec.md` so the "Login uses 2FA branch when TOTP enrolled" requirement applies to BOTH `POST /login` (web) and `POST /auth/login` (JSON), with the JSON path returning `202 Accepted` + `pending_login` cookie instead of a redirect.
