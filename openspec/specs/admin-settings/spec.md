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

### Requirement: org.signup_url is optional and gates the login page's create-account link

The system SHALL provide an org setting `org.signup_url`, defaulting to the empty
string, holding the absolute URL of the organization's public account-signup page.

The portal login page SHALL render a "create account" link if and only if
`org.signup_url` is non-empty, and SHALL point that link at the configured value.
When the setting is empty the login page SHALL render no such link at all.

The link SHALL NOT point at `/public/signup`. That route is a POST-only JSON API
consumed by the organization's public site; a browser following it as a GET
receives a 405 and downloads the error body as a file. Coterie SHALL NOT host a
self-service signup page, so there is no internal destination for this link —
account creation belongs to the organization's public site, and the portal is for
people who already have accounts.

`org.website_url` SHALL NOT be reused for this purpose. It answers a different
question — where the organization's site is — and its stock value is a
placeholder, so reusing it would either send a prospective member to a homepage
to hunt for a join form or, on a fresh install, produce another dead link.

Defaulting to empty is deliberate: a deployment that has not configured a signup
page SHALL advertise none, because no link is strictly better than a broken one.

#### Scenario: An unconfigured deployment shows no create-account link

- **WHEN** `org.signup_url` is empty and an anonymous visitor loads the login page
- **THEN** no create-account link SHALL be rendered

#### Scenario: A configured deployment links to its own signup page

- **WHEN** `org.signup_url` is set to the organization's public join page
- **THEN** the login page SHALL render a create-account link pointing at that URL

#### Scenario: The link never targets the signup API

- **WHEN** the login page template is rendered under any configuration
- **THEN** the create-account link SHALL NOT target `/public/signup`, which is
  POST-only and downloads a 405 body when followed by a browser

