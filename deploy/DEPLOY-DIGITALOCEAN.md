# Deploying Coterie on DigitalOcean

End-to-end walkthrough for running Coterie on a single DigitalOcean
droplet, with attached block storage for the database and DigitalOcean
Spaces (or any S3-compatible service) for offsite backups.

Target audience: an operator who has never deployed Coterie before.
Time to first running instance: about 45 minutes.

---

## What you'll have at the end

```
                       coterie.example.com
                                │
                                ▼
                       ┌────────────────────┐
                       │  DO Droplet        │
                       │  ┌──────────────┐  │
                       │  │   Caddy      │  │   (reverse proxy + TLS)
                       │  │   :443       │  │
                       │  └──────┬───────┘  │
                       │         │          │
                       │  ┌──────▼───────┐  │
                       │  │   Coterie    │  │   (systemd, :8080)
                       │  └──────┬───────┘  │
                       └─────────┼──────────┘
                                 │
                       ┌─────────▼──────────┐
                       │  Block storage     │   /var/lib/coterie
                       │  (durable, sized   │
                       │   to fit growth)   │
                       └────────────────────┘
                                 │
                                 ▼ daily 03:30
                       ┌────────────────────┐
                       │  DO Spaces         │   offsite backups
                       │  (or any S3-compat)│
                       └────────────────────┘
```

---

## 0. Prerequisites

