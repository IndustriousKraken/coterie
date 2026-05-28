## 1. Invalidate other sessions and re-issue the caller's session on password change

- [x] 1.1 In `src/web/portal/profile.rs`, change `update_password`'s extractors to also pull `State(auth_service): State<Arc<AuthService>>`, `State(settings): State<Arc<Settings>>`, and a `CookieJar` so the handler can mint a fresh session and set the cookie on the response.
- [x] 1.2 After a successful `member_repo.update_password_hash(...)`, call `auth_service.invalidate_all_sessions(current_user.member.id).await`. Match the `reset_password_handler` convention in `src/web/templates/reset.rs:266-276`: log the failure at error level but still continue (the password change DID succeed; failing the response would hide that).
- [x] 1.3 Immediately after the invalidation, call `auth_service.create_session(current_user.member.id, 24).await?` and attach the new session cookie to the response so the caller stays logged in on this device. Use `auth_service.create_session_cookie(&token, settings.server.cookies_are_secure())` (same helper used by `src/api/handlers/auth.rs::login`).
- [x] 1.4 Audit-log the change with `audit_service.log(Some(member.id), "password_change", "member", &member.id.to_string(), None, Some("via portal"), None).await`; the audit_service is already on `AppState` and extractable. Mirror the existing "logout" entry shape from `src/web/templates/auth.rs:408-418` for consistency.

## 2. Spec update

- [x] 2.1 In `openspec/specs/password-management/spec.md`, replace the "Password change does NOT currently invalidate other sessions" requirement (the one whose body explicitly disclaims the security best practice) with a positive "Password change invalidates all other sessions and re-issues the caller's session" requirement. Keep two scenarios: (a) other-device sessions for the member are gone after a successful change, and (b) the caller's session cookie remains valid because a fresh one was issued.

## 3. Tests

- [x] 3.1 Add an integration test `password_change_invalidates_other_sessions` under `tests/`: create a member, mint two sessions (A and B) via `auth_service.create_session`, POST `/portal/profile/password` carrying session A's cookie with a valid current+new password, assert that session B's token no longer validates and session A's NEW token (from the response `Set-Cookie`) does validate.
- [x] 3.2 Add a negative test `password_change_with_wrong_current_does_not_touch_sessions`: confirm both sessions remain valid when the `current_password` field is wrong (no invalidate sweep on a rejected change).
