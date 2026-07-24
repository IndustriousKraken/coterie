# bot-challenge Specification

## ADDED Requirements

### Requirement: Bot-challenge is configured from admin settings

Bot-challenge configuration SHALL live in DB-backed `app_settings` (category
`bot_challenge`), administrable from the portal settings page, NOT in environment
variables. The rows SHALL be: `bot_challenge.provider` (`disabled` or `turnstile`,
default `disabled`), `bot_challenge.secret_key` (marked sensitive and encrypted at
rest via `SecretCrypto`), `bot_challenge.site_key` (public; stored for admin
reference), and `bot_challenge.timeout_ms` (default 5000). The verifier SHALL read
provider, secret, and timeout from settings at request time, so a change made in
the portal takes effect WITHOUT a restart. The `provider` field SHALL render as a
select (disabled/turnstile) and `secret_key` as a sensitive field. This mirrors
the Stripe/Discord settings model; the environment-variable configuration path is
removed.

#### Scenario: Provider is read from settings, not env

- **WHEN** `bot_challenge.provider` is set to `turnstile` in the settings page with
  a secret key
- **THEN** verification SHALL run against that provider using the stored secret,
  with no environment variable involved

#### Scenario: Changing the provider takes effect without a restart

- **WHEN** an admin changes `bot_challenge.provider` from `disabled` to `turnstile`
  (or back) in the portal
- **THEN** subsequent public requests SHALL be verified under the new value without
  restarting the service

#### Scenario: The secret key is encrypted at rest

- **WHEN** `bot_challenge.secret_key` is saved
- **THEN** it SHALL be stored encrypted (like the Stripe secret key) and decrypted
  only to call the provider's siteverify endpoint, never logged