- A DigitalOcean account
- The `doctl` CLI authenticated, OR comfort with the web console
- A domain (you'll point `coterie.example.com` at the droplet)
- A built Coterie binary (run `make release` locally) — or use the
  Docker image instead (see "Docker alternative" at the end)

---

## 1. Provision the droplet

Sizing is straightforward — Coterie is single-process, single-tenant,
SQLite, with a small memory footprint:

| Org size              | Droplet size        | Monthly cost (Apr 2026) |
| --------------------- | ------------------- | ----------------------- |
| < 200 members         | s-1vcpu-1gb         | ~$6                     |
| 200–2000 members      | s-1vcpu-2gb         | ~$12                    |
| > 2000 members        | s-2vcpu-4gb         | ~$24                    |

Region: pick one geographically near your members (lower latency on
the portal). Easiest: same region as your existing infrastructure.

Image: **Ubuntu 24.04 LTS** (or the latest LTS — these instructions
assume systemd and `apt`).

```bash
# Web console: Create → Droplets → Ubuntu 24.04 → s-1vcpu-2gb → SSH key.
# Or via doctl:
doctl compute droplet create coterie-prod \
    --image ubuntu-24-04-x64 \
    --size s-1vcpu-2gb \
    --region nyc3 \
    --ssh-keys "$(doctl compute ssh-key list --format ID --no-header | head -1)"
```

Wait ~30 seconds for it to boot, then note the public IP.

---

## 2. Attach block storage

Why separate block storage:
- Survives droplet rebuild (move volume to a new droplet, data is intact)
- Independent snapshot schedule
- Sized independently from the droplet — start small, grow as needed

**Sizing**: the SQLite file is small (a 5000-member instance with five
years of audit log is well under 1 GB). 10 GB gives massive headroom
for `data/uploads/` (event/announcement images), backups awaiting
upload, and growth. Start at 10 GB; resize later if needed (DO supports
online expansion).

```bash
# Web console: Volumes → Create → 10 GB, attach to coterie-prod.
# Or via doctl:
doctl compute volume create coterie-data \
    --size 10GiB \
    --region nyc3 \
    --fs-type ext4

doctl compute volume-action attach \
    "$(doctl compute volume list --format ID,Name --no-header | awk '/coterie-data/{print $1}')" \
    "$(doctl compute droplet list --format ID,Name --no-header | awk '/coterie-prod/{print $1}')"
```

---

## 3. SSH in, mount the volume

```bash
ssh root@<droplet-ip>

# Find the volume's device path. DO mounts volumes as /dev/sda
# (occasionally /dev/disk/by-id/scsi-0DO_Volume_coterie-data — the
# /dev/disk/by-id/ form is more stable and survives kernel renames).
ls -l /dev/disk/by-id/ | grep coterie
# scsi-0DO_Volume_coterie-data -> ../../sda

# Mount target
mkdir -p /var/lib/coterie

# Persistent mount via /etc/fstab (using by-id to avoid /dev/sdX races)
echo '/dev/disk/by-id/scsi-0DO_Volume_coterie-data /var/lib/coterie ext4 defaults,nofail,discard 0 2' \
    >> /etc/fstab
mount -a

df -h /var/lib/coterie
# /dev/sda  9.8G  ...   /var/lib/coterie
```

`nofail` is important: if the volume detaches, the droplet still
boots and Coterie will fail with a clear "data dir missing" error
instead of dropping the host into emergency mode.

---

## 4. System packages

```bash
apt-get update
apt-get install -y --no-install-recommends \
    sqlite3 \
    ca-certificates \
    curl \
    debian-keyring \
    debian-archive-keyring \
    apt-transport-https

# Caddy (official repo — Caddy auto-renews TLS certs)
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
    | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
    | tee /etc/apt/sources.list.d/caddy-stable.list
apt-get update
apt-get install -y caddy

# AWS CLI (for backups to DO Spaces — Spaces is S3-compatible)
apt-get install -y awscli
```

---

## 5. Deploy the Coterie code

You need the `coterie` binary, the `static/` directory (built CSS),
and the `deploy/` directory. The simplest way is rsync from your dev
machine:

```bash
# On your local machine, from the repo root:
make release      # builds target/release/coterie + static/style.css
rsync -avz --progress \
    target/release/coterie \
    static/ \
    deploy/ \
    .env.example \
    root@<droplet-ip>:/opt/coterie/
```

Or build on the droplet directly (slower, but no need for matching
host architecture between your laptop and the droplet):

```bash
# On the droplet:
apt-get install -y build-essential pkg-config libssl-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source $HOME/.cargo/env
git clone https://github.com/your-org/coterie /tmp/coterie-build
cd /tmp/coterie-build
make release
mkdir -p /opt/coterie
cp target/release/coterie /opt/coterie/
cp -r static deploy /opt/coterie/
cp .env.example /opt/coterie/
```

---

## 6. Run the installer

```bash
cd /opt/coterie
bash deploy/install.sh
```

The installer creates the `coterie` system user, sets up
`/var/lib/coterie/` (DB + uploads) and `/var/lib/coterie/backups/`,
and installs the systemd unit. It does not start the service —
`.env` isn't configured yet.

---

## 7. Configure `.env`

```bash
cp /opt/coterie/.env.example /opt/coterie/.env
chown coterie:coterie /opt/coterie/.env
chmod 0640 /opt/coterie/.env

# Generate session secret
SESSION_SECRET=$(openssl rand -hex 32)
echo "Session secret: $SESSION_SECRET"

# Edit .env
nano /opt/coterie/.env
```

Minimum changes from the example:

```
COTERIE__SERVER__BASE_URL=https://coterie.example.com
COTERIE__AUTH__SESSION_SECRET=<paste the openssl rand output>
COTERIE__SERVER__DATA_DIR=/var/lib/coterie
COTERIE__DATABASE__URL=sqlite://coterie.db
```

Add Stripe / Discord blocks if you're using them; see `.env.example`
for the field list.

---

## 8. Configure Caddy

```bash
cp /opt/coterie/deploy/Caddyfile.example /etc/caddy/Caddyfile
nano /etc/caddy/Caddyfile
# - Replace coterie.example.com with your domain
# - If you're not hosting a public site on this droplet, delete the
#   second site block

caddy validate --config /etc/caddy/Caddyfile
systemctl reload caddy
```

---

## 9. DNS

Point an A record for `coterie.example.com` at the droplet's public
IP. TTL 300 is fine. AAAA record optional (DO supports IPv6).

Wait for DNS to propagate. Check with:

```bash
dig +short coterie.example.com
```

---

## 10. Start Coterie

```bash
systemctl enable --now coterie
journalctl -u coterie -f
```

You should see:

```
Starting Coterie server on 127.0.0.1:8080
Using database: sqlite:///var/lib/coterie/coterie.db
Stripe payment processing disabled       (or enabled, depending on .env)
Server listening on http://127.0.0.1:8080
```

In a second terminal, watch Caddy provision the cert:

```bash
journalctl -u caddy -f
```

You should see TLS handshake success messages once the cert is issued
(usually under 30 seconds after first request to the domain).

Visit `https://coterie.example.com` — you should land on the setup
page (first-run admin creation).

---

## 11. First-run setup

In the browser at `https://coterie.example.com/setup`, create your
first admin account. After that the setup route auto-disables and
won't appear again.

Configure the rest from the admin UI:

- `/portal/admin/settings` — org name, contact email, member fees
- `/portal/admin/settings/email` — SMTP for outbound mail
- `/portal/admin/settings/discord` (if using Discord)
- `/portal/admin/settings/billing` (if using Stripe)
- `/portal/admin/types` — membership types, event types, announcement types

---

## 12. Schedule backups

```bash
# (Optional) configure offsite push to DO Spaces.
# Create a Space first via the DO web console: Spaces → Create → name
# it e.g. "my-coterie-backups", choose a region. Generate access keys
# under API → Spaces Keys.

cat > /etc/default/coterie-backup <<'EOF'
# DigitalOcean Spaces (S3-compatible)
COTERIE_BACKUP_S3_URI=s3://my-coterie-backups/prod/
AWS_ACCESS_KEY_ID=<your DO Spaces access key>
AWS_SECRET_ACCESS_KEY=<your DO Spaces secret>
AWS_DEFAULT_REGION=nyc3
AWS_ENDPOINT_URL_S3=https://nyc3.digitaloceanspaces.com
EOF
chmod 0600 /etc/default/coterie-backup

# Install timer
cp /opt/coterie/deploy/coterie-backup.service /etc/systemd/system/
cp /opt/coterie/deploy/coterie-backup.timer   /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now coterie-backup.timer

# Verify
systemctl list-timers coterie-backup.timer
# Should show next run at ~03:30 local time

# Trigger one immediately to confirm it works end-to-end
systemctl start coterie-backup.service
journalctl -u coterie-backup -n 50
ls -lh /var/lib/coterie/backups/daily/
# coterie-2026-04-27.db.gz
aws --endpoint-url https://nyc3.digitaloceanspaces.com \
    s3 ls s3://my-coterie-backups/prod/daily/
# coterie-2026-04-27.db.gz
```

You now have a daily local + offsite backup. **Once a quarter, do a
test restore** (see `RESTORE.md`) on a throwaway droplet — the only
way to know your backups work is to use them.

---

## 13. Snapshot strategy

Two layers of protection beyond Coterie's own backups:

1. **Volume snapshots** (DO web console → Volumes → coterie-data →
   Snapshots). Keep weekly for 4 weeks. Cost: pennies. Use for
   restoring after a fat-finger of the volume itself.
2. **Droplet snapshots** are heavier and not strictly needed —
   the droplet has nothing on it that can't be reproduced from
   `/opt/coterie/`. Skip unless you've made manual config changes
   you'd rather not redo.

---

## Docker alternative (steps 5–7)

If you'd rather run Coterie as a container (e.g. for parity with a
local dev container or for easier upgrades):

