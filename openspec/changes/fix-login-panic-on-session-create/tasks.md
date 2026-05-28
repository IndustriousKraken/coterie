## 1. Replace the panic with a clean 500

- [ ] 1.1 In `src/web/templates/auth.rs::login_handler` around lines 285-295, replace `auth_service.create_session(member.id, ...).await.unwrap()` with a `match`. On `Ok((session, token))` continue with the existing cookie-build code; on `Err(e)`, log via `tracing::error!("Failed to create session after password verify: {}", e)` and return `(StatusCode::INTERNAL_SERVER_ERROR, Json(LoginResponse { success: false, redirect: None, error: Some("Login failed. Please try again.".to_string()) })).into_response()`. Use the same response shape that the pending-login-mint path at lines 218-229 already uses, so the frontend's existing error-handling for `/login` covers it.
- [ ] 1.2 Verify there is no other `.unwrap()` on a fallible `auth_service` / `member_repo` / `csrf_service` call inside `login_handler` or `login_totp_handler`. If found, replace with the same pattern.

## 2. Spec update

- [ ] 2.1 In `openspec/specs/session-auth/spec.md`, add a scenario under the "Sessions are server-side records with hashed tokens" requirement (or the "Login lookup by email" requirement, whichever is the better home): when `create_session` errors during a non-TOTP login, the handler SHALL respond `500 Internal Server Error` with the generic login-failed body — it SHALL NOT panic and SHALL NOT set a session cookie.

## 3. Test

- [ ] 3.1 Add a unit/integration test `login_handler_returns_500_when_session_create_fails`. Drive the handler with a member whose password verifies but whose `auth_service.create_session` returns `Err` (e.g. by closing the pool the `SessionStore` holds, or by injecting a faked AuthService). Assert the HTTP status is `500`, the JSON body matches `LoginResponse { success: false, error: Some(_), .. }`, and NO `Set-Cookie: session=...` header was emitted.
