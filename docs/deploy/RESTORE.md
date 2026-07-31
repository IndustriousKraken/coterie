# Restoring Coterie from a backup

A backup produced by `deploy/backup.sh` is **one bundle**:

```
coterie-YYYY-MM-DD.tar.gz
├── coterie.db          the vacuumed database snapshot
├── uploads/            public images (event flyers, announcement banners)
└── private-uploads/    submission attachments
```

All three, because the database references files it does not contain.
A database-only restore brings back submission rows whose attachments
are gone and event rows whose images 404 — and nothing errors, so you
find out from a member, weeks later.

Restoring is running `deploy/restore.sh` against a bundle. This doc
covers:

1. Where backups live
2. Restoring with the script (the normal path)
3. Restoring onto a fresh host
4. Restoring from offsite (S3) when the original host is gone
5. The manual procedure, for when the script can't run
6. Installing the backup timer on a deployment that predates it
7. Validating the restore
8. Why this is not the same thing as a droplet snapshot

**Do it once on staging before you ever need to do it on production.**
The first time you read these instructions should not be while users
are paging you. The round trip (backup → destroy → restore → working
instance) is also exercised by the test suite on every change, so the
procedure below is not folklore — but your host is not the test host.

---

## 1. Where backups live

```
/var/lib/coterie/backups/
├── daily/    coterie-YYYY-MM-DD.tar.gz   (last 7)
├── weekly/   coterie-YYYY-W##.tar.gz     (last 4)
└── monthly/  coterie-YYYY-MM.tar.gz      (last 12)
```

If `COTERIE_BACKUP_S3_URI` is configured (see `backup.sh`), every daily
bundle also gets pushed to that bucket under `daily/`.

**Point `COTERIE_BACKUP_DIR` at a separate volume if the host has one.**
The default (`{data_dir}/backups`) keeps the backups on the same disk as
the data they protect, so a single disk failure takes both. Set it in
`/etc/default/coterie-backup`, which the timer's service unit reads:

```bash
# /etc/default/coterie-backup
COTERIE_BACKUP_DIR=/mnt/backups/coterie
# and/or offsite:
COTERIE_BACKUP_S3_URI=s3://my-coterie-backups/prod/
```

A volume mount is one disk failure of protection; the S3 push is one
provider failure of protection. They stack; neither replaces the other.

Pick the most recent bundle that pre-dates whatever went wrong. If
you're not sure when corruption occurred, restore the latest and check
the data; if it's bad, walk backwards.

---

## 2. Restore with the script (the normal path)

```bash
sudo bash /opt/coterie/deploy/restore.sh \
    /var/lib/coterie/backups/daily/coterie-2026-07-31.tar.gz
```

That is the whole procedure. In order, the script:

1. **verifies the bundle** — readable, and carries all three components.
   A truncated or corrupt bundle fails **here**, with the service still
   running and your data untouched.
2. stops `coterie`
3. **moves** the current database and both upload roots into
   `/var/lib/coterie/pre-restore-<timestamp>/` — it never deletes them
4. extracts the bundle into place
5. `chown`s everything to the `coterie` service account
6. runs `PRAGMA integrity_check`
7. starts `coterie` — only if that check passed

The displaced-data path is printed at the end. **Restoring the wrong
snapshot is reversible**: stop the service and move those entries back.
Nothing else deletes that directory — once the restore is confirmed
good, that's your job:

```bash
sudo rm -rf /var/lib/coterie/pre-restore-20260731T031200
```

If your instance uses a non-default data dir, the script reads
`COTERIE__SERVER__DATA_DIR` from the environment, or takes `--data-dir`:

```bash
sudo bash /opt/coterie/deploy/restore.sh --data-dir /srv/coterie <bundle>
```

### Taking a bundle on demand

Before a risky migration or upgrade, take one by hand. It is the same
script and the same artifact as the scheduled run:

```bash
sudo -u coterie bash /opt/coterie/deploy/backup.sh
```

---

## 3. Restore onto a fresh host