```bash
# On the droplet:
apt-get install -y docker.io
systemctl enable --now docker

# Pull or build
docker pull ghcr.io/your-org/coterie:latest
# OR build from source:
git clone https://github.com/your-org/coterie /tmp/coterie-build
cd /tmp/coterie-build && docker build -t coterie:latest .

# Place .env at /opt/coterie/.env (same as systemd flow above)
mkdir -p /opt/coterie /var/lib/coterie
# (write .env as in step 7)

# Run
docker run -d --name coterie --restart unless-stopped \
    --env-file /opt/coterie/.env \
    -p 127.0.0.1:8080:8080 \
    -v /var/lib/coterie:/data \
    coterie:latest

# Caddy config (step 8) is identical — it still proxies to 127.0.0.1:8080.
```

Backups still run on the host (the systemd timer + script). The
container exposes the SQLite file via the bind mount, so the host's
backup script reads it directly.

---

## Troubleshooting

**`bind: address already in use`** — another service is on 8080.
Either stop it or set `COTERIE__SERVER__PORT=8081` and update the
Caddyfile to match.

**Caddy says "challenge failed"** — DNS hasn't propagated yet, or your
firewall is blocking inbound 80/443. DigitalOcean's UFW defaults
allow these on Ubuntu 24.04, but a custom cloud-firewall might not.

**Coterie logs `Failed to load configuration`** — look at the next
line; it names the missing field. 90% of the time it's a typo in
`COTERIE__` (single underscore between sections — must be double).

**Backup timer doesn't fire** — check `systemctl list-timers
coterie-backup.timer`. If it's listed, the schedule is just waiting.
If `Failed to start` is in the log, the script probably can't read
`COTERIE_DB`; verify the path matches your `.env` `DATA_DIR` setting.
