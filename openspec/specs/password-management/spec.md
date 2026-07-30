# password-management Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Passwords are hashed with Argon2

The system SHALL hash passwords with Argon2 (default parameters from the `argon2` crate) using a per-password random salt. The plaintext password SHALL never be persisted.

#### Scenario: Hash includes salt and parameters

- **WHEN** a password is hashed (via `AuthService::hash_password` or directly with `Argon2::default().hash_password`)
- **THEN** the resulting string SHALL be in PHC format (e.g., `$argon2id$...`) embedding salt and parameters

#### Scenario: Verification uses the embedded parameters

- **WHEN** an existing hash is checked
- **THEN** verification SHALL parse the parameters from the stored hash, not assume defaults

### Requirement: Password complexity is validated at change/reset/signup

`crate::auth::validate_password` SHALL be invoked before hashing on every code path that sets a password (signup, in-portal change, reset, setup). The validator's rules are the single source of truth for complexity. The validator SHALL enforce both a minimum length AND a maximum length: a password longer than the upper bound (128 **bytes**) SHALL be rejected before it is Argon2-hashed, so an unauthenticated caller cannot force expensive hashing of an oversized input (an Argon2 CPU-amplification denial of service).

The bound SHALL be denominated in **bytes**, and every message describing it SHALL say bytes rather than characters. The check measures UTF-8 byte length, which is the quantity the denial-of-service argument is actually about — Argon2's pre-hash cost scales with bytes fed to it, not with Unicode scalar values. Describing a byte limit as a character limit is not a harmless simplification: a 60-character password made of emoji is 240 bytes, so a user is told they exceeded "128 characters" when they typed 60. Someone hitting that message reasonably concludes the system is broken.

An over-length rejection SHALL state both the ceiling and the size of what was submitted, so the user can act on it rather than guess how much to remove. The same rule SHALL apply to the minimum-length message, which carries the same ambiguity.

The password SHALL NOT be silently truncated to fit. Truncation would leave the account with a credential that is a prefix of what the user believes they set — indistinguishable, from the user's side, from the account being broken.

Password inputs SHALL carry a visible hint stating the ceiling, so it is discoverable before submission rather than only by tripping it. The hint is a convenience only: the server-side check remains authoritative and SHALL NOT be weakened or skipped on the assumption that the browser enforced it.

Password inputs SHALL NOT carry a `maxlength` attribute. Two independent reasons:

1. **It silently truncates.** A browser clips pasted input to `maxlength` without notifying the user. Someone pasting a 200-character password from a password manager would have 128 characters accepted and stored, believe they set the longer one, and be unable to sign in afterwards — reintroducing at the client exactly the silent-truncation failure this requirement forbids at the server. For a field whose contents are masked, the user cannot even see that it happened.
2. **The units do not match.** `maxlength` counts UTF-16 code units, while the bound is bytes. `maxlength="128"` would admit 128 characters, which can be several hundred bytes, so it cannot express the server's rule even approximately for non-ASCII input.

Client-side feedback, where provided, SHALL be non-destructive: it MAY warn that the entered value exceeds the ceiling — measuring UTF-8 byte length, e.g. via `TextEncoder`, to match the server — but SHALL NOT alter, clip, or reject the user's input. The server's rejection message is what tells the user authoritatively.

#### Scenario: Weak password rejected at change

- **WHEN** a member submits the password-change form with a password failing complexity rules
- **THEN** the handler SHALL render an inline error and SHALL NOT update the hash

#### Scenario: Weak password rejected at reset

- **WHEN** a reset-token consumer submits a password failing complexity rules
- **THEN** the handler SHALL reject the submission and the token SHALL NOT be marked consumed

#### Scenario: Over-long password rejected before hashing

- **WHEN** a password exceeding the maximum length (128 bytes) is submitted on any set-password path (signup, reset, in-portal change, setup)
- **THEN** `validate_password` SHALL return an error and the password SHALL NOT be Argon2-hashed or persisted

