# member-profile Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Members manage profile, password, and 2FA

The member portal SHALL provide:
- `GET /portal/profile` — view profile.
- `POST /portal/profile` — update profile fields the member is allowed to change.
- `POST /portal/profile/password` — change password (requires current password).
- `GET /portal/profile/security` — security page (TOTP enrollment, recovery codes).
- `POST /portal/profile/security/totp/enroll/start` — begin TOTP enrollment.
- `POST /portal/profile/security/totp/enroll/confirm` — confirm with code.
- `POST /portal/profile/security/totp/disable` — disable TOTP.
- `POST /portal/profile/security/totp/recovery-codes/regenerate` — regenerate codes.

All routes SHALL require Active/Honorary status via `require_auth_redirect`.

#### Scenario: Profile update accepts only full_name today

- **WHEN** a member submits the profile-update form
- **THEN** the handler SHALL persist only `full_name` via `member_repo.update`; other fields in the body SHALL be ignored

#### Scenario: Profile update is NOT currently audited

- **WHEN** a member updates their full_name from `/portal/profile`
- **THEN** no `audit_logs` row SHALL be written today. (This is a known gap noted as a potential follow-up; the spec captures observed behavior.)

#### Scenario: Member cannot update admin-only fields

- **WHEN** a member submits a profile update with extra fields in the body (e.g., `is_admin`, `status`)
- **THEN** the handler SHALL ignore them because the construction of `UpdateMemberRequest` populates only `full_name`

