# session-auth Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Sessions are server-side records with hashed tokens

The system SHALL identify users via a server-side `sessions` row keyed by a cryptographically random token. The cookie SHALL carry the plaintext token; the database SHALL store ONLY a SHA-256 hash of the token. Validation SHALL hash the cookie value and look up the row.

#### Scenario: Login creates a session row with hashed token

- **WHEN** a member submits valid credentials
- **THEN** a `sessions` row SHALL be inserted with `token_hash = SHA-256(token)`, the cookie SHALL receive the plaintext token, and the row SHALL include `expires_at`, `created_at`, `last_used_at`

#### Scenario: Database compromise does not yield session tokens

- **WHEN** an attacker reads the `sessions` table
- **THEN** they SHALL find only token hashes; they CANNOT impersonate users without also obtaining cookies in transit

#### Scenario: Session validation updates last_used_at

- **WHEN** a session is successfully validated
- **THEN** the row's `last_used_at` SHALL be updated to the current time before returning the session

### Requirement: Login rejects Pending and Suspended; allows Expired through

`POST /auth/login` SHALL allow members with status `Active`, `Honorary`, or `Expired` to log in. `Pending` and `Suspended` members SHALL be rejected. Expired members SHALL log in so they can reach the restoration flow.

#### Scenario: Suspended member cannot log in

- **WHEN** a Suspended member submits valid credentials
- **THEN** login SHALL return `Forbidden` and no session SHALL be created

#### Scenario: Pending member cannot log in until verified

- **WHEN** a Pending (unverified) member submits valid credentials
- **THEN** login SHALL return `Forbidden`

#### Scenario: Expired member logs in to reach restoration

- **WHEN** an Expired member submits valid credentials
- **THEN** login SHALL succeed; the redirect logic in `require_auth_redirect` SHALL bounce them to `/portal/restore` on subsequent navigation

### Requirement: Login burns Argon2 time on unknown user

When the looked-up email/username does not exist, `POST /auth/login` SHALL still execute Argon2 work (via `verify_dummy`) so the response timing does not distinguish "user does not exist" from "wrong password."

#### Scenario: Unknown email runs dummy hash

- **WHEN** a login request specifies an email with no matching row
- **THEN** the handler SHALL call `verify_dummy` to consume Argon2 time before returning `Unauthorized`

### Requirement: Login invalidates pre-existing sessions for the member

To defend against session fixation, `POST /auth/login` SHALL invalidate ALL existing sessions for the authenticating member before creating a new one.

#### Scenario: Stale sessions on other devices are dropped

- **WHEN** a member with active sessions on Device A and Device B logs in from Device C
- **THEN** the sessions on Device A and Device B SHALL be invalidated and the new Device C session SHALL be the only valid one

### Requirement: Login lookup by email (api) and username-or-email (web)

`POST /auth/login` (api JSON handler in `src/api/handlers/auth.rs`) SHALL look up the member by email only. `POST /login` (web template handler in `src/web/templates/auth.rs`) SHALL look up by username first, falling back to email. Both handlers SHALL apply rate limiting before any database lookup.

#### Scenario: API handler accepts email only

- **WHEN** a JSON request to `/auth/login` carries `email`
- **THEN** the handler SHALL look up by `email` and SHALL NOT attempt a username lookup

#### Scenario: Web handler accepts username OR email

- **WHEN** a JSON request to `/login` (the web template handler) carries a `username` field
- **THEN** the handler SHALL try `find_by_username` first and fall back to `find_by_email` if username yields nothing

### Requirement: Login uses 2FA branch when TOTP enrolled

When the authenticating member has TOTP enrolled, BOTH login surfaces SHALL refuse to issue a session at the password-only step and SHALL instead defer session creation to a second-factor step:

- `POST /login` (web handler in `src/web/templates/auth.rs::login_handler`) SHALL mint a short-lived `pending_login` row via `PendingLoginService`, set the `pending_login` cookie, and respond with a redirect to `/login/totp` (preserving any requested redirect as a query param).
- `POST /auth/login` (JSON handler in `src/api/handlers/auth.rs::login`) SHALL mint a short-lived `pending_login` row via `PendingLoginService`, set the `pending_login` cookie, AND include the `pending_token` value in the JSON response body (so non-cookie JSON clients can carry it). It SHALL respond with `202 Accepted` and a body indicating a second factor is required (e.g. `{"message": "2fa_required", "pending_token": "<token>"}`). It SHALL NOT call `invalidate_all_sessions`, SHALL NOT call `create_session`, and SHALL NOT set the `session` cookie.

The TOTP-enrollment check (`TotpService::is_enabled`) at the password-only step SHALL fail closed: if the underlying query errors, the handler SHALL propagate the error (surfacing as `500 Internal Server Error`) rather than treating the member as not-enrolled and issuing a session. A transient failure of the enrollment check SHALL NOT bypass the 2FA branch.

In both flows, session-fixation invalidation (clearing pre-existing sessions for the member) SHALL be deferred to the second-factor step — clearing pre-existing sessions on password-only success would let an attacker who guessed the password log the victim out at will.

The second-factor endpoints — `POST /login/totp` (web) and `POST /auth/login/totp` (JSON) — SHALL each accept the `pending_login` token (cookie or, for the JSON surface, a body field), verify either a TOTP code or a recovery code, atomically consume the pending row, invalidate pre-existing sessions for the member, create a new session, and clear the `pending_login` cookie.

#### Scenario: TOTP-enrolled member is sent to /login/totp (web)

