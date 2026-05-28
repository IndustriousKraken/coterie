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

`crate::auth::validate_password` SHALL be invoked before hashing on every code path that sets a password (signup, in-portal change, reset). The validator's rules are the single source of truth for complexity.

#### Scenario: Weak password rejected at change

- **WHEN** a member submits the password-change form with a password failing complexity rules
- **THEN** the handler SHALL render an inline error and SHALL NOT update the hash

#### Scenario: Weak password rejected at reset

- **WHEN** a reset-token consumer submits a password failing complexity rules
- **THEN** the handler SHALL reject the submission and the token SHALL NOT be marked consumed

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

1. `POST /forgot-password` rate-limits via `login_limiter`, then issues a single-use email token bound to the member.
2. `GET /reset-password?token=...` validates the token and renders the form.
3. `POST /reset-password` consumes the token, hashes the new password, updates the stored hash, AND calls `invalidate_all_sessions(member_id)` to log the member out everywhere.

#### Scenario: Reset invalidates all sessions for the member

- **WHEN** a reset is completed successfully
- **THEN** `invalidate_all_sessions(member_id)` SHALL be called so any pre-existing sessions (the attacker's, on assumption they had one) are invalidated

#### Scenario: Token cannot be redeemed twice

- **WHEN** a reset token is consumed by setting a new password
- **THEN** a second redemption attempt with the same token SHALL fail (single-use)

#### Scenario: Reset request is rate-limited

- **WHEN** an IP exceeds the `login_limiter` budget
- **THEN** further `/forgot-password` requests SHALL be rejected before any token issuance

