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

When the authenticating member has TOTP enrolled, `/login` (web handler) SHALL NOT issue a session. Instead it SHALL mint a short-lived `pending_login` cookie and redirect to `/login/totp` for the second factor. Session-fixation invalidation SHALL be deferred to `/login/totp` (clearing pre-existing sessions on password-only success would let an attacker who guessed the password log the victim out at will).

#### Scenario: TOTP-enrolled member is sent to /login/totp

- **WHEN** a TOTP-enrolled member submits valid credentials to `/login`
- **THEN** the response SHALL set a `pending_login` cookie and redirect to `/login/totp` with the requested redirect preserved as a query param

#### Scenario: Session is not created until second factor

- **WHEN** the password-only step succeeds for a TOTP-enrolled member
- **THEN** no `sessions` row SHALL be inserted at this step; the row SHALL be inserted only after `/login/totp` verifies the code

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

