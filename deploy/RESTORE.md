# Restoring Coterie from a backup

Backups produced by `deploy/backup.sh` are single-file SQLite snapshots
(`*.db.gz`) generated via `VACUUM INTO`. Restoring is replacing the
live database file with one of those snapshots. This doc covers:

1. Where backups live
2. Restoring on the same host
3. Restoring on a fresh host
4. Restoring from offsite (S3) when the original host is gone
5. Validating the restore

This procedure has been tested end-to-end on a throwaway droplet —
**do it once on staging before you ever need to do it on production**.
The first time you read these instructions should not be while users
are paging you.

---

## 1. Where backups live

```
/var/lib/coterie/backups/
├── daily/    coterie-YYYY-MM-DD.db.gz   (last 7)
├── weekly/   coterie-YYYY-W##.db.gz     (last 4)
└── monthly/  coterie-YYYY-MM.db.gz      (last 12)
```

If `COTERIE_BACKUP_S3_URI` is configured (see `backup.sh`), every daily
snapshot also gets pushed to that bucket under `daily/`.

Pick the most recent backup that pre-dates whatever went wrong. If
you're not sure when corruption occurred, restore the latest and check
the data; if it's bad, walk backwards.

---

## 2. Restore on the same host (most common)

The Coterie service must be **stopped** before replacing the file —
running `coterie` with someone else swapping the SQLite file
underneath corrupts WAL state.

```bash
# 1. Stop the service
sudo systemctl stop coterie

# 2. Pick the snapshot
SRC=/var/lib/coterie/backups/daily/coterie-2026-04-26.db.gz

# 3. Move the current DB aside (don't delete — you might need it)
sudo mv /var/lib/coterie/coterie.db          /var/lib/coterie/coterie.db.broken
sudo mv /var/lib/coterie/coterie.db-shm      /var/lib/coterie/coterie.db-shm.broken 2>/dev/null || true
sudo mv /var/lib/coterie/coterie.db-wal      /var/lib/coterie/coterie.db-wal.broken 2>/dev/null || true

# 4. Decompress the snapshot in place
sudo gunzip -c "$SRC" | sudo tee /var/lib/coterie/coterie.db > /dev/null

# 5. Fix ownership (gunzip ran as root)
sudo chown coterie:coterie /var/lib/coterie/coterie.db
sudo chmod 0640 /var/lib/coterie/coterie.db

# 6. Sanity-check the file
sudo -u coterie sqlite3 /var/lib/coterie/coterie.db 'PRAGMA integrity_check;'
# Expect: ok

# 7. Start the service
sudo systemctl start coterie
sudo journalctl -u coterie -n 50

# 8. Once you've confirmed everything works, delete the .broken files
sudo rm /var/lib/coterie/coterie.db.{broken,shm.broken,wal.broken}
```

Login pages and member portal should be reachable within seconds.

---

## 3. Restore onto a fresh host

Use this when the original VM is gone (provider failure, manual
rebuild, migrating providers).

```bash
# After you've installed Coterie on the new host (see DEPLOY-*.md):

# 1. Service must be stopped or never-started
sudo systemctl stop coterie 2>/dev/null || true

# 2. Place the snapshot
sudo mkdir -p /var/lib/coterie
sudo gunzip -c /path/to/coterie-2026-04-26.db.gz \
    | sudo tee /var/lib/coterie/coterie.db > /dev/null
sudo chown coterie:coterie /var/lib/coterie/coterie.db
sudo chmod 0640 /var/lib/coterie/coterie.db

# 3. Verify
sudo -u coterie sqlite3 /var/lib/coterie/coterie.db 'PRAGMA integrity_check;'

# 4. Start
sudo systemctl start coterie
```

Coterie will run any newer migrations against the restored DB at
startup. This is normal — migrations are forward-compatible. If
startup fails on a migration error, see "Troubleshooting" below.

---

## 4. Restore from offsite (S3 / B2 / R2 / Spaces)

If the host is gone and you don't have a local copy:

```bash
# AWS S3
aws s3 ls s3://my-coterie-backups/prod/daily/ | sort | tail
aws s3 cp s3://my-coterie-backups/prod/daily/coterie-2026-04-26.db.gz .

# Backblaze / Wasabi / DO Spaces / Cloudflare R2
# Same commands; just set AWS_ENDPOINT_URL_S3 first:
export AWS_ENDPOINT_URL_S3=https://s3.us-west-002.backblazeb2.com
aws s3 ls s3://my-coterie-backups/prod/daily/ | sort | tail
aws s3 cp s3://my-coterie-backups/prod/daily/coterie-2026-04-26.db.gz .
```

Then proceed as in section 3 (fresh host).

---

## 5. Validating the restore

After the service is up, click through:

- `/login` — log in as a known admin
- `/portal/admin/members` — member list matches expectations
- `/portal/admin/audit` — last entry's timestamp matches when you'd
  expect (i.e. shortly before the snapshot was taken)
- `/portal/admin/settings/email` — SMTP password should be intact
  (it's encrypted with a key derived from `session_secret`; if you
  also rotated the secret as part of the restore, you'll need to
  re-enter the SMTP password)
- A test payment in Stripe test mode if Stripe is configured

If the audit log's last entry is much older than expected, you may
have grabbed a stale snapshot. Check newer ones.

---

## Troubleshooting

**`integrity_check` says anything other than `ok`.**
The snapshot itself is corrupt. Try the next-oldest one.

**Service starts but dues calculations look wrong.**
Compare `dues_paid_until` for a sample of members against what you
expect. If the snapshot is from before a recent payment, that payment
is now lost; you may need to re-record manually via the admin UI.

**Migration error at startup after restore.**
Means you're restoring a snapshot taken under an older binary against
a newer binary. Two paths:

1. Roll back the binary to the version that was running when the
   snapshot was taken (usually the cleanest), then upgrade.
2. Skip ahead: `sqlx migrate run` is automatic, so if it errored,
   read `journalctl -u coterie -n 100` for the specific migration.
   Most failures are recoverable by hand-editing or by dropping a
   newly-added constraint and re-adding it.

**SMTP / outbound emails silent after restore.**
The encrypted SMTP password in the DB is keyed by `session_secret`.
If `.env` was restored from a different host (which had a different
`session_secret`), the ciphertext can't be decrypted. Re-enter the
SMTP password through the admin UI; Coterie re-encrypts under the
current secret. (See `deploy/OPS.md` for the full secret-rotation
context.)

**Stripe webhooks failing after restore.**
The `processed_stripe_events` table is part of the snapshot, so
Stripe retries since the snapshot will be deduplicated correctly.
Webhooks for events created during the gap (between snapshot and
restore) will arrive and be processed normally on Stripe's retry
schedule (~3 days).
