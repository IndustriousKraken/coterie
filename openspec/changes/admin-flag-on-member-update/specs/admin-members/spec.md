# admin-members Specification

## ADDED Requirements

### Requirement: Admin flag is granted and revoked via an explicit update-form control

The member update form at `/portal/admin/members/:id` SHALL carry an explicit Administrator checkbox bound to the member's `is_admin` flag, routed through `MemberService::update` like every other admin-driven member mutation. Free-text content (the notes field included) SHALL NOT affect adminness. A grant or revoke SHALL write a dedicated audit entry (`grant_admin` / `revoke_admin`) with the old and new values; an unchanged flag SHALL write none. Revoking SHALL be rejected while the target is the only administrator — a zero-admin database locks all operators out and re-arms the unauthenticated `/setup` page on restart. The flag takes effect on the member's next request (the auth middleware reads `is_admin` per request); no re-login is required.

#### Scenario: Granting admin via the update form

- **WHEN** an admin submits the member update form with the Administrator checkbox checked for a non-admin member
- **THEN** the member's `is_admin` SHALL be set, a `grant_admin` audit entry SHALL be written, and the member SHALL see the admin portal on their next request without re-logging-in

#### Scenario: Revoking the last administrator is rejected

- **WHEN** the update form is submitted with the Administrator checkbox unchecked for the only member with `is_admin` set
- **THEN** the update SHALL be rejected with an explanatory error and the flag SHALL remain set

#### Scenario: Notes text never affects adminness

- **WHEN** a member's notes are saved containing the string "ADMIN"
- **THEN** the member's `is_admin` flag SHALL be unchanged — no free-text mechanism grants privileges
