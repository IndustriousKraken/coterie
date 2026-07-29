# member-profile Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Members manage profile, password, and 2FA

The member portal SHALL provide:
- `GET /portal/profile` — view profile.
- `POST /portal/profile` — update profile fields the member is allowed to change.
- `POST /portal/profile/custom-fields` — update the member's own values for active, member-editable org-defined fields (see the member-custom-fields capability; audited as `update_member_fields`).
- `POST /portal/profile/password` — change password (requires current password).
- `GET /portal/profile/security` — security page (TOTP enrollment, recovery codes).
- `POST /portal/profile/security/totp/enroll/start` — begin TOTP enrollment.
- `POST /portal/profile/security/totp/enroll/confirm` — confirm with code.
- `POST /portal/profile/security/totp/disable` — disable TOTP.
- `POST /portal/profile/security/totp/recovery-codes/regenerate` — regenerate codes.

All routes SHALL require Active/Honorary status via `require_auth_redirect`.

#### Scenario: Profile update accepts only full_name today

- **WHEN** a member submits the profile-update form (`POST /portal/profile`)
- **THEN** the handler SHALL persist only `full_name` via `member_repo.update`; other fields in the body SHALL be ignored. (Org-defined custom fields are saved through the separate `/portal/profile/custom-fields` endpoint, not this form.)

#### Scenario: Profile update is NOT currently audited

- **WHEN** a member updates their full_name from `/portal/profile`
- **THEN** no `audit_logs` row SHALL be written today. (This is a known gap noted as a potential follow-up; the spec captures observed behavior. The custom-fields endpoint, by contrast, audits its saves.)

#### Scenario: Member cannot update admin-only fields

- **WHEN** a member submits a profile update with extra fields in the body (e.g., `is_admin`, `status`)
- **THEN** the handler SHALL ignore them because the construction of `UpdateMemberRequest` populates only `full_name`

### Requirement: Profile update bounds its free-text input

`POST /portal/profile` SHALL validate and length-bound `full_name` before
persisting it, so an authenticated member cannot write unbounded data into
`members.full_name`. The handler SHALL reject the request with an inline error
and SHALL NOT call `member_repo.update` when, checked against the trimmed value,
`full_name` is empty or longer than 200 characters. The trimmed value is what
SHALL be persisted.

This bound matches the one `POST /public/signup` already applies to the same
field (see the `public-signup` capability's requirement that signup bounds and
validates its input fields), so the two entry points that write
`members.full_name` cannot drift apart. The bound matters here because
`full_name` is rendered on the admin member list, on every event and class
roster, in the member CSV export, and in outbound email — surfaces a single
oversized row would degrade for everyone.

#### Scenario: An over-long name is rejected

- **WHEN** a member submits `POST /portal/profile` with a `full_name` longer than
  200 characters
- **THEN** the handler SHALL return its inline error fragment naming the limit,
  and the member's stored `full_name` SHALL be unchanged

#### Scenario: A blank name is rejected

- **WHEN** a member submits `POST /portal/profile` with a `full_name` that is
  empty or whitespace-only
- **THEN** the handler SHALL return its inline error fragment, and the member's
  stored `full_name` SHALL be unchanged

#### Scenario: A valid name is trimmed and saved

- **WHEN** a member submits `POST /portal/profile` with a `full_name` of
  `"  Ada Lovelace  "`
- **THEN** the update SHALL succeed and the stored `full_name` SHALL be exactly
  `"Ada Lovelace"`