#### Scenario: A multi-byte password is measured and described in the same unit

- **WHEN** a user submits a password of 60 emoji characters, which is 240 UTF-8 bytes
- **THEN** it SHALL be rejected, and the message SHALL describe the limit in bytes and report the submitted byte size; it SHALL NOT claim the user exceeded a 128-**character** limit

#### Scenario: An over-long password is never truncated to fit

- **WHEN** a password longer than the bound is submitted on any set-password path
- **THEN** the submission SHALL be refused outright; no prefix of it SHALL be hashed or stored, and the account's existing credential SHALL be left unchanged

#### Scenario: The ceiling is discoverable before submission

- **WHEN** a user focuses a password field on any set-password form
- **THEN** a visible hint SHALL state the ceiling, so the limit is known before the form is submitted

#### Scenario: A pasted over-length password is not silently clipped

- **WHEN** a user pastes a 200-character password into any password field
- **THEN** the full value SHALL remain in the field and be submitted as typed; no `maxlength` SHALL clip it to the bound, because a masked field gives the user no way to notice the loss and they would be locked out by a credential they never chose

#### Scenario: Client-side feedback measures the same unit as the server

- **WHEN** a client-side warning about length is shown
- **THEN** it SHALL measure UTF-8 byte length so it agrees with the server's rule, and SHALL NOT modify the entered value

### Requirement: Password change requires the current password

`POST /portal/profile/password` SHALL require the member to provide the current password. The handler SHALL look up the stored hash by the member's email, verify with `AuthService::verify_password`, and reject the change on mismatch.

#### Scenario: Wrong current password is rejected

- **WHEN** a member submits the password-change form with an incorrect current password
- **THEN** the handler SHALL render "Current password is incorrect" and the stored hash SHALL be unchanged

#### Scenario: New + confirm must match

- **WHEN** the new and confirm fields differ
- **THEN** the handler SHALL render an inline error before any password validation

### Requirement: Password change invalidates all other sessions and re-issues the caller's session

`POST /portal/profile/password` SHALL update the stored hash via `member_repo.update_password_hash` AND SHALL call `auth_service.invalidate_all_sessions(member_id)` so any other active sessions for the member (potentially an attacker's) are terminated. To keep the caller signed in on the device they just changed their password from, the handler SHALL then call `auth_service.create_session(member_id, 24)` and emit a fresh session cookie on the response. The handler SHALL also write an audit-log entry with action `password_change`.

If `invalidate_all_sessions` errors, the handler SHALL log the failure at error level via `tracing` but still report success to the caller — the password DID change, and surfacing a generic failure would hide a successful credential rotation. The new session cookie SHALL still be issued.

This replaces the prior requirement that explicitly disclaimed session invalidation on in-portal password change; the in-portal change now matches the password-reset flow.

#### Scenario: Other-device session is invalidated after password change

- **WHEN** a member with active sessions on Device A and Device B changes their password from Device A
- **THEN** the session on Device B SHALL be invalidated immediately (the next request from Device B SHALL be treated as unauthenticated)

#### Scenario: Caller's device stays logged in via a fresh session

- **WHEN** Device A completes a successful password change
- **THEN** Device A's response SHALL carry a `Set-Cookie` for a NEW `session` token that validates on the next request; the cookie issued before the password change SHALL no longer validate

#### Scenario: Rejected password change does not invalidate sessions

- **WHEN** the password-change handler rejects the submission (wrong current password, new/confirm mismatch, complexity violation)
- **THEN** NO sessions SHALL be invalidated and NO new session cookie SHALL be issued

#### Scenario: Audit log records the password change

- **WHEN** a password change succeeds
- **THEN** the audit log SHALL contain an entry with `action = "password_change"`, `entity_type = "member"`, `entity_id = <member uuid>`, and `actor_id = <member uuid>`

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

