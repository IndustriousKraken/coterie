# Tasks

## 1. DB-backed Stripe config + encryption

- [ ] 1.1 In `src/service/settings_service.rs`, add a `stripe` key module
  mirroring `discord`/`email`: `ENABLED`, `PUBLISHABLE_KEY`, `SECRET_KEY`,
  `WEBHOOK_SECRET`, `SUCCESS_URL`, `CANCEL_URL`, and `LAST_TEST_AT` /
  `LAST_TEST_OK` / `LAST_TEST_ERROR`. Mark `SECRET_KEY` and
  `WEBHOOK_SECRET` `is_sensitive` (encrypted at rest); the publishable key
  is not sensitive.
- [ ] 1.2 Add `UpdateStripeConfig` with the established "`Some(nonempty)` =
  encrypt + replace, `None`/blank = keep existing" semantics for the two
  secrets (mirror `UpdateDiscordConfig`).
- [ ] 1.3 Migration: delete the dead `integrations.stripe.enabled`,
  `integrations.stripe.success_url`, and `integrations.stripe.cancel_url`
  rows. They are read nowhere, so no data carry-over is needed.

## 2. Hot-swappable Stripe client

- [ ] 2.1 Replace the startup-only `Option<Arc<StripeClient>>` with a
  swappable handle (e.g. `arc_swap::ArcSwapOption<StripeRuntime>` where
  `StripeRuntime { client, webhook_secret, enabled }`) in `AppState` and
  the webhook dispatcher.
- [ ] 2.2 Add `rebuild_stripe()` that reads current DB config (decrypting
  the secrets), constructs a client (or `None` when disabled/misconfigured),
  and swaps the handle. Reuse `config::nonblank` so a blank secret yields an
  unconfigured client (no forgeable webhook).
- [ ] 2.3 Call `rebuild_stripe()` at startup and after every successful
  Stripe settings save.
- [ ] 2.4 Point handlers and the webhook dispatcher at the handle (read the
  current value per request) instead of a captured `Arc`.

## 3. `.env` seed-on-first-boot

- [ ] 3.1 At startup, if the DB has no `stripe.*` values and
  `COTERIE__STRIPE__*` is present, seed the DB once (encrypting the
  secrets) and log that it happened. After seeding, `.env` Stripe values
  are ignored.

## 4. Portal settings page

- [ ] 4.1 Add `src/web/portal/admin/stripe.rs` +
  `templates/admin/stripe_settings.html` mirroring `discord.rs`/`email.rs`:
  GET renders current config (secrets blank), POST updates then calls
  `rebuild_stripe()`, `POST /test` validates the submitted (or stored)
  secret key against the Stripe API (e.g. retrieve account) without
  persisting.
- [ ] 4.2 Register `/portal/admin/settings/stripe` (get/post/test) in the
  admin router so it inherits `require_admin_redirect` + CSRF.
- [ ] 4.3 Add a "Stripe" link in the admin nav and on the settings index.
- [ ] 4.4 Audit-log saves via `audit_service` with secrets redacted.
- [ ] 4.5 If `enabled` is being turned off while any member is
  `billing_mode = stripe_subscription`, require an explicit confirmation
  that names the affected count before applying.

## 5. Tests

- [ ] 5.1 Unit: `UpdateStripeConfig` blank-secret-keeps / nonempty-replaces;
  a blank webhook secret resolves to unconfigured.
- [ ] 5.2 Integration (using the `FakeStripeGateway` seams): saving a valid
  config builds a client and a subsequent webhook signed with the new
  secret verifies; saving a blank secret disables Stripe (webhook → 503) —
  all without a restart.
- [ ] 5.3 Seed-from-env: DB empty + env present → DB seeded once and Stripe
  enabled; DB present → env ignored.
- [ ] 5.4 Assert `/portal/admin/settings` no longer renders any
  `integrations.stripe.*` row.
