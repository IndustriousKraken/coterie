# Operations Guide

Reference for operators running Coterie in production. Covers things
that aren't obvious from reading the code, with a focus on what breaks
when you change something.

For first-time setup:

- `DEPLOY-DIGITALOCEAN.md` — fresh DigitalOcean droplet end-to-end (Ubuntu)
- `DEPLOY-AWS.md` — fresh EC2 + EBS (or Lightsail) end-to-end (Ubuntu)
- `DEPLOY-ALPINE.md` — Alpine Linux with OpenRC (any provider)
- `MIGRATION.md` — moving Coterie between hosts (DO ↔ AWS, Ubuntu ↔ Alpine, etc.)
- `RESTORE.md` — restoring from a backup
- `SETUP.md` — staging environment with GitHub Actions deploys

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

## Backups

Coterie ships a backup script + systemd timer (`deploy/backup.sh`,
`coterie-backup.{service,timer}`) that:

- Runs daily at 03:30, producing **one bundle** per run: a `VACUUM INTO`
  database snapshot plus `uploads/` and `private-uploads/`
- Maintains 7 daily + 4 weekly + 12 monthly retention
- Optionally pushes each daily bundle to S3-compatible storage
  (DO Spaces, AWS S3, Backblaze B2, Cloudflare R2, Wasabi)

The wizard installs and enables the timer. On a deployment that
predates that, install it retroactively — see `RESTORE.md` §6. Check
which case you are in:

```bash
systemctl list-timers coterie-backup.timer
```

For an ad-hoc bundle (before an upgrade, say) run the same script:

```bash
sudo -u coterie bash /opt/coterie/deploy/backup.sh
```

Set `COTERIE_BACKUP_DIR` in `/etc/default/coterie-backup` to a
**separate volume** where the host has one — the default puts the
backups on the same disk as the data they protect. Restore procedure:
see `RESTORE.md` (`deploy/restore.sh <bundle>`).

**Test your backups.** A backup that's never been restored is a wish,
not a backup. Once a quarter, restore the latest bundle onto a
throwaway droplet and click through the portal — including an image and
a submission attachment, which is what a database-only backup would
have silently lost. Instructions in `RESTORE.md`.

---

## Log locations

Coterie writes to stdout / stderr; systemd captures those into the
journal:

```bash
sudo journalctl -u coterie -f       # tail live
sudo journalctl -u coterie --since "1 hour ago"
sudo journalctl -u coterie -u caddy  # alongside reverse proxy
```

### Authentication logs contain email addresses

Every outcome on the authentication surface emits a structured event —
`event=auth.login`, `auth.totp`, `auth.rate_limited`,
`auth.password_reset_requested`, and so on — with `outcome`, `member_id`,
`ip`, and a `reason` when denied. Filter on the fields rather than
grepping prose:

```bash
sudo journalctl -u coterie | grep 'event=auth.login'          # every login attempt
sudo journalctl -u coterie | grep 'reason=rate_limited'       # who hit the limiter
```

**These logs record the email address that was submitted**, including
addresses that match no member. Passwords, reset tokens, session tokens,
TOTP codes and recovery codes are never logged at any level, and an
identifier that isn't a syntactically valid email is replaced with a
placeholder (people type passwords into the email field).

The attempted address is there because "why can't this member sign in"
is not answerable without it — but it makes the log store personal data.
It inherits whatever retention and access controls that store already
has, which is worth checking **before** you ship logs off-box to a
hosted aggregator: journald on the host is one trust boundary, a
third-party log service is another. Set a retention window on the
journal (`SystemMaxUse` / `MaxRetentionSec` in `journald.conf`) if your
context calls for one.

Two lines to check at every startup:

- `Client IP: X-Forwarded-For / X-Real-Ip TRUSTED (…)` — how rate
  limiting keys callers, and whether that came from config or from
  inferring it off the base URL's scheme
- `Secure cookies: true (…)` — same, for the session cookie's `Secure`
  flag

If you instead see a warning about a **SINGLE SHARED BUCKET**, forwarded
headers are not trusted and every caller keys on `127.0.0.1`: a handful
of failed logins anywhere locks out the whole organization. Behind a
reverse proxy that sets `X-Forwarded-For`, set
`COTERIE__SERVER__TRUST_FORWARDED_FOR=true`.

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

Migrations are embedded in the binary (via `sqlx::migrate!`). Templates
are also compiled into the binary (askama). The runtime needs only:

- the `coterie` binary
- the `static/` directory (CSS / JS served by ServeDir)

To upgrade in place:

1. **Take a backup first.**
   ```bash
   sudo systemctl start coterie-backup.service     # one-shot
   ```
2. Deploy the new binary (and `static/` if it changed):
   ```bash
   sudo systemctl stop coterie
   sudo install -m 0755 -o coterie -g coterie \
       new-coterie /opt/coterie/coterie
   sudo rsync -a static/ /opt/coterie/static/
   sudo systemctl start coterie
   ```
