## MODIFIED Requirements

### Requirement: Login uses 2FA branch when TOTP enrolled

When the authenticating member has TOTP enrolled, BOTH login surfaces SHALL refuse to issue a session at the password-only step and SHALL instead defer session creation to a second-factor step:

- `POST /login` (web handler in `src/web/templates/auth.rs::login_handler`) SHALL mint a short-lived `pending_login` row via `PendingLoginService`, set the `pending_login` cookie, and respond with a redirect to `/login/totp` (preserving any requested redirect as a query param).
- `POST /auth/login` (JSON handler in `src/api/handlers/auth.rs::login`) SHALL mint a short-lived `pending_login` row via `PendingLoginService`, set the `pending_login` cookie, AND include the `pending_token` value in the JSON response body (so non-cookie JSON clients can carry it). It SHALL respond with `202 Accepted` and a body indicating a second factor is required (e.g. `{"message": "2fa_required", "pending_token": "<token>"}`). It SHALL NOT call `invalidate_all_sessions`, SHALL NOT call `create_session`, and SHALL NOT set the `session` cookie.

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
