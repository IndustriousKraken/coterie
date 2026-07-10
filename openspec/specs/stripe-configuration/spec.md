# stripe-configuration Specification

## Purpose
TBD - created by archiving change portal-configurable-stripe. Update Purpose after archive.
## Requirements
### Requirement: Stripe configuration is DB-backed and encrypted at rest

The system SHALL store all Stripe configuration in the `app_settings`
table under keys in the `stripe` namespace: the enabled flag, publishable
key, secret key, webhook signing secret, and optional success and cancel
URLs. The secret key and webhook signing secret SHALL be encrypted at rest
using the same mechanism as the Discord bot token and SMTP password, and
marked sensitive so they are never returned to the browser.

#### Scenario: Secrets are stored as ciphertext

- **WHEN** a Stripe secret key is saved
- **THEN** the stored value SHALL be ciphertext, and reading it for use SHALL
  decrypt it in-process only

#### Scenario: An undecryptable secret disables Stripe safely

- **WHEN** the stored secret cannot be decrypted (e.g. the session secret was
  rotated)
- **THEN** Stripe SHALL be treated as unconfigured — no client is built and
  the webhook endpoint returns 503 — rather than using a broken key, and the
  settings page SHALL surface the decrypt failure

### Requirement: The running Stripe client is hot-reloaded on config change

The shared Stripe client and the webhook signing secret SHALL be held behind
a swappable handle rather than captured once at startup. A successful Stripe
settings save SHALL rebuild the client and swap the handle so charges and
webhook verification pick up the new values without a restart. A blank or
whitespace-only secret (secret key or webhook signing secret) SHALL yield an
unconfigured client — the webhook endpoint SHALL return 503 rather than
verify against a zero-length HMAC key.

#### Scenario: A new webhook secret verifies immediately

- **WHEN** the webhook signing secret is changed via the portal
- **THEN** a webhook signed with the new secret SHALL verify and one signed
  with the old secret SHALL be rejected, with no restart

#### Scenario: Blank secret leaves the endpoint unconfigured

- **WHEN** the secret key or webhook signing secret is saved blank
- **THEN** the rebuilt client SHALL be `None` and the webhook endpoint SHALL
  return 503 rather than accept forgeable events

### Requirement: .env seeds the database once, then the database is authoritative

On startup the system SHALL seed the database once from the provisioning
environment: when the database holds no Stripe settings but the environment
provides them, the system SHALL copy them in (encrypting the secrets) and
log that it did so. Thereafter the database SHALL be the single source of
truth and the environment Stripe values SHALL be ignored, so installs
provisioned with Stripe via the wizard come up configured with no portal
action.

#### Scenario: A wizard-provisioned install comes up configured

- **WHEN** a fresh install boots with Stripe enabled and keyed in the
  environment and no Stripe settings in the database
- **THEN** the database SHALL be seeded from the environment and Stripe SHALL
  be enabled without any portal action

#### Scenario: Portal edits win over .env

- **WHEN** Stripe settings already exist in the database
- **THEN** startup SHALL use the database values and SHALL NOT re-read or
  re-seed from the environment

### Requirement: Dead integrations.stripe settings are removed

The system SHALL remove the seeded `integrations.stripe.enabled`,
`integrations.stripe.success_url`, and `integrations.stripe.cancel_url`
rows — they are read nowhere and cause the generic settings page to
misreport Stripe status. The generic `/portal/admin/settings` page SHALL NOT
display any Stripe toggle; Stripe is managed only at
`/portal/admin/settings/stripe`.

#### Scenario: No false Stripe status on the generic settings page

- **WHEN** an admin views `/portal/admin/settings`
- **THEN** no `integrations.stripe.*` row SHALL appear, so the page cannot
  show "Stripe disabled" while Stripe is enabled

