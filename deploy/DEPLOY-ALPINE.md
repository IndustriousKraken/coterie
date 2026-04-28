# Deploying Coterie on Alpine Linux

End-to-end walkthrough for running Coterie on Alpine Linux as a
native service (no Docker), using OpenRC for process supervision and
crond for scheduled backups.

Why Alpine: small footprint (~5 MB base), no glibc surface area,
independent of the Debian/Ubuntu release cycle. Coterie is fully
rustls (no OpenSSL link), so musl-static binaries build cleanly.

This guide parallels `DEPLOY-DIGITALOCEAN.md` and `DEPLOY-AWS.md`.
The main differences:

| Step | Ubuntu/Debian path | Alpine path |
| ---- | ------------------ | ----------- |
| Init | systemd            | OpenRC      |
| Build deps | `apt-get install build-essential pkg-config libssl-dev` | `apk add build-base sqlite-dev` |
| Caddy | apt repo from Cloudsmith | `apk add caddy` |
| Backup schedule | systemd timer | crontab |
| Launcher | `EnvironmentFile=` directive | `deploy/run.sh` wrapper sources `.env` |

Provider-agnostic — the host can be a DO droplet, AWS Lightsail,
Hetzner, a bare-metal box, etc.

Time to first running instance: about 60 minutes (the build is the
slow part — 5–10 min on a small VM).

---

## 0. Prerequisites

- An Alpine 3.20+ host (this guide assumes systemd is NOT in use)
- Root SSH access
- A domain pointing at the host

---

## 1. Bootstrap the system

```sh
# All commands below run as root unless noted. Switch to root with
# `doas su -` or `sudo -i` (Alpine ships doas by default; sudo is
# in the community repo if you prefer it).

# Update package index
apk update
apk upgrade

# Core packages
apk add --no-cache \
    ca-certificates \
    sqlite \
    curl \
    bash \
    openrc \
    logrotate

# Caddy (Alpine community repo)
apk add --no-cache caddy

# Optional: a friendlier shell + editor than the BusyBox defaults
# apk add --no-cache nano htop tmux
```

OpenRC is already installed and active on Alpine; the package above
just makes that explicit.

---

## 2. Create system user and directories

```sh
addgroup -S coterie
adduser -S -D -H -G coterie -h /opt/coterie -s /sbin/nologin coterie

install -d -m 0755 -o coterie -g coterie /opt/coterie
install -d -m 0750 -o coterie -g coterie /var/lib/coterie
install -d -m 0750 -o coterie -g coterie /var/lib/coterie/backups
install -d -m 0750 -o coterie -g coterie /var/log/coterie
```

---

## 3. Build the Coterie binary

Two options.

### Option A: build natively on the Alpine host

Since Alpine's native toolchain produces musl-linked binaries, this
"just works" — no cross-compile dance. It does need ~2 GB free for
the cargo build cache.

```sh
# Build dependencies. sqlite-dev is needed by sqlx's macro evaluation.
apk add --no-cache build-base sqlite-dev git

# Install rustup (preferred over apk's rust package — newer, easier
# to manage toolchain versions)
adduser -D builder
su -l builder

# As builder:
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable
. "$HOME/.cargo/env"

git clone https://github.com/your-org/coterie ~/coterie-build
cd ~/coterie-build
make release        # builds tailwind CSS + cargo build --release

# Back to root to install
exit
install -m 0755 -o coterie -g coterie \
    /home/builder/coterie-build/target/release/coterie \
    /opt/coterie/coterie
cp -r /home/builder/coterie-build/static /opt/coterie/
cp -r /home/builder/coterie-build/deploy /opt/coterie/
chown -R coterie:coterie /opt/coterie/static /opt/coterie/deploy

# Optional cleanup — frees the build cache (~2 GB)
deluser --remove-home builder
```

### Option B: cross-build elsewhere, ship to the host

If you'd rather not run a Rust toolchain on the production box,
build for `x86_64-unknown-linux-musl` from your dev machine (or a
CI worker) and rsync the resulting binary.

