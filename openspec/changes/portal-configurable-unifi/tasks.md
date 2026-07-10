# Tasks

## 1. DB-backed UniFi config + encryption

- [ ] 1.1 In `src/service/settings_service.rs`, add a `unifi` key module
  mirroring `discord`: `ENABLED`, `CONTROLLER_URL`, `USERNAME`, `PASSWORD`,
  `SITE_ID`, and `LAST_TEST_AT` / `LAST_TEST_OK` / `LAST_TEST_ERROR`. Mark
  `PASSWORD` `is_sensitive` (encrypted at rest).
- [ ] 1.2 Add `UpdateUnifiConfig` with the "`Some(nonempty)` = encrypt +
  replace, `None`/blank = keep existing" semantics for the password (mirror
  `UpdateDiscordConfig`).
- [ ] 1.3 Migration: delete the dead `integrations.unifi.enabled` row.

## 2. Read config from the store at operation time

- [ ] 2.1 Confirm the UniFi integration reads its configuration from the
  settings store when it acts (as Discord does) and gates on `unifi.enabled`;
  adjust it to read `unifi.*` from the DB rather than the startup `.env`
  config. No persistent client handle is needed.

## 3. `.env` seed-on-first-boot

- [ ] 3.1 At startup, if the DB has no `unifi.*` values and
  `COTERIE__INTEGRATION__UNIFI__*` is present, seed the DB once (encrypting
  the password) and log it. After seeding, `.env` UniFi values are ignored.

## 4. Portal settings page

- [ ] 4.1 Add `src/web/portal/admin/unifi.rs` +
  `templates/admin/unifi_settings.html` mirroring `discord.rs`: GET renders
  current config (password blank), POST updates, `POST /test` authenticates
  to the controller with the submitted (or stored) credentials without
  persisting.
- [ ] 4.2 Register `/portal/admin/settings/unifi` (get/post/test) in the
  admin router so it inherits `require_admin_redirect` + CSRF.
- [ ] 4.3 Add a "UniFi" link in the admin nav and on the settings index.
- [ ] 4.4 Audit-log saves via `audit_service` with the password redacted.

## 5. Tests

- [ ] 5.1 Unit: `UpdateUnifiConfig` blank-password-keeps / nonempty-replaces.
- [ ] 5.2 Integration: `POST /test` reports success/failure without
  persisting; a save updates the store and the next gating action reads the
  new values.
- [ ] 5.3 Seed-from-env: DB empty + env present → seeded once; DB present →
  env ignored.
- [ ] 5.4 Assert `/portal/admin/settings` no longer renders the
  `integrations.unifi.enabled` row.
