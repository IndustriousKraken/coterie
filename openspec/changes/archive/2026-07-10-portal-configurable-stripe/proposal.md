# portal-configurable-stripe

## Why

Coterie's integrations are configurable two different ways, and Stripe is
on the wrong side of the split:

- **Discord** (`/portal/admin/settings/discord`) and **Email**
  (`/portal/admin/settings/email`) are fully portal-configurable — stored
  in `app_settings`, secrets encrypted at rest, applied at runtime, with a
  "test connection" affordance. A non-technical admin manages them without
  a shell.
- **Stripe** is `.env`-only (`COTERIE__STRIPE__*`, `src/config/mod.rs`),
  read **once at startup** to build the shared client (`src/main.rs:219`)
  and the webhook dispatcher's signing secret. Changing any of it requires
  editing `.env` on the server and restarting the service.

Worse, the generic settings page renders a seeded
`integrations.stripe.enabled` toggle (`migrations/001_initial_schema.sql`)
that is **read nowhere in the code** — it is dead. An operator sees
"Stripe: disabled" while Stripe is actually enabled via `.env`, and
toggling it does nothing. This is actively misleading and has already cost
debugging time.

Coterie's own principle (capability `admin-settings`: "Setting changes
take effect without restart") is violated for the one integration that
moves money.

- **Trigger:** an admin needs to add Stripe after initial setup, rotate a
  key, or switch test→live keys.
- **Harm today:** it cannot be done from the portal — it needs SSH + a
  `.env` edit + a restart — and the portal reports a false Stripe status.

## What Changes

- Stripe configuration (enabled, publishable key, secret key, webhook
  signing secret, optional success/cancel URLs) becomes **DB-backed** in
  `app_settings` under a `stripe.*` namespace, following the Discord/Email
  pattern: the secret key and webhook signing secret are stored
  **encrypted at rest** and never rendered back to the browser (a blank
  field means "keep the stored value").
- A dedicated **`/portal/admin/settings/stripe`** page (GET/POST/`POST
  /test`) mirroring the Discord/Email settings pages — admin-gated + CSRF,
  audit-logged.
- The shared Stripe client and the webhook dispatcher's signing secret are
  held behind a **hot-swappable handle** so a settings save **takes effect
  without a restart**.
- **`.env` becomes a seed, not the source of truth:** on first boot, if the
  DB has no Stripe config and `COTERIE__STRIPE__*` is present, seed the DB
  from it once (so wizard/IaC installs come up configured). Thereafter the
  DB is authoritative and `.env` Stripe values are ignored.
- The dead `integrations.stripe.*` seeded settings are **removed** by
  migration so the generic settings page stops showing a false toggle.

## Impact

- **Spec:** `admin-integrations` — ADDED "Stripe settings page with test
  and status". New capability `stripe-configuration` — DB-backed source of
  truth, hot-reload, encryption, seed-from-env, and disable safety.
- **Code:** `src/service/settings_service.rs` (stripe key constants +
  `UpdateStripeConfig`), `src/web/portal/admin/stripe.rs` (new handler,
  mirrors `discord.rs`), `templates/admin/stripe_settings.html`,
  `src/main.rs` (build the client from the DB with a one-time `.env` seed;
  wrap client + webhook secret in a hot-swappable handle),
  `src/api/state.rs` (swappable handle), the webhook dispatcher (read the
  current signing secret from the handle). Migration to delete
  `integrations.stripe.*`.
- **Security:** secret key + webhook secret encrypted via
  `secret_crypto`; write-only in the UI; a decrypt failure (e.g. session
  secret rotated) surfaces and Stripe is treated as unconfigured — the same
  fail-safe as `src/web/portal/admin/discord.rs`. A blank/whitespace secret
  yields an unconfigured client (webhook → 503), preserving the existing
  empty-webhook-secret guard (`config::nonblank`). Disabling Stripe while
  live `stripe_subscription` members exist SHALL warn.
- **Out of scope / unchanged:** the webhook wire format, the charge flow,
  `saved-card-management`, and `recurring-billing`. `stripe-webhook`
  signature verification is unchanged except that the signing secret's
  *source* becomes the hot-reloadable handle instead of a startup capture.
