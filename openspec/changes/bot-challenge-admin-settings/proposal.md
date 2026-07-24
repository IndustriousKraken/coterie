# bot-challenge-admin-settings

## Why

Bot-challenge (Turnstile) is configured only from **environment variables**
loaded at startup (`settings.bot_challenge`), so there is no admin UI for it —
an operator looking to turn on the captcha finds nothing in the portal and has
no reason to know to edit a file on the server. A secret that lives only in a
server file is, day to day, invisible.

Move bot-challenge config to **DB-backed `app_settings`**, exactly as the Stripe
and Discord integrations already do (migration 029): the admin sets the provider
and secret from the settings page, secrets encrypted at rest, no restart. This is
what makes the Turnstile rollout (widget already built) actually operable.

## What Changes

- New `app_settings` rows (category `bot_challenge`): `bot_challenge.provider`
  (`disabled` | `turnstile`, default `disabled`), `bot_challenge.secret_key`
  (**sensitive**, encrypted at rest), `bot_challenge.site_key` (public — admin
  reference; the marketing join form drives the widget from its own config),
  `bot_challenge.timeout_ms` (number, default 5000).
- The verifier reads its config from settings at request time (a dynamic verifier
  matching the email `DynamicSender` / Stripe pattern), replacing the
  startup-static `from_config(&settings.bot_challenge)`. Changing the provider or
  key from the portal takes effect **without a restart**.
- The provider field renders as a **dropdown** (`disabled`/`turnstile`) reusing
  the `is_signup_mode` select mechanism; `secret_key` renders as a sensitive
  (password) field like the Stripe secret.
- Behavior is unchanged: fail-closed when the provider is active, the disabled
  opt-out, the trait abstraction, and the verify outcomes all stand — only the
  config **source** moves from env to settings.
- The env `BotChallengeConfig` is retired (as 029 retired Stripe's env), so there
  is one source of truth.

## Impact

- **Spec:** `bot-challenge` — 1 ADDED requirement ("Bot-challenge is configured
  from admin settings"). The existing requirements (fail-closed verification,
  disabled opt-out, swappable trait) are unchanged — they reference the provider
  *value*, which is now a setting.
- **Code:** a migration seeding the four rows; a `DynamicBotChallengeVerifier`
  (reads provider/secret/timeout from `SettingsService`, decrypts the secret via
  `SecretCrypto`); `main.rs` wires the dynamic verifier instead of `from_config`;
  `settings.rs` marks `bot_challenge.provider` as a select (like `signup_mode`);
  remove `BotChallengeConfig` from `config/mod.rs`.
- **Tests:** provider from settings drives verification (disabled → pass-through;
  turnstile → verify); secret is stored encrypted and decrypts for the siteverify
  call; changing the setting changes behavior without a rebuild of the verifier.
- **Rollout:** after this ships, the operator sets provider + secret in the portal
  (no env, no restart); the marketing site still holds the public site key.
