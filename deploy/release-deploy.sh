#!/bin/sh
# Pull a tagged Coterie release from GitHub and install it.
#
# Usage:
#   release-deploy.sh                 # install the latest release
#   release-deploy.sh v1.2.3          # install a specific tag (rollback)
#
# Assumes:
#   - /opt/coterie exists and is owned by the coterie user
#   - The systemd unit (or OpenRC service) is named "coterie"
#   - `curl`, `python3`, `tar`, `sha256sum` are installed
#
# Idempotent: if the requested release is already installed (matches
# /opt/coterie/VERSION), exits 0 without restarting the service.

set -eu

REPO="IndustriousKraken/coterie"
INSTALL_DIR="/opt/coterie"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

# Resolve the requested tag (latest if no arg).
if [ $# -ge 1 ]; then
    TAG="$1"
    API_URL="https://api.github.com/repos/$REPO/releases/tags/$TAG"
else
    API_URL="https://api.github.com/repos/$REPO/releases/latest"
fi

echo "Querying GitHub for release: ${TAG:-latest}"
# Fetch and parse with Python rather than jq. GitHub's API
# occasionally returns release bodies with unescaped control bytes
# (raw newlines lifted from commit messages, etc.) which strict JSON
# parsers like jq reject. Python's json module is equally strict by
# default but accepts `strict=False` which tolerates the cases we
# see in practice. Python is in Debian's base install so this isn't
# an extra dep beyond what jq was.
RELEASE_JSON_FILE="$TMP_DIR/release.json"
curl -sfL "$API_URL" > "$RELEASE_JSON_FILE" || {
    echo "ERROR: couldn't fetch release info from $API_URL"
    exit 1
}

# Extract tag_name. Using Python with strict=False so embedded
# raw control chars in the release body don't blow us up.
TAG="$(python3 -c "
import json, sys
with open('$RELEASE_JSON_FILE') as f:
    data = json.load(f, strict=False)
print(data['tag_name'])
")"
echo "Resolved release: $TAG"

# Skip if this version is already installed.
if [ -f "$INSTALL_DIR/VERSION" ]; then
    CURRENT="$(head -n 1 "$INSTALL_DIR/VERSION")"
    if [ "$CURRENT" = "$TAG" ]; then
        echo "Already on $TAG, nothing to do."
        exit 0
    fi
fi

# Find the tarball + checksum URLs.
TARBALL_URL="$(python3 -c "
import json, re
with open('$RELEASE_JSON_FILE') as f:
    data = json.load(f, strict=False)
for a in data['assets']:
    if re.match(r'coterie-.*-x86_64-linux-musl\.tar\.gz\$', a['name']):
        print(a['browser_download_url'])
        break
")"
CHECKSUM_URL="$(python3 -c "
import json
with open('$RELEASE_JSON_FILE') as f:
    data = json.load(f, strict=False)
for a in data['assets']:
    if a['name'].endswith('.sha256'):
        print(a['browser_download_url'])
        break
")"

if [ -z "$TARBALL_URL" ] || [ -z "$CHECKSUM_URL" ]; then
    echo "ERROR: couldn't find release assets in $TAG"
    exit 1
fi

# Download both.
cd "$TMP_DIR"
echo "Downloading $TARBALL_URL"
curl -sfL -O "$TARBALL_URL"
echo "Downloading $CHECKSUM_URL"
curl -sfL -O "$CHECKSUM_URL"

# Verify checksum.
sha256sum -c ./*.sha256

# Extract.
TARBALL="$(ls coterie-*.tar.gz)"
tar xzf "$TARBALL"
STAGE_DIR="$(basename "$TARBALL" .tar.gz)"

# Detect first-install vs. update by checking if the binary is
# already in place. First-install path skips the service stop/start
# dance since the unit doesn't exist yet — instead it runs
# install.sh from the extracted tarball after files are placed.
FIRST_INSTALL=false
if [ ! -f "$INSTALL_DIR/coterie" ]; then
    FIRST_INSTALL=true
    echo "First install detected (no existing binary at $INSTALL_DIR/coterie)."
fi

# Pick a service manager for later use. Default to systemd on
# first-install since install.sh installs a systemd unit; the
# OpenRC path is a TODO if we ever publish an Alpine-flavored
# install.sh.
if command -v systemctl >/dev/null 2>&1; then
    SERVICE_MGR="systemd"
elif command -v rc-service >/dev/null 2>&1; then
    SERVICE_MGR="openrc"
else
    echo "ERROR: no service manager found (systemd or openrc)"
    exit 1
fi

# Stop the service so we can swap files cleanly. Skipped on
# first-install (no unit to stop).
if [ "$FIRST_INSTALL" = "false" ]; then
    if [ "$SERVICE_MGR" = "systemd" ]; then
        systemctl stop coterie
    else
        rc-service coterie stop
    fi
fi

# Ensure the install dir exists. On first install this creates it;
# on update it's a no-op. install.sh later sets correct ownership.
mkdir -p "$INSTALL_DIR"

# Atomic file swaps where it matters. We replace the binaries
# specifically and rsync the supporting files. Anything in
# $INSTALL_DIR that's NOT in the release stays (specifically: .env
# stays, /var/lib/coterie data is untouched — it's not under
# $INSTALL_DIR anyway).
install -m 0755 "$STAGE_DIR/coterie" "$INSTALL_DIR/coterie.new"
install -m 0755 "$STAGE_DIR/seed"    "$INSTALL_DIR/seed.new"
mv "$INSTALL_DIR/coterie.new" "$INSTALL_DIR/coterie"
mv "$INSTALL_DIR/seed.new"    "$INSTALL_DIR/seed"

# Static, migrations, deploy: just replace wholesale.
rm -rf "$INSTALL_DIR/static" "$INSTALL_DIR/migrations"
cp -r "$STAGE_DIR/static"      "$INSTALL_DIR/"
cp -r "$STAGE_DIR/migrations"  "$INSTALL_DIR/"

# Keep .env.example up to date so operators can diff against their
# live .env after upgrades to see what new settings exist. NEVER
# touches .env itself.
if [ -f "$STAGE_DIR/.env.example" ]; then
    cp -f "$STAGE_DIR/.env.example" "$INSTALL_DIR/.env.example"
fi
# deploy/ scripts are kept up-to-date too, but don't overwrite a
# possibly-modified config or install.sh aggressively — copy each
# file individually so operators can pin local changes if needed.
mkdir -p "$INSTALL_DIR/deploy"
cp -f "$STAGE_DIR/deploy/"*.sh        "$INSTALL_DIR/deploy/" 2>/dev/null || true
cp -f "$STAGE_DIR/deploy/"*.service   "$INSTALL_DIR/deploy/" 2>/dev/null || true
cp -f "$STAGE_DIR/deploy/"*.openrc    "$INSTALL_DIR/deploy/" 2>/dev/null || true
cp -f "$STAGE_DIR/deploy/"*.timer     "$INSTALL_DIR/deploy/" 2>/dev/null || true
cp -f "$STAGE_DIR/deploy/Caddyfile.example" "$INSTALL_DIR/deploy/" 2>/dev/null || true

# Record the version.
cp "$STAGE_DIR/VERSION" "$INSTALL_DIR/VERSION"

if [ "$FIRST_INSTALL" = "true" ]; then
    # On first install: run install.sh (creates coterie user,
    # /var/lib/coterie, systemd unit). install.sh chowns $INSTALL_DIR
    # itself but doesn't recurse into it, so we chown the contents
    # afterwards.
    if [ ! -f "$INSTALL_DIR/deploy/install.sh" ]; then
        echo "ERROR: $INSTALL_DIR/deploy/install.sh not found"
        echo "Cannot complete first-install bootstrap."
        exit 1
    fi
    echo "Running install.sh to set up user, dirs, and systemd unit..."
    bash "$INSTALL_DIR/deploy/install.sh"

    # chown the contents we placed (install.sh only chowns the dir).
    if id coterie >/dev/null 2>&1; then
        chown -R coterie:coterie "$INSTALL_DIR"
    fi

    cat <<EOF

============================================================
First-install bootstrap complete. Next steps:

  1. Create /opt/coterie/.env from .env.example and fill in
     required settings (database URL, session secret, Stripe
     keys, etc.). See deploy/SETUP.md for the field reference.

       cp /opt/coterie/.env.example /opt/coterie/.env
       chown coterie:coterie /opt/coterie/.env
       chmod 0640 /opt/coterie/.env

  2. Configure Caddy (see deploy/Caddyfile.example) and DNS.

  3. Start the service:
       systemctl start coterie
       systemctl enable coterie    # start on boot
       systemctl status coterie

  4. Visit https://your-domain/setup to create the first admin.

For subsequent updates, just re-run this script — it'll do the
service-stop/swap/restart dance correctly once the unit exists.
============================================================
EOF
else
    # Update path: chown then restart.
    if id coterie >/dev/null 2>&1; then
        chown -R coterie:coterie "$INSTALL_DIR"
    fi
    if [ "$SERVICE_MGR" = "systemd" ]; then
        systemctl start coterie
        systemctl status coterie --no-pager
    else
        rc-service coterie start
        rc-service coterie status
    fi
fi

echo "Installed Coterie $TAG"
