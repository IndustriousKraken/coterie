# Operations Guide

Reference for operators running Coterie in production. Covers things
that aren't obvious from reading the code, with a focus on what breaks
when you change something.

---

## `session_secret` rotation

`COTERIE__AUTH__SESSION_SECRET` is Coterie's master application secret.
It's used as the key-derivation input for three things:

1. **Login sessions** (`src/auth/mod.rs`). Sessions are actually stored
   server-side with a random 32-byte token, so rotating the secret
   **doesn't invalidate sessions on its own** — they live in the
   `sessions` table and are looked up by hashed token.
2. **CSRF tokens** (`src/auth/csrf.rs`). Stateless HMAC tokens. A key
   derived from `session_secret` signs every token. Rotating the secret
   invalidates **every outstanding CSRF token** — any form submitted
   while the user is mid-flow returns 403; the user reloads and tries
   again.
3. **Encrypted settings** (`src/auth/secret_crypto.rs`). The SMTP
   password (and any future secret stored in `app_settings` with
   `is_sensitive = 1`) is encrypted at rest with a key derived from
   `session_secret`. Rotating the secret makes existing ciphertext
   **unreadable** — the admin has to re-enter the value through the
   settings UI, where it gets re-encrypted with the new key.

### Safe rotation procedure

1. **Collect anything you'll need to re-enter.** Log into the admin
   portal first and note the SMTP provider credentials (or grab them
   from your provider's dashboard). You'll paste these back in after
   the rotation.

2. **Generate a new secret.**
   ```bash
   openssl rand -hex 32
   ```
   (Or anything with >=32 bytes of entropy.)

3. **Edit `.env` and restart the service.**
   ```bash
   sudo systemctl restart coterie
   ```

4. **Expect these symptoms during the transition window:**
   - Any user with an open form in a browser tab: their next submit
     returns 403 (bad CSRF token). Reloading the page fixes it.
   - Outbound email temporarily silent: the encrypted SMTP password
     can't be decrypted. Coterie's `DynamicSender` falls back to log
     mode, and the admin email settings page shows an amber "can't
     decrypt" banner.

5. **Re-enter the SMTP password** via `/portal/admin/settings/email`.
   Coterie re-encrypts it under the new secret. The next outbound email
   (any test message, verification link, reminder, etc.) will succeed.

### When NOT to rotate

- Routinely. There's no "expiration" on session_secret — rotate only
  in response to a known or suspected exposure (former admin's laptop,
  stolen backup, git leak, etc.).
- Without access to the SMTP credentials. You'll need to paste them
  back after rotation.

### What session_secret is NOT

- Not used to sign session cookies. Session cookies contain a random
  token that's looked up server-side; there's no signing.
- Not used for password hashing. Passwords use Argon2id with per-user
  salts — rotating the secret doesn't invalidate anyone's password.
- Not a Stripe key, not a webhook secret, not a DB encryption key at
  the storage layer. Only what's listed above.

---

## Database backups

Coterie uses a single SQLite file (default `data/coterie.db`). Backup
strategy is whatever your VPS provider offers for file-level snapshots,
plus occasional `.dump`s for extra safety:

```bash
sudo -u coterie sqlite3 /opt/coterie/data/coterie.db ".backup '/opt/coterie/data/backup-$(date +%F).db'"
```

The WAL-mode database means the live file is safe to copy as long as
you also copy `coterie.db-shm` and `coterie.db-wal`. Restoring is just
replacing all three.

---

## Log locations

Coterie writes to stdout / stderr; systemd captures those into the
journal:

```bash
sudo journalctl -u coterie -f       # tail live
sudo journalctl -u coterie --since "1 hour ago"
sudo journalctl -u coterie -u caddy  # alongside reverse proxy
```

Key log lines to watch for:

- `Billing runner started` — background job is alive
- `Dues reminders: N sent, M skipped` — reminder cycle finished
- `Email mode: log` — email is NOT going out, DB configured as log mode
- `Reminder send failed` / `SMTP send failed` — email send errors
  (details on the following line)
- `Invalid signature` (in Stripe webhook path) — webhook secret
  mismatch; regenerate in Stripe dashboard and update the admin email
  settings page

---

## Upgrading

Migrations are embedded in the binary and run automatically at startup
(see `sqlx::migrate!` in `src/main.rs`). To upgrade:

1. Deploy the new binary + `templates/` + `static/` + `migrations/`.
2. Restart the service. Migrations run during startup.
3. If startup fails, the service doesn't start. Check
   `journalctl -u coterie` for the migration error; the DB is left in
   its prior state.

Rollback isn't automated. For a deployed release-candidate that
introduces problems, restore the prior binary and restore the pre-
migration database snapshot (from the backup you took before
upgrading — see above).

---

## Routine maintenance

- **Audit log size**: prunes automatically based on
  `audit.retention_days` (default 365). Set lower if you want
  smaller backups.
- **Sessions**: expired rows are deleted hourly by the background
  cleanup task.
- **Orphaned uploads**: event/announcement delete handlers delete the
  file. If you notice accumulation in `data/uploads/`, something
  upstream wasn't going through the proper handler — check your
  integrations.
