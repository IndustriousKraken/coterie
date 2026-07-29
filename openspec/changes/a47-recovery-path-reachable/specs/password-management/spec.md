# password-management Specification

## MODIFIED Requirements

### Requirement: Password reset uses single-use email tokens and DOES invalidate all sessions

The reset flow SHALL be:

1. `POST /forgot-password` rate-limits via the dedicated `recovery_limiter` (NOT `login_limiter` — see the `rate-limiting` capability), then issues a single-use email token bound to the member.
2. `GET /reset-password?token=...` validates the token and renders the form.
3. `POST /reset-password` consumes the token, hashes the new password, updates the stored hash, AND calls `invalidate_all_sessions(member_id)` to log the member out everywhere.

`POST /reset-password` SHALL return a status code that reflects the outcome. A reset that did not change the password — invalid token, expired token, already-consumed token, or a new password failing the complexity rules — SHALL NOT return `200`. Only a reset that actually updated the stored hash SHALL return a success status.

The rendered page SHALL continue to say exactly what it says today; this requirement constrains the status code, not the body, and SHALL NOT be used to make the response more revealing to an anonymous caller than it already is.

The reason is diagnosability. Returning `200` for a refused reset makes success and failure indistinguishable in every log — the application's own, the reverse proxy's, and any uptime monitor — so neither the member nor the operator can tell whether a password was changed. In the 2026-07-29 incident a member submitted `POST /reset-password` five times, received `200` every time, and still could not sign in; the logs could not answer why, because a refusal and a success looked identical.

#### Scenario: Reset invalidates all sessions for the member

- **WHEN** a reset is completed successfully
- **THEN** `invalidate_all_sessions(member_id)` SHALL be called so any pre-existing sessions (the attacker's, on assumption they had one) are invalidated

#### Scenario: Token cannot be redeemed twice

- **WHEN** a reset token is consumed by setting a new password
- **THEN** a second redemption attempt with the same token SHALL fail (single-use)

#### Scenario: Reset request is rate-limited

- **WHEN** an IP exceeds the `recovery_limiter` budget
- **THEN** further `/forgot-password` requests SHALL be rejected before any token issuance

#### Scenario: A refused reset is distinguishable from a successful one in the logs

- **WHEN** `POST /reset-password` is submitted with an already-consumed token, or with a new password that fails the complexity rules
- **THEN** the response status SHALL NOT be `200`, so the refusal is distinguishable from a successful reset in the application and proxy logs

#### Scenario: A successful reset still returns success

- **WHEN** `POST /reset-password` consumes a valid token and updates the stored hash
- **THEN** the response SHALL carry a success status and the member SHALL be able to sign in with the new password
