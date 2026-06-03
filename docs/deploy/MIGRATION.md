# Migrating Coterie between hosts

The point of having both `DEPLOY-DIGITALOCEAN.md` and `DEPLOY-AWS.md`
isn't symmetry for symmetry's sake — it's so this runbook exists.
When you need to move Coterie (provider failure, cost optimization,
acquired company's infrastructure, geography), the migration is
mechanical, not improvisational.

This doc covers:

1. The shared concept (what actually moves between hosts)
2. DigitalOcean → AWS
3. AWS → DigitalOcean
4. Verification + cutover
5. Rollback plan

The procedure is the same in both directions. The two sections below
just call out provider-specific commands.

---

## 1. What moves

Everything Coterie needs lives in **three places**:

| Where             | What                                       | How to move           |
| ----------------- | ------------------------------------------ | --------------------- |
| DB file           | `/var/lib/coterie/coterie.db`         | `VACUUM INTO` snapshot |
| Uploads           | `/var/lib/coterie/uploads/`           | rsync / tar           |
| `.env`            | `/opt/coterie/.env`                        | scp                   |

**Not in scope** — none of these need to move:

- The binary (build fresh on the new host or pull a versioned image)
- `/opt/coterie/deploy/` (re-cloned from the repo)
- systemd units (re-installed by `install.sh`)
- Caddy config (regenerated from `Caddyfile.example`)
- TLS certs (Caddy provisions fresh ones the moment DNS points at the
  new host)

---

## 2. DigitalOcean → AWS

### 2a. Provision the new host

Follow `DEPLOY-AWS.md` sections 1–7 to bring up the new EC2 instance,
attach the EBS volume, install Caddy + Coterie, and create `.env`.

**Stop before step 8 (DNS).** You don't want DNS to flip until the
new host has the data. The new instance should be running with an
empty database — that's fine, you'll overwrite it shortly.

```bash
# On the new (AWS) host, stop Coterie so we can replace its DB
sudo systemctl stop coterie
```

### 2b. Take the migration snapshot on the old host

```bash
# On the old (DigitalOcean) host
sudo systemctl stop coterie

# Take a clean snapshot
sudo -u coterie sqlite3 /var/lib/coterie/coterie.db \
    "VACUUM INTO '/tmp/coterie-migrate.db'"

# Tar up the uploads directory alongside
sudo tar czf /tmp/coterie-uploads.tar.gz -C /var/lib/coterie uploads

# Grab the .env (you'll edit it on the new host before bringing it up)
sudo cp /opt/coterie/.env /tmp/coterie.env

# Optional: re-start the old service so it stays serving until DNS flips
sudo systemctl start coterie
```

If the old host has a recent backup in `/var/lib/coterie/backups/`
that will work too — but a fresh `VACUUM INTO` taken right before
cutover gives you the smallest gap.

### 2c. Ship the files to the new host

```bash
# From your laptop:
scp root@old-do-host:/tmp/coterie-migrate.db    /tmp/
scp root@old-do-host:/tmp/coterie-uploads.tar.gz /tmp/
scp root@old-do-host:/tmp/coterie.env            /tmp/

scp /tmp/coterie-migrate.db    ubuntu@new-aws-host:/tmp/
scp /tmp/coterie-uploads.tar.gz ubuntu@new-aws-host:/tmp/
scp /tmp/coterie.env            ubuntu@new-aws-host:/tmp/
```

(`scp` directly host-to-host works too if you have the keys forwarded
and the firewalls allow it. Going through your laptop is the
straightforward path.)

### 2d. Install on the new host

```bash
# On the new (AWS) host:
sudo -i

# 1. Database
mv /tmp/coterie-migrate.db /var/lib/coterie/coterie.db
chown coterie:coterie /var/lib/coterie/coterie.db
chmod 0640 /var/lib/coterie/coterie.db

# Sanity-check
sudo -u coterie sqlite3 /var/lib/coterie/coterie.db 'PRAGMA integrity_check;'
# Expect: ok

# 2. Uploads
tar xzf /tmp/coterie-uploads.tar.gz -C /var/lib/coterie
chown -R coterie:coterie /var/lib/coterie/uploads

# 3. .env
# IMPORTANT: keep the OLD session_secret. Rotating it during a
# migration breaks the encrypted SMTP password and forces re-entry.
# But UPDATE base_url if the URL is changing as part of the move.
mv /tmp/coterie.env /opt/coterie/.env
chown coterie:coterie /opt/coterie/.env
chmod 0640 /opt/coterie/.env

# Edit if any host-specific paths differ
nano /opt/coterie/.env
# - COTERIE__SERVER__DATA_DIR is /var/lib/coterie on both hosts (no change)
# - COTERIE__SERVER__BASE_URL stays the same (DNS will flip)

# 4. Bring it up
systemctl start coterie
journalctl -u coterie -f
```

**Don't flip DNS yet.** Coterie is now running on the new host with
the migrated data, but the old host is still authoritative for
external traffic.

### 2e. Verify, then flip DNS

```bash
# Quick smoke test from your laptop, hitting the new host directly
# via /etc/hosts (won't have a valid TLS cert yet because DNS hasn't
# flipped — use --resolve in curl)
curl --resolve coterie.example.com:443:<new-aws-ip> \
    https://coterie.example.com/health
# {"status":"ok"}
```

The TLS cert won't validate until you flip DNS — Caddy can't
provision the cert until the ACME challenge succeeds, which requires
DNS pointing at the new host. The `curl --resolve` trick lets you
verify the app responds; the cert error is expected.

When you're satisfied, flip DNS:

```bash
# Route 53:
aws route53 change-resource-record-sets ...        # see DEPLOY-AWS.md §8
# DigitalOcean:
doctl compute domain records update example.com --record-id <id> \
    --record-data <new-aws-ip>
```

Lower the TTL **before** you start the migration (24h ahead is good)
so the propagation window is short. Reset to 300+ once stable.

After DNS propagates (~minutes with low TTL), Caddy on the new host
provisions a fresh cert automatically. Watch:

```bash
sudo journalctl -u caddy -f
```

You should see:

```
... certificate obtained successfully ... coterie.example.com
```

### 2f. Decommission the old host

After **at least 24 hours** of confirmed-good operation on the new
host (full audit-log accrual, payment webhooks landing, dues
reminder cycle if it falls in the window):

```bash
# Old DO host
sudo systemctl stop coterie
# Take a final cold backup before deleting
sudo -u coterie sqlite3 /var/lib/coterie/coterie.db \
    "VACUUM INTO '/tmp/coterie-final-snapshot.db'"
scp root@old-do-host:/tmp/coterie-final-snapshot.db .  # to your laptop, archive somewhere
```

Then in the DO console: destroy droplet, detach + destroy volume.
Keep the final snapshot for at least a quarter — if some edge-case
data corruption surfaces later, this is your last-resort recovery
point.

---

## 3. AWS → DigitalOcean

The procedure is identical in structure. Provision new (DO) host
following `DEPLOY-DIGITALOCEAN.md` sections 1–8, then run sections
2b–2f above with the directions reversed.

The only meaningful provider-specific difference: instead of
managing an Elastic IP on AWS, DigitalOcean droplets get a public IP
from the start. The DNS flip looks the same.

---

## 4. Cutover checklist

Print this. Tick it off during the migration.

- [ ] DNS TTL lowered to 60–300 at least 1h before cutover
- [ ] Old host: `systemctl stop coterie`
- [ ] Old host: `VACUUM INTO` snapshot taken
- [ ] Old host: uploads tar'd
- [ ] Old host: .env copied
- [ ] New host: snapshot, uploads, .env transferred
- [ ] New host: file ownership/permissions correct (`coterie:coterie`, 0640)
- [ ] New host: `PRAGMA integrity_check` returns `ok`
- [ ] New host: `systemctl start coterie` succeeded
- [ ] New host: `/health` responds via `curl --resolve`
- [ ] DNS flipped at registrar / Route 53
- [ ] DNS propagation verified (`dig +short` from external machine)
- [ ] TLS cert reissued by Caddy on new host (journalctl shows it)
- [ ] Logged in as admin via real https URL, member list intact
- [ ] Test payment ran end-to-end (Stripe test mode)
- [ ] Backup timer running on new host (`systemctl list-timers`)
- [ ] Old host left running for 24 hours as a safety net
- [ ] Old host destroyed, final snapshot archived

---

## 5. Rollback plan

If something goes wrong **before DNS flip**: stop the new Coterie,
restore the old host normally (it's still running). Total user impact:
zero.

If something goes wrong **after DNS flip but within the propagation
window** (~minutes): flip DNS back to the old host. Most users still
have the old IP cached; partial users on the new IP will see brief
errors. Old host's DB is still authoritative.

If something goes wrong **hours later**: the gap between the
migration snapshot and the rollback is data lost on the old host.
Two options:

1. Accept the gap. Roll DNS back, restart old host. Anything members
   did during the window is gone — apologize via email.
2. Reverse the migration. Take a `VACUUM INTO` snapshot of the new
   host (which has the recent data), restore it onto the old host
   following section 2 in reverse. Then flip DNS back. Total downtime
   is one cycle of the migration — typically 15–30 minutes.

Option 2 is preferable when the new host has accumulated meaningful
new state (payments, signups, audit entries) since cutover.

---

## Common mistakes

**Forgetting to update DNS TTL ahead of time.** Default TTL is often
24h or longer. If you flip DNS without lowering TTL first, some users
will hit the old (now-stopped) host for a full TTL after cutover.
Lower it to 60–300 the day before.

**Rotating `session_secret` during the migration.** Don't. The
encrypted SMTP password in the DB is keyed by it. Migrate first,
rotate later (and re-enter the SMTP password through the admin UI).

**Skipping `PRAGMA integrity_check` after the file transfer.** A
truncated transfer can produce a file SQLite opens but reads garbage
from. Always run `integrity_check` before starting the service on
the new host.

**Forgetting the uploads directory.** Members' profile photos,
event banners, etc. live there — the DB only stores filenames. If
you skip the uploads tarball, the portal renders broken images.

**Not stopping the old service first.** Snapshotting a live SQLite
file via `VACUUM INTO` is safe (that's the point of the command) —
but the gap between when you take the snapshot and when you cut over
is data the new host won't have. Stopping the old service first
makes the gap zero.
