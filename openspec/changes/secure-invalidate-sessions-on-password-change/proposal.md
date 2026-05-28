## Why

`POST /portal/profile/password` in `src/web/portal/profile.rs:108-193` updates the password hash via `member_repo.update_password_hash` but never calls `auth_service.invalidate_all_sessions(member.id)`. If a member changes their password specifically because they suspect their session was compromised (stolen cookie, abandoned laptop, etc.), the attacker's existing `session` row remains valid for its full TTL — defeating the user's defensive action.

The codebase already establishes this invariant elsewhere: `reset_password_handler` in `src/web/templates/reset.rs:266` calls `invalidate_all_sessions(consumed.member_id)` after a successful reset, and `password-management/spec.md:48-55` flags the in-portal change handler's omission as "a known gap noted as a potential follow-up." This change closes that gap.

Attacker / input: holder of a stolen session cookie for member M (e.g. exfiltrated via XSS-via-some-other-vector, malware on an abandoned device, shared-machine session pickup).

Harm: session takeover persists across the user's intentional credential rotation. The member believes the rotation revoked all access; in practice the attacker's session continues to work until natural expiry (24 h to 30 d depending on `remember_me`).

## What Changes

Update `update_password` in `src/web/portal/profile.rs` to call `auth_service.invalidate_all_sessions(current_user.member.id)` after a successful `update_password_hash`, then mint a fresh session for the current request (and update the `Set-Cookie` header) so the caller who just changed their password isn't immediately logged out. Mirror the `reset_password_handler` logging convention — best-effort on invalidate failure (the password DID change), but log loudly so an operator can react.

Update `password-management/spec.md` to MODIFY the "Password change does NOT currently invalidate other sessions" requirement: now it SHALL invalidate other sessions, preserving the caller's session via re-issuance.

## Impact

- `src/web/portal/profile.rs` — extend `update_password` to take `State(auth_service): State<Arc<AuthService>>` (the type already implements `FromRef<AppState>` per `src/api/state.rs:292`) and `Settings` (already extractable) for cookie secure-flag, then call `invalidate_all_sessions` + `create_session` + emit `Set-Cookie` on success.
- `openspec/specs/password-management/spec.md` — flip the "does NOT invalidate" requirement to a positive "SHALL invalidate other sessions and re-issue current session" rule.
- Tests: a new integration test that asserts (a) other sessions for the member are gone after a successful password change and (b) the current session cookie is still valid.