Use this when the original VM is gone (provider failure, manual
rebuild, migrating providers). The bundle is self-contained: with the
config file and a binary, it *is* the instance.

```bash
# 1. Install Coterie on the new host (see DEPLOY-*.md or the wizard),
#    but don't let it serve traffic yet.
sudo systemctl stop coterie 2>/dev/null || true

# 2. Copy the bundle across, then restore it.
sudo bash /opt/coterie/deploy/restore.sh /tmp/coterie-2026-07-31.tar.gz

# 3. Restore /opt/coterie/.env from the old host as well — the bundle
#    carries data, not configuration. Keep the OLD session_secret:
#    rotating it makes the encrypted SMTP password undecryptable.
```

Coterie runs any newer migrations against the restored DB at startup.
This is normal — migrations are forward-compatible. If startup fails on
a migration error, see "Troubleshooting" below.

---

## 4. Restore from offsite (S3 / B2 / R2 / Spaces)

If the host is gone and you don't have a local copy:

```bash
# AWS S3
aws s3 ls s3://my-coterie-backups/prod/daily/ | sort | tail
aws s3 cp s3://my-coterie-backups/prod/daily/coterie-2026-07-31.tar.gz .

# Backblaze / Wasabi / DO Spaces / Cloudflare R2
# Same commands; just set AWS_ENDPOINT_URL_S3 first:
export AWS_ENDPOINT_URL_S3=https://s3.us-west-002.backblazeb2.com
aws s3 ls s3://my-coterie-backups/prod/daily/ | sort | tail
aws s3 cp s3://my-coterie-backups/prod/daily/coterie-2026-07-31.tar.gz .
```

Then proceed as in section 3 (fresh host).

---

## 5. The manual procedure (fallback)

Use this when the script can't run — an older host that predates it, a
rescue shell without the repo, or a bundle you need to open selectively.
It is the same steps the script takes, by hand.

```bash
BUNDLE=/var/lib/coterie/backups/daily/coterie-2026-07-31.tar.gz

# 1. Verify BEFORE touching anything. This must list coterie.db,
#    uploads/ and private-uploads/. If it errors, the bundle is bad —
#    stop here, with the service still up.
tar -tzf "$BUNDLE"

# 2. Stop the service. Swapping the SQLite file under a running
#    process corrupts WAL state.
sudo systemctl stop coterie

# 3. Move the current state aside — do NOT delete it.
sudo mkdir -p /var/lib/coterie/pre-restore
sudo mv /var/lib/coterie/coterie.db      /var/lib/coterie/pre-restore/ 2>/dev/null || true
sudo mv /var/lib/coterie/coterie.db-wal  /var/lib/coterie/pre-restore/ 2>/dev/null || true
sudo mv /var/lib/coterie/coterie.db-shm  /var/lib/coterie/pre-restore/ 2>/dev/null || true
sudo mv /var/lib/coterie/uploads         /var/lib/coterie/pre-restore/ 2>/dev/null || true
sudo mv /var/lib/coterie/private-uploads /var/lib/coterie/pre-restore/ 2>/dev/null || true

# 4. Extract the bundle. All three components, not just the database —
#    restoring the database alone leaves rows pointing at missing files.
sudo tar -xzf "$BUNDLE" -C /var/lib/coterie

# 5. Fix ownership (tar ran as root)
sudo chown -R coterie:coterie \
    /var/lib/coterie/coterie.db \
    /var/lib/coterie/uploads \
    /var/lib/coterie/private-uploads
sudo chmod 0640 /var/lib/coterie/coterie.db

# 6. Sanity-check the database
sudo -u coterie sqlite3 /var/lib/coterie/coterie.db 'PRAGMA integrity_check;'
# Expect: ok — if not, do NOT start; try an older bundle.

# 7. Start
sudo systemctl start coterie
sudo journalctl -u coterie -n 50

# 8. Once you've confirmed everything works:
sudo rm -rf /var/lib/coterie/pre-restore
```

Login pages and member portal should be reachable within seconds.

---

## 6. Existing deployments: install the backup timer

