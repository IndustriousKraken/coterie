## Why

The JSON login handler at `src/api/handlers/auth.rs:31-100` (function `login`) authenticates a member with email + password and then unconditionally calls `auth_service.create_session(...)` and sets the session cookie, with NO check for TOTP enrollment.

The web-form login handler at `src/web/templates/auth.rs:100-237` correctly handles this case: after password verification it calls `totp_service.is_enabled(member_id)` and, if 2FA is enrolled, mints a short-lived `pending_login` cookie and redirects to `/login/totp` instead of issuing a session. The `session-auth` spec at `openspec/specs/session-auth/spec.md:76-88` codifies the 2FA branch but scopes it only to the web handler — leaving the JSON API as a documented hole.

Concrete harm: any caller in possession of a member's email + password — phished password, credential dump, compromised browser store — can POST `{"email": "...", "password": "..."}` to `/auth/login` and receive a fully-authorized session cookie, bypassing the TOTP second factor the member enrolled in. The `auth.require_totp_for_admins` admin redirect does not close the hole because it only checks TOTP enrollment, not whether the second factor was verified for this session — once `/auth/login` mints a session for a TOTP-enrolled admin, that session passes the admin gate.

## What Changes

- Make `src/api/handlers/auth.rs::login` mirror the web handler's TOTP branch: after a successful password check, call `totp_service.is_enabled(member.id)`; if enrolled, mint a `pending_login` token, return it as a cookie (and include the token in the JSON body so non-cookie clients can carry it forward), respond with `202 Accepted` and a body indicating a second factor is required, and do NOT create a session or invalidate existing sessions.
- Add a new JSON endpoint `POST /auth/login/totp` that reads the `pending_login` cookie (or `pending_token` body field), accepts a TOTP code or recovery code, performs the same verification + recovery-code-consume + session-fixation invalidation + session create that the web `/login/totp` handler does, and returns the session cookie on success.
- Update the `session-auth` capability spec so the "Login uses 2FA branch when TOTP enrolled" requirement covers BOTH login surfaces (web AND JSON), and add a scenario covering the JSON path.

## Impact

- `src/api/handlers/auth.rs` — extend `login`, add `login_totp` handler, add request/response types.
- `src/api/mod.rs` — add `POST /auth/login/totp` route.
- `openspec/specs/session-auth/spec.md` — modified by the change's `specs/session-auth/spec.md` delta in this proposal.
- Tests under `tests/` — new integration tests for the JSON 2FA branch (enrolled user gets 202 + no session) and the second-step handler.
