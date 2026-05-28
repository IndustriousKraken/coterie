# password-management Specification Delta

## MODIFIED Requirements

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
