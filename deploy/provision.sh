#!/usr/bin/env bash
# Bootstrap script for the Coterie provisioning wizard.
#
# What this does (and only this):
#   1. Refuses to run if not root or not Debian/Ubuntu.
#   2. Looks up the latest stable Coterie release (or `--tag <v...>`).
#   3. Downloads the coterie-provision musl tarball + its .sha256.
#   4. Verifies the checksum.
#   5. Extracts and `exec`s coterie-provision install "$@".
#
# Everything else (interactive prompts, system setup, .env, Caddyfile,
# create_admin, smoke test) lives in the Rust binary it `exec`s.
#
# Usage:
#   curl -sfL https://raw.githubusercontent.com/IndustriousKraken/coterie/master/deploy/provision.sh -o /tmp/provision.sh
#   sudo bash /tmp/provision.sh                    # default: latest stable
#   sudo bash /tmp/provision.sh --tag v1.0.0       # pin a specific tag
#   sudo bash /tmp/provision.sh --no-prompt        # IaC mode (env vars + flags)

set -euo pipefail

trap 'echo "ERROR: provision.sh failed at line $LINENO" >&2' ERR

REPO="IndustriousKraken/coterie"
TAG=""
PASSTHROUGH=()

# Extract --tag <v...> for our own use; everything else is forwarded
# to coterie-provision install.
while [ $# -gt 0 ]; do
    case "$1" in
        --tag)
            TAG="$2"
            shift 2
            ;;
        --tag=*)
            TAG="${1#--tag=}"
            shift
            ;;
        *)
            PASSTHROUGH+=("$1")
            shift
            ;;
    esac
done

# Root check.
if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: provision.sh must run as root (sudo bash provision.sh ...)" >&2
    exit 1
fi

# OS check.
if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    case "${ID:-}" in
        debian|ubuntu) ;;
        *)
            echo "ERROR: only Debian/Ubuntu is supported (got ID=${ID:-unknown})" >&2
            exit 1
            ;;
    esac
else
    echo "ERROR: /etc/os-release missing; cannot identify OS" >&2
    exit 1
fi

# Resolve tag if not provided.
if [ -z "$TAG" ]; then
    API_URL="https://api.github.com/repos/$REPO/releases/latest"
    TAG="$(curl -sfL "$API_URL" \
        | python3 -c "import json,sys; print(json.load(sys.stdin, strict=False)['tag_name'])")"
    if [ -z "$TAG" ]; then
        echo "ERROR: could not resolve latest stable release tag" >&2
        exit 1
    fi
fi

echo "Bootstrapping coterie-provision $TAG ..."

ASSET="coterie-provision-${TAG}-x86_64-unknown-linux-musl.tar.gz"
ASSET_URL="https://github.com/$REPO/releases/download/$TAG/$ASSET"
SHA_URL="${ASSET_URL}.sha256"

WORK_DIR="$(mktemp -d -t coterie-provision-XXXXXX)"
trap 'rm -rf "$WORK_DIR"' EXIT

cd "$WORK_DIR"
echo "Downloading $ASSET ..."
curl -sfL -o "$ASSET" "$ASSET_URL"
echo "Downloading ${ASSET}.sha256 ..."
curl -sfL -o "${ASSET}.sha256" "$SHA_URL"

echo "Verifying SHA256 ..."
sha256sum -c "${ASSET}.sha256"

echo "Extracting ..."
tar -xzf "$ASSET"

# The tarball contains the bare `coterie-provision` binary.
BIN="$WORK_DIR/coterie-provision"
if [ ! -x "$BIN" ]; then
    BIN="$(find "$WORK_DIR" -maxdepth 3 -type f -name coterie-provision -perm -u+x | head -n 1 || true)"
fi
if [ -z "$BIN" ] || [ ! -x "$BIN" ]; then
    echo "ERROR: coterie-provision binary not found inside $ASSET" >&2
    exit 1
fi

echo "Launching coterie-provision install ..."
exec "$BIN" install "${PASSTHROUGH[@]}"
