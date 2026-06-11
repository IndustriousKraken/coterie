## ADDED Requirements

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
