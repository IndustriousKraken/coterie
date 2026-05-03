# admin-settings Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Settings page lists name/value pairs and allows updates

`GET /portal/admin/settings` SHALL render a settings page; `POST /portal/admin/settings` SHALL update one or more settings. The handler SHALL call `settings_service.update_setting(...)` to persist, then SHALL call `audit_service.log` directly with the old + new values.

#### Scenario: Setting update is audited from the handler

- **WHEN** an admin changes the value of `auth.require_totp_for_admins`
- **THEN** the handler SHALL invoke `settings_service.update_setting` to persist, AND SHALL call `audit_service.log` recording the actor, key, old value, and new value. (`SettingsService` itself does NOT emit audit entries; the handler does.)

### Requirement: Setting changes take effect without restart

Settings consumed at request time SHALL be re-read on each request (e.g., `auth.require_totp_for_admins` in `require_admin_redirect`). Settings consumed at startup are documented as such.

#### Scenario: TOTP-for-admins toggle takes effect immediately

- **WHEN** an admin flips `auth.require_totp_for_admins` from `false` to `true`
- **THEN** the next request to an admin route by an admin without TOTP SHALL be redirected to the security page

### Requirement: Setting lookup failures default to safe behavior

When a setting lookup fails (row missing or read error), consumers SHALL fall back to a safe default rather than 500. Specifically, `auth.require_totp_for_admins` SHALL default to "not enforced" so a misconfigured setting cannot lock all admins out.

#### Scenario: Missing setting row does not lock admins out

- **WHEN** the `auth.require_totp_for_admins` row is missing
- **THEN** admin routes SHALL behave as if the toggle were `false`

