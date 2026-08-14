#!/usr/bin/env bash
# Coterie update bootstrap.
#
# Downloads the `coterie-provision` binary for the latest stable release,
# verifies its SHA256, and execs `coterie-provision update`, which does
# the hardened in-place update — pre-update DB snapshot, atomic binary
# swap with previous-binary retention, post-restart /health check, and
# automatic rollback — in testable Rust rather than in bash.
#
# Curl-and-bash invocation:
#   curl -sfL https://raw.githubusercontent.com/IndustriousKraken/coterie/master/deploy/update.sh \
#       -o /tmp/update.sh
#   sudo bash /tmp/update.sh                 # update to the latest stable release
#   sudo bash /tmp/update.sh --tag v1.2.3    # pin / roll back to a specific tag
#
# All flags (including --tag) are forwarded verbatim to
# `coterie-provision update`. The downloaded provision binary is always
# the latest stable so the update logic itself is current even when
# rolling Coterie back to an older tag. It is NOT installed: it lives in
# a temp dir for the length of the run, which is why an installed host
# has no /opt/coterie/coterie-provision.
#
# This script is bash, not POSIX sh — the `trap ... ERR` below is a bash
# pseudo-signal. Run it directly (it is executable) or with `bash`.
# Running it with `sh` fails on Debian, where /bin/sh is dash:
# "trap: 26: bad trap". tests/deploy_script_interpreter_test.rs enforces
# that no script in this repo invokes another with a contradicting
# interpreter.

set -euo pipefail

REPO="IndustriousKraken/coterie"
TARGET_TRIPLE="x86_64-unknown-linux-musl"

trap 'echo "[update.sh] ERROR on line $LINENO — exit $?" >&2' ERR

if [ "$(id -u)" -ne 0 ]; then
    echo "[update.sh] must run as root (try sudo)" >&2
    exit 1
fi

for bin in curl tar sha256sum; do
    command -v "$bin" >/dev/null 2>&1 || {
        echo "[update.sh] missing required tool: $bin" >&2
        exit 1
    }
done

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

# Resolve the latest stable release tag for the provision binary via the
# `releases/latest` redirect — no python3, no JSON parsing. GitHub's
# "latest" excludes prereleases, so this is the latest *stable* tag.
echo "[update.sh] resolving latest stable coterie-provision release..."
LATEST_URL="$(curl -fsSL -o /dev/null -w '%{url_effective}' \
    "https://github.com/${REPO}/releases/latest")"
PROVISION_TAG="${LATEST_URL##*/tag/}"
if [ -z "$PROVISION_TAG" ] || [ "$PROVISION_TAG" = "$LATEST_URL" ]; then
    echo "[update.sh] could not resolve the latest release tag from $LATEST_URL" >&2
    exit 1
fi
echo "[update.sh] bootstrapping coterie-provision from $PROVISION_TAG"

ASSET_NAME="coterie-provision-${PROVISION_TAG}-${TARGET_TRIPLE}.tar.gz"
ASSET_URL="https://github.com/${REPO}/releases/download/${PROVISION_TAG}/${ASSET_NAME}"
SHA_URL="${ASSET_URL}.sha256"

cd "$WORK_DIR"
echo "[update.sh] downloading $ASSET_NAME..."
curl -sfL -o "$ASSET_NAME" "$ASSET_URL"
curl -sfL -o "${ASSET_NAME}.sha256" "$SHA_URL"
echo "[update.sh] verifying checksum..."
sha256sum -c "${ASSET_NAME}.sha256"

echo "[update.sh] extracting..."
tar -xzf "$ASSET_NAME"
test -x ./coterie-provision || {
    echo "[update.sh] extracted tarball did not contain a coterie-provision binary" >&2
    exit 1
}

echo "[update.sh] handing off to coterie-provision update..."
exec ./coterie-provision update "$@"