3. Migrations run during startup. If startup fails, the service stays
   down and the DB is left in its prior state. Check
   `journalctl -u coterie -n 100`.
4. New settings arrive with safe defaults, so nothing here blocks the
   upgrade — but a few want an operator value afterwards. Currently:
   `org.signup_url` (Settings → Organization). It holds the absolute URL
   of your public join page and gates the login screen's "create a new
   account" link; empty (the default) renders no link at all. Coterie
   hosts no signup page of its own — `/public/signup` is the POST-only
   API your public site submits to, not a page a browser can open.

### `coterie-provision` is fetched per update, not installed

`/opt/coterie/coterie-provision` does not exist on a normal install, and
nothing places it there. That is expected, not a broken install.

The update logic lives in the `coterie-provision update` binary, and
`deploy/update.sh` bootstraps it: it downloads the **latest stable**
`coterie-provision` release asset, verifies its SHA256, and execs it from
a temp directory that is removed when the run ends. Fetching it per run is
the point — the update logic is always current, even when you are pinning
Coterie itself to an older tag with `--tag`.

So `release-deploy.sh` on an existing install resolves in this order:

1. `/opt/coterie/coterie-provision`, then a `coterie-provision` on `PATH`
   — used only if you deliberately put one there;
2. otherwise `deploy/update.sh`, which fetches one. The script says so on
   stdout when it takes this route, so an operator who *did* install a
   binary can see that it was not the one used.

`update.sh` is a bash script and is exec'd directly, honouring its
shebang. Running it under `sh` on Debian (where `/bin/sh` is dash) fails
with `trap: bad trap` — `tests/deploy_script_interpreter_test.rs` keeps
any caller in this repo from doing that.

### `deploy/` tracks the installed release

`coterie-provision update` refreshes `/opt/coterie/deploy/` from the
release it installs, so the ops scripts beside the binary are the ones
that shipped with it. It names every script it changed in its output.

- **A locally-modified script is preserved.** Where update replaces a
  script it first copies the current contents to `<name>.prev` beside it
  — the same convention as `coterie.prev` for the binary. If you had
  pinned a local edit to `backup.sh`, it is at `backup.sh.prev`; re-apply
  it or restore it from there.
- **A file you added that the release doesn't carry is left alone.**
  Scripts are copied one by one, not by replacing the directory.
- **A refreshed systemd unit is NOT activated.** Update writes inside
  `/opt/coterie` only; it never touches `/etc/systemd/system` and never
  runs `systemctl enable` or `daemon-reload`. A newer
  `deploy/coterie-backup.timer` on disk changes nothing about what is
  running. An instance provisioned before the backup timer existed still
  installs it once by hand — see `RESTORE.md` §6 — and
  `systemctl list-timers coterie-backup.timer` tells you which case you
  are in.

Before this, `deploy/` was only ever written on first install: an
instance updated for months kept the scripts from the release that
provisioned it, so `RESTORE.md` could name a `restore.sh` that was not on
the box.

### Update reports host discrepancies, and does not fix them

Because a refreshed unit file is not an activated one, `update` finishes
by checking whether the host is actually in the state the release it just
installed expects — starting with whether the units under `deploy/` are
enabled — and printing what it found:

```
Host state this release expects but did not find:
  - coterie-backup.timer ships with this release but is not enabled on this host.
      sudo cp /opt/coterie/deploy/coterie-backup.timer /etc/systemd/system/ && sudo systemctl daemon-reload && sudo systemctl enable --now coterie-backup.timer
Reported only — this update enabled, started, and reloaded nothing.
```

- **It reports; you resolve.** Update runs no `systemctl enable`, `start`,
  or `daemon-reload` on your behalf — it only asks `is-enabled`. Enabling
  a unit you deliberately switched off, on a host whose layout it cannot
  see, is not a call an update gets to make. Run the printed command if
  the finding is one you want fixed.
- **Silence means nothing was found.** There is no header and no
  all-clear line on a conformant host; the section appearing at all is
  the signal. If you see nothing here, every unit the release ships is
  enabled.
- **A finding never fails the update.** `update` exits zero either way —
  the update itself succeeded. Nothing here is a reason to stop trusting
  the exit code.
- **A check that can't tell says nothing.** If the host query itself
  cannot run (no systemd, for instance), no finding is reported rather
  than a guessed one.

This exists because `coterie-backup.timer` shipped in a release, was
never enabled on an instance provisioned before the wizard installed it,
and no backup ran for months — while `VERSION` reported a current release
and the scripts sat beside it looking installed.

Rollback isn't automated. If a release introduces problems:

1. Restore the previous binary
2. Restore the pre-upgrade DB snapshot (see `RESTORE.md`)
3. Restart

Forward-compatible migrations are the norm; rollback is an escape
hatch, not a routine.

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
