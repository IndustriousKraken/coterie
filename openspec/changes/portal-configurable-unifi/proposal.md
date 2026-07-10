# portal-configurable-unifi

## Why

Same split as Stripe (see `portal-configurable-stripe`): Discord and Email
are portal-configurable, but **UniFi** is `.env`-only
(`COTERIE__INTEGRATION__UNIFI__*`, `src/config/mod.rs`) and carries a dead
`integrations.unifi.enabled` seeded toggle
(`migrations/001_initial_schema.sql`) that is read nowhere. An admin cannot
add UniFi after setup, rotate credentials, or point at a different
controller without SSH — and the generic settings page misreports UniFi
status.

Unlike Stripe, UniFi has no persistent client captured at startup and no
signing secret in the request path, so this is the smaller, lower-risk half
of the same cleanup — a straight mirror of the Discord settings page.

## What Changes

- UniFi configuration (enabled, controller URL, username, password, site id)
  becomes **DB-backed** in `app_settings` under a `unifi.*` namespace, with
  the password **encrypted at rest** and never echoed to the browser (blank
  field = keep existing).
- A **`/portal/admin/settings/unifi`** page (GET/POST/`POST /test`) mirroring
  the Discord settings page: admin-gated + CSRF, audit-logged, with a "test
  connection" that authenticates to the controller without persisting.
- **`.env` seeds the DB once** on first boot (as with Stripe); the DB is
  authoritative thereafter.
- The dead `integrations.unifi.enabled` seeded row is **removed** by
  migration.

## Impact

- **Spec:** `admin-integrations` — ADDED "UniFi settings page with test
  connection".
- **Code:** `src/service/settings_service.rs` (unifi key constants +
  `UpdateUnifiConfig`), `src/web/portal/admin/unifi.rs` +
  `templates/admin/unifi_settings.html` (mirror `discord.rs`), route
  registration, a one-time `.env` seed at startup, and a migration to delete
  the dead row. The UniFi integration SHALL read its config from the settings
  store at operation time (it already gates on `enabled`), so **no persistent
  client rebuild is required** — this is the key difference from
  `portal-configurable-stripe`.
- **Security:** the password is encrypted via `secret_crypto`, write-only in
  the UI; a decrypt failure means UniFi is treated as unconfigured.
- **Unchanged:** the access-gating behavior described in `unifi-integration`.
