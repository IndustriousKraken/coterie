# Tasks

Mirror the Stripe env→settings migration (029). Behavior is unchanged; only the
config source moves. Fail-closed and the disabled opt-out MUST be preserved.

## 1. Settings rows

- [ ] 1.1 Migration: `INSERT INTO app_settings` for `bot_challenge.provider`
  (`disabled`, string, category `bot_challenge`), `bot_challenge.secret_key`
  (`''`, string, `is_sensitive=1`), `bot_challenge.site_key` (`''`, string,
  public), `bot_challenge.timeout_ms` (`5000`, number). No env carry-over (env was
  never in the DB); admin fills them in.

## 2. Dynamic verifier

- [ ] 2.1 Add a `DynamicBotChallengeVerifier` (implements `BotChallengeVerifier`)
  that, per `verify` call, reads `bot_challenge.provider` from `SettingsService`;
  if `disabled`, pass through; if `turnstile`, read + decrypt `secret_key`
  (`SecretCrypto`) and `timeout_ms`, and POST to siteverify — reusing the existing
  `TurnstileVerifier` request/verify logic. Never log the secret.
- [ ] 2.2 `main.rs`: construct the dynamic verifier (with `settings_service` +
  http client) instead of `from_config(&settings.bot_challenge)`. Remove
  `BotChallengeConfig` from `src/config/mod.rs` and the `from_config` env path.

## 3. Admin UI

- [ ] 3.1 `settings.rs`: mark `bot_challenge.provider` as a select
  (`disabled`/`turnstile`) reusing the `is_signup_mode` mechanism;
  `secret_key` already renders as sensitive via `is_sensitive`. Confirm the
  `bot_challenge` category shows on the settings page.

## 4. Tests

- [ ] 4.1 Provider `disabled` in settings → request passes through (no verify).
- [ ] 4.2 Provider `turnstile` + stored secret → verify runs; missing token fails
  closed (403), matching the existing bot-challenge scenarios.
- [ ] 4.3 The secret round-trips encrypted (stored ciphertext; decrypts for the
  siteverify call) and never appears in logs.
- [ ] 4.4 Changing the provider setting changes behavior without reconstructing
  the app (the verifier reads settings live).
- [ ] 4.5 Existing bot-challenge suite (fail-closed, disabled opt-out, trait
  swap) still passes.

## 5. Verify

- [ ] 5.1 `openspec validate bot-challenge-admin-settings --strict` passes.
- [ ] 5.2 `cargo test` (bot-challenge + settings suites) green; `cargo clippy` clean.
