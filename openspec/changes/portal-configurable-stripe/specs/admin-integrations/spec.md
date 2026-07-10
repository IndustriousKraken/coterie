# admin-integrations Specification (delta)

## ADDED Requirements

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
