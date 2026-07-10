# admin-integrations Specification (delta)

## ADDED Requirements

### Requirement: UniFi settings page with test connection

`/portal/admin/settings/unifi` SHALL provide:

- `GET` — a settings page rendering the current UniFi configuration
  (controller URL, username, site id). The password SHALL render as an empty
  input and SHALL never be echoed back to the browser.
- `POST` — update UniFi settings. The password is stored encrypted at rest;
  submitting the password field blank SHALL preserve the stored value.
- `POST /test` — authenticate to the UniFi controller with the submitted (or
  stored) credentials and report success/failure without persisting.

The page SHALL be gated by `require_admin_redirect` and CSRF, and every save
SHALL be audit-logged with the password redacted. UniFi configuration SHALL
be stored in `app_settings` under a `unifi.*` namespace and read from there
at operation time, so changes take effect without a restart.

#### Scenario: Password is write-only

- **WHEN** the settings page renders with a UniFi password already stored
- **THEN** the password input SHALL render empty, and submitting it empty
  SHALL preserve the stored (encrypted) password rather than overwriting it
  with a blank

#### Scenario: Test connection does not persist

- **WHEN** an admin clicks "Test Connection"
- **THEN** the handler SHALL authenticate to the controller with the
  submitted credentials and report success/failure WITHOUT writing them to
  the settings store

#### Scenario: .env seeds the store once, then the store wins

- **WHEN** a fresh install boots with `COTERIE__INTEGRATION__UNIFI__*` in
  `.env` and no `unifi.*` rows in the database
- **THEN** the database SHALL be seeded from `.env` once; once `unifi.*` rows
  exist, startup SHALL use the database values and ignore `.env`

#### Scenario: The dead integrations.unifi.enabled row is gone

- **WHEN** an admin views `/portal/admin/settings`
- **THEN** no `integrations.unifi.enabled` row SHALL appear; UniFi is managed
  only at `/portal/admin/settings/unifi`