```sh
# On your dev machine (Linux or macOS):
rustup target add x86_64-unknown-linux-musl

# Linux dev box: this just works once the target is added.
# macOS dev box: need a musl cross toolchain (e.g. `brew install
# filosottile/musl-cross/musl-cross`) and the linker setting:
#   export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc
#   export CC_x86_64_unknown_linux_musl=x86_64-linux-musl-gcc

cargo build --release --target x86_64-unknown-linux-musl
make css

rsync -avz \
    target/x86_64-unknown-linux-musl/release/coterie \
    static/ \
    deploy/ \
    root@<alpine-host>:/opt/coterie/
```

The output binary is fully static (`file target/.../release/coterie`
will say "statically linked") and runs on any musl host.

---

## 4. Configure `.env`

```sh
cp /opt/coterie/.env.example /opt/coterie/.env
chown coterie:coterie /opt/coterie/.env
chmod 0640 /opt/coterie/.env

# Generate a session secret
openssl rand -hex 32
# (or: head -c 32 /dev/urandom | xxd -p -c 64)

# Edit .env
vi /opt/coterie/.env       # or `apk add --no-cache nano` first
```

Minimum changes from the example:

```
COTERIE__SERVER__BASE_URL=https://coterie.example.com
COTERIE__AUTH__SESSION_SECRET=<paste the openssl rand output>
COTERIE__SERVER__DATA_DIR=/var/lib/coterie
COTERIE__DATABASE__URL=sqlite://coterie.db
```

See `.env.example` for the full set of options (Stripe, Discord, etc.).

---

## 5. Install the OpenRC service

```sh
# The wrapper script + init script
install -m 0755 -o coterie -g coterie \
    /opt/coterie/deploy/run.sh /opt/coterie/run.sh
install -m 0755 \
    /opt/coterie/deploy/coterie.openrc /etc/init.d/coterie

# Register and start
rc-update add coterie default
rc-service coterie start

# Verify
rc-service coterie status
tail -f /var/log/coterie/output.log
```

You should see Coterie's startup log:

```
Starting Coterie server on 127.0.0.1:8080
Using database: sqlite:///var/lib/coterie/coterie.db
Server listening on http://127.0.0.1:8080
```

If something goes wrong, see "Troubleshooting" at the end.

---

## 6. Configure Caddy

Identical to the Ubuntu path:

```sh
cp /opt/coterie/deploy/Caddyfile.example /etc/caddy/Caddyfile
vi /etc/caddy/Caddyfile     # update domain

caddy validate --config /etc/caddy/Caddyfile

rc-update add caddy default
rc-service caddy restart
```

Caddy on Alpine is the same Caddy as on Ubuntu — automatic TLS, same
config syntax. Provisioning the cert needs DNS already pointing at
the host and ports 80 + 443 reachable.

---

## 7. DNS

Point your domain at the host's public IP. From your laptop:

```sh
dig +short coterie.example.com
```

Wait for propagation, then visit `https://coterie.example.com` —
setup page appears, create your first admin.

---

## 8. Schedule backups (cron)

Alpine uses BusyBox `crond` (already installed and running by
default). The Coterie backup script (`deploy/backup.sh`) is portable
across BusyBox and GNU userlands — both `date -Iseconds` and
`stat -c%s` are supported on Alpine.

