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
#   - `curl`, `jq`, `tar`, `sha256sum` are installed
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
RELEASE_JSON="$(curl -sfL "$API_URL")" || {
    echo "ERROR: couldn't fetch release info from $API_URL"
    exit 1
}

TAG="$(echo "$RELEASE_JSON" | jq -r '.tag_name')"
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
TARBALL_URL="$(echo "$RELEASE_JSON" | jq -r \
    '.assets[] | select(.name | test("coterie-.*-x86_64-linux-musl\\.tar\\.gz$")) | .browser_download_url')"
CHECKSUM_URL="$(echo "$RELEASE_JSON" | jq -r \
    '.assets[] | select(.name | endswith(".sha256")) | .browser_download_url')"

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

# Stop the service so we can swap files cleanly. The brief downtime
# is fine; a more elaborate setup could use a blue/green dance but
# that's overkill for a single-instance deploy.
if command -v systemctl >/dev/null 2>&1; then
    SERVICE_MGR="systemd"
    systemctl stop coterie
elif command -v rc-service >/dev/null 2>&1; then
    SERVICE_MGR="openrc"
    rc-service coterie stop
else
    echo "ERROR: no service manager found (systemd or openrc)"
    exit 1
fi

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

# Fix ownership in case any file landed as root.
if id coterie >/dev/null 2>&1; then
    chown -R coterie:coterie "$INSTALL_DIR"
fi

# Restart.
if [ "$SERVICE_MGR" = "systemd" ]; then
    systemctl start coterie
    systemctl status coterie --no-pager
else
    rc-service coterie start
    rc-service coterie status
fi

echo "Installed Coterie $TAG"