The provisioning wizard installs and enables the backup timer, but
**provisioning only covers new hosts**. Any deployment created before
that landed has `backup.sh` sitting on disk with nothing scheduling it.
Check first — this is the answer to "do I have backups?", and it comes
from the service manager rather than from files existing on disk:

```bash
systemctl list-timers coterie-backup.timer
```

Nothing listed means no backups are running. Install them:

```bash
sudo cp /opt/coterie/deploy/coterie-backup.service /etc/systemd/system/
sudo cp /opt/coterie/deploy/coterie-backup.timer   /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now coterie-backup.timer

# Confirm, then prove it works rather than assuming:
systemctl list-timers coterie-backup.timer
sudo systemctl start coterie-backup.service
journalctl -u coterie-backup -n 20
ls -la /var/lib/coterie/backups/daily/
```

---

## 7. Validating the restore

After the service is up, click through:

- `/login` — log in as a known admin
- `/portal/admin/members` — member list matches expectations
- **an event or announcement with an image** — the image renders. This
  is the check that a database-only restore would fail.
- **a submission with an attachment** — the attachment downloads from
  `/portal/submissions/<id>/attachment`
- `/portal/admin/audit` — last entry's timestamp matches when you'd
  expect (i.e. shortly before the backup was taken)
- `/portal/admin/settings/email` — SMTP password should be intact
  (it's encrypted with a key derived from `session_secret`; if you
  also rotated the secret as part of the restore, you'll need to
  re-enter the SMTP password)
- A test payment in Stripe test mode if Stripe is configured

If the audit log's last entry is much older than expected, you may
have grabbed a stale bundle. Check newer ones.

---

## 8. This is not the same thing as a droplet snapshot

If your provider takes disk snapshots (DigitalOcean droplet backups,
EBS snapshots, and so on), keep them. They cover a different failure.

Provider snapshots are **disaster recovery**: the host is gone and you
need it back. They are a poor fit for everything else:

- **Granularity.** Restoring one file, or yesterday's database, means
  standing up a whole VM from an image and copying out of it.
- **Consistency.** A disk snapshot of a live SQLite database in WAL
  mode is crash-consistent, not application-consistent. SQLite usually
  recovers. "Usually" is doing real work in that sentence — `VACUUM
  INTO` makes consistency a property of how the artifact was made.
- **Retention depth.** Weekly snapshots with a handful of slots will
  not answer "a member was deleted three weeks ago."
- **Same failure domain.** Same provider, same account, same billing
  relationship as the thing being protected.
- **Portability.** A DO snapshot restores to a DO droplet. A bundle
  restores anywhere — which is why `MIGRATION.md` uses it.

This feature is **operational recovery and portability**. Neither
covers the other's cases; run both.

---

## Troubleshooting

**`integrity_check` says anything other than `ok`.**
The snapshot itself is corrupt. The script stops there without starting
the service, and your previous data is still in `pre-restore-*`. Try
the next-oldest bundle.

**`restore.sh` says the bundle has no `uploads/` member.**
It isn't a bundle from this version of `backup.sh` — most likely an old
`*.db.gz` database-only snapshot. Those still restore by hand: `gunzip
-c old.db.gz > /var/lib/coterie/coterie.db` and follow section 5 from
step 5, but understand that images and attachments taken after that
snapshot's era are not in it and will 404.

**`restore.sh` refuses to run.**
It requires root — it stops a systemd unit, writes into the data dir,
and chowns to the service account. Re-run it with `sudo`.

**Service starts but dues calculations look wrong.**
Compare `dues_paid_until` for a sample of members against what you
expect. If the bundle is from before a recent payment, that payment
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
current secret. (See `OPS.md` for the full secret-rotation
context.)

**Stripe webhooks failing after restore.**
The `processed_stripe_events` table is part of the snapshot, so
Stripe retries since the snapshot will be deduplicated correctly.
Webhooks for events created during the gap (between backup and
restore) will arrive and be processed normally on Stripe's retry
schedule (~3 days).
