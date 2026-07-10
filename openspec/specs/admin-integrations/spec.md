# admin-integrations Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Discord settings page with test connection

`/portal/admin/settings/discord` SHALL provide:
- `GET` — settings page rendering current Discord configuration.
- `POST` — update Discord settings.
- `POST /test` — test the Discord connection without persisting.
- `POST /reconcile` — trigger a one-shot reconciliation of Discord roles for all linked members.

#### Scenario: Test connection does not persist anything

- **WHEN** an admin clicks "Test Connection"
- **THEN** the handler SHALL call the Discord API with the submitted credentials and report success/failure WITHOUT writing them to the settings store

#### Scenario: Reconcile is admin-only and audit-logged

- **WHEN** an admin triggers role reconciliation
- **THEN** the action SHALL be admin-gated and the service SHALL emit an audit-log entry summarizing the run

### Requirement: Email settings page with test send

`/portal/admin/settings/email` SHALL provide:
- `GET` — page rendering current email configuration.
- `POST` — update email settings.
- `POST /test` — send a test email to the admin's address using the current (or submitted) configuration.

#### Scenario: Test send uses the submitted-but-not-yet-saved values

- **WHEN** an admin enters new SMTP credentials and clicks "Send test"
- **THEN** the handler SHALL attempt the send with the submitted values without persisting them, so a bad config can be discovered before save

### Requirement: Stripe settings page with test and status

`/portal/admin/settings/stripe` SHALL provide:

- `GET` — a settings page rendering the current Stripe configuration. The
  publishable key MAY be shown; the secret key and webhook signing secret
  SHALL render as empty inputs and SHALL never be echoed back to the
  browser.
- `POST` — update Stripe settings. On success the running Stripe client and
  webhook signing secret SHALL be rebuilt so the change takes effect without
  a process restart.
- `POST /test` — validate the submitted (or stored) secret key against the
  Stripe API without persisting anything.

The page SHALL be gated by `require_admin_redirect` and CSRF, and every save
SHALL be audit-logged with secret values redacted.

#### Scenario: Secret fields are write-only

- **WHEN** the settings page renders with a Stripe secret key already stored
- **THEN** the secret key and webhook signing secret inputs SHALL render
  empty, and submitting them empty SHALL preserve the stored (encrypted)
  values rather than overwriting them with blanks

#### Scenario: Test does not persist

- **WHEN** an admin clicks "Test" with a secret key in the form
- **THEN** the handler SHALL call the Stripe API with the submitted key and
  report success/failure WITHOUT writing it to the settings store

#### Scenario: Save takes effect without restart

- **WHEN** an admin saves a new secret key or webhook signing secret
- **THEN** subsequent charges and webhook signature verification SHALL use
  the new values immediately, with no process restart

#### Scenario: Disabling with live subscriptions requires confirmation

- **WHEN** an admin unchecks "Enable Stripe" and one or more members have
  `billing_mode = stripe_subscription`
- **THEN** the UI SHALL require an explicit confirmation naming the count of
  affected members before applying, because disabling stops the webhook from
  crediting their renewals

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