```sh
# (Optional) configure offsite push to S3-compatible storage.
apk add --no-cache aws-cli      # in community repo

# /etc/conf.d/coterie-backup is read by /etc/init.d when starting
# OpenRC services; for cron we just put env in a sourcable file.
cat > /etc/coterie-backup.env <<'EOF'
COTERIE_BACKUP_S3_URI=s3://my-coterie-backups/prod/
AWS_ACCESS_KEY_ID=<key>
AWS_SECRET_ACCESS_KEY=<secret>
AWS_DEFAULT_REGION=us-east-1
# For non-AWS S3: set AWS_ENDPOINT_URL_S3
# AWS_ENDPOINT_URL_S3=https://nyc3.digitaloceanspaces.com
EOF
chmod 0600 /etc/coterie-backup.env

# Place the backup script
install -m 0755 -o coterie -g coterie \
    /opt/coterie/deploy/backup.sh /opt/coterie/backup.sh

# Add to crontab — run daily at 03:30
crontab -u coterie -l 2>/dev/null > /tmp/coterie-cron 2>&1 || true
cat >> /tmp/coterie-cron <<'EOF'
30 3 * * * . /etc/coterie-backup.env && /opt/coterie/backup.sh >> /var/log/coterie/backup.log 2>&1
EOF
crontab -u coterie /tmp/coterie-cron
rm /tmp/coterie-cron

# Make sure crond is up
rc-update add crond default
rc-service crond start

# Verify the entry
crontab -u coterie -l

# Run it once immediately to confirm
sudo -u coterie sh -c '. /etc/coterie-backup.env && /opt/coterie/backup.sh'
ls -lh /var/lib/coterie/backups/daily/
```

---

## 9. Logrotate (optional but recommended)

The OpenRC init script writes Coterie's stdout to
`/var/log/coterie/output.log`. Without rotation, that file grows
forever. Alpine ships logrotate as a separate package (installed in
step 1). Drop a config:

```sh
cat > /etc/logrotate.d/coterie <<'EOF'
/var/log/coterie/*.log {
    daily
    rotate 14
    compress
    delaycompress
    missingok
    notifempty
    copytruncate
    create 0640 coterie coterie
}
EOF
```

logrotate runs from /etc/periodic/daily on Alpine; nothing else to
enable.

---

## What this skips vs the Ubuntu/Debian guides

The main `DEPLOY-DIGITALOCEAN.md` and `DEPLOY-AWS.md` use systemd's
hardening directives extensively (`ProtectSystem=strict`,
`SystemCallFilter`, namespace isolation). OpenRC has analogous
features via `cgroups` integration and the `command_user`/`chroot`
options, but they're less granular. If sandboxing is a hard
requirement, run Coterie inside a Docker container even on Alpine —
the `Dockerfile` in the repo root works on any container host.

For most member-management deployments, the OpenRC defaults plus
`coterie:coterie` user isolation, the read-only `/opt/coterie`
layout, and Caddy as a TLS-terminating front end are sufficient.

---

## Troubleshooting

**`rc-service coterie start` says "started" but `status` says
"crashed"** — check `/var/log/coterie/error.log`. Most common cause:
typo in `/opt/coterie/.env` (the `COTERIE__` prefix uses double
underscores between sections — single underscores within a single
section name).

**Build fails with `linker 'cc' not found`** — `apk add build-base`.
Alpine's default install doesn't include a C toolchain.

**Build fails with `pkg-config not found` or `failed to find sqlite3`**
— `apk add sqlite-dev pkg-config`. sqlx's compile-time macros
inspect the system sqlite3.

**Cross-build from macOS produces a binary that segfaults on Alpine** —
your local linker isn't the musl one. Set
`CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER` to a musl-aware
linker (the `musl-cross` Homebrew package provides one).

**Caddy fails ACME challenge** — same as on any other distro: DNS
hasn't propagated, or the firewall is blocking 80/443. `apk add
nmap-ncat` and `nc -zv 0.0.0.0 80` from another host to probe.

**`crond` not running** — `rc-service crond status`; if disabled,
`rc-update add crond default && rc-service crond start`. BusyBox
crond logs to `/var/log/messages` by default; tail that to confirm
backups fire on schedule.

**Backup script fails with `aws: not found`** — `apk add aws-cli`
(Alpine's package is in the `community` repo).

---

## Migrating to / from Alpine

The data layout (`/var/lib/coterie/`) is identical to the Ubuntu/AWS
guides, so `MIGRATION.md` works in both directions — Alpine ↔ Ubuntu,
Alpine ↔ AWS, etc. The differences are init system commands
(`rc-service coterie stop` instead of `systemctl stop coterie`) and
cron management (`crontab -u coterie -e` instead of editing
`coterie-backup.timer`). The DB snapshot, uploads tarball, and `.env`
move identically.