- **WHEN** a TOTP-enrolled member submits valid credentials to `POST /login`
- **THEN** the response SHALL set a `pending_login` cookie and redirect to `/login/totp` with the requested redirect preserved as a query param

#### Scenario: TOTP-enrolled member receives 202 on JSON login

- **WHEN** a TOTP-enrolled member submits valid credentials to `POST /auth/login` (JSON)
- **THEN** the response status SHALL be `202 Accepted`, the response SHALL set a `pending_login` cookie, the response body SHALL include a `pending_token` field, and NO `session` cookie SHALL be set

#### Scenario: Session is not created at password-only step for TOTP-enrolled member

- **WHEN** the password-only step succeeds for a TOTP-enrolled member on EITHER `/login` or `/auth/login`
- **THEN** no `sessions` row SHALL be inserted at this step; a row SHALL be inserted only after the matching `/login/totp` or `/auth/login/totp` endpoint verifies the code

#### Scenario: JSON second-factor endpoint creates the session

- **WHEN** a client posts a valid TOTP or recovery code to `POST /auth/login/totp` carrying the `pending_login` cookie or `pending_token` body field minted by `/auth/login`
- **THEN** the handler SHALL consume the pending row, invalidate pre-existing sessions for the member, create a new session, set the `session` cookie, clear the `pending_login` cookie, and respond `200 OK`

#### Scenario: Pre-existing sessions are not cleared at the password-only step

- **WHEN** a TOTP-enrolled member completes only the password step on EITHER surface
- **THEN** that member's existing sessions on other devices SHALL remain valid; they SHALL be invalidated only after the second factor verifies

#### Scenario: TOTP enrollment check failure surfaces as 500, not a 2FA bypass

- **WHEN** `TotpService::is_enabled` returns an error during the password-only step of EITHER `/login` or `/auth/login` for a member whose password just verified
- **THEN** the handler SHALL respond `500 Internal Server Error`; no session cookie SHALL be set; no `pending_login` row SHALL be created; the error SHALL be logged at error level via `tracing` with sufficient context for an operator to diagnose

### Requirement: Session cookie attributes

A session cookie SHALL be set with `HttpOnly`, `SameSite=Lax`, and `Secure` (when `cookies_are_secure() = true`). The cookie name SHALL be `session`.

#### Scenario: Cookie carries HttpOnly + SameSite=Lax

- **WHEN** any login flow issues a session cookie
- **THEN** the `Set-Cookie` header SHALL include `HttpOnly` and `SameSite=Lax`

### Requirement: Logout deletes the session row and clears the cookie

`POST /auth/logout` (and `POST /logout` web equivalent) SHALL delete the session row, log a `logout` audit-log entry, and respond with a cookie that clears the `session` cookie. Logout SHALL be CSRF-protected (it is NOT in `CSRF_EXEMPT_PATHS`).

#### Scenario: Logout removes the row and audits the action

- **WHEN** an authenticated user POSTs to `/auth/logout`
- **THEN** the row SHALL be deleted via `invalidate_session`, an audit-log row SHALL be written with action `logout`, and the response SHALL clear the cookie

#### Scenario: Logout without CSRF token is rejected

- **WHEN** a logout POST arrives without a valid CSRF token
- **THEN** the top-level CSRF middleware SHALL reject it with 403 before the handler runs

### Requirement: Session expiry is enforced on lookup

A session whose `expires_at` is in the past SHALL be treated as not present. A background `cleanup_expired` job SHALL prune stale rows; lookup MUST treat expired rows as invalid regardless of pruning.

#### Scenario: Expired token is rejected

- **WHEN** a request presents a session token whose row has `expires_at <= now()`
- **THEN** the lookup SHALL return `None` and downstream middleware SHALL treat the request as anonymous

### Requirement: Login surfaces a clean 500 if session creation fails

Both `POST /login` (web handler in `src/web/templates/auth.rs::login_handler`) and `POST /auth/login` (JSON handler in `src/api/handlers/auth.rs::login`) SHALL handle a `create_session` error as a `500 Internal Server Error` with the generic login-failed body. Neither handler SHALL panic on the error path: `unwrap()` / `expect()` on the `Result` returned by `AuthService::create_session` is forbidden. The error SHALL be logged at error level via `tracing` with enough context for an operator to correlate with the underlying database failure. No session cookie SHALL be set on the response.

This is a defense against panic-based DoS: the password is by definition attacker-supplied, and the underlying `sessions` INSERT can fail under DB contention or a transient SQLite `database is locked`. Returning 500 lets the caller retry; panicking drops the connection and leaves no actionable trail.

#### Scenario: Web login returns 500 on session-create failure

- **WHEN** `login_handler` (web) verifies a member's password successfully but `auth_service.create_session(...)` returns `Err`
- **THEN** the handler SHALL respond `500 Internal Server Error` with a `LoginResponse { success: false, error: Some("Login failed. Please try again."), redirect: None }` body, SHALL log the underlying error via `tracing::error!`, and SHALL NOT emit a `Set-Cookie: session=...` header

#### Scenario: JSON login returns 500 on session-create failure

- **WHEN** `login` (JSON, in `src/api/handlers/auth.rs`) verifies a member's password successfully but `auth_service.create_session(...)` returns `Err`
- **THEN** the handler SHALL surface the error via `?` (or an equivalent match) so the framework's `AppError` mapping produces a `500 Internal Server Error` response; no session cookie SHALL be set

