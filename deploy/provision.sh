#!/usr/bin/env bash
# Coterie provisioning bootstrap.
#
# Downloads the latest `coterie-provision` binary from the GitHub
# Release matching the requested tag (defaulting to the latest stable
# release), verifies its SHA256, and execs into it so all wizard logic
# lives in the Rust binary.
#
# Curl-and-bash invocation:
#   curl -sfL https://raw.githubusercontent.com/IndustriousKraken/coterie/master/deploy/provision.sh \
#       -o /tmp/provision.sh
#   sudo bash /tmp/provision.sh
#
# Pass --tag <vX.Y.Z> to pin a specific release; any other flags are
# forwarded verbatim to `coterie-provision install`.

set -euo pipefail

REPO="IndustriousKraken/coterie"
TARGET_TRIPLE="x86_64-unknown-linux-musl"
TAG=""
PASSTHRU=()

while [ $# -gt 0 ]; do
    case "$1" in
        --tag)
            TAG="${2:-}"; shift 2 ;;
        --tag=*)
            TAG="${1#--tag=}"; shift ;;
        *)
            PASSTHRU+=("$1"); shift ;;
    esac
done

trap 'echo "[provision.sh] ERROR on line $LINENO — exit $?" >&2' ERR

if [ "$(id -u)" -ne 0 ]; then
    echo "[provision.sh] must run as root (try sudo)" >&2
    exit 1
fi

if [ ! -r /etc/os-release ]; then
    echo "[provision.sh] /etc/os-release missing — cannot detect OS" >&2
    exit 1
fi
. /etc/os-release
case "${ID:-}:${ID_LIKE:-}" in
    debian:*|*:*debian*) : ;;
    *)
        echo "[provision.sh] this wizard only supports Debian-family distros (got ID=${ID:-?})" >&2
        exit 1
        ;;
esac

for bin in curl tar sha256sum python3; do
    command -v "$bin" >/dev/null 2>&1 || {
        echo "[provision.sh] missing required tool: $bin" >&2
        exit 1
    }
done

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

# Resolve the tag if not provided.
if [ -z "$TAG" ]; then
    echo "[provision.sh] resolving latest stable release..."
    LATEST_JSON="$WORK_DIR/latest.json"
    curl -sfL -H "User-Agent: coterie-provision-bootstrap" \
        "https://api.github.com/repos/${REPO}/releases/latest" >"$LATEST_JSON"
    TAG="$(python3 -c "
import json, sys
with open('$LATEST_JSON') as f:
    d = json.load(f, strict=False)
print(d['tag_name'])
")"
fi
echo "[provision.sh] using release: $TAG"

ASSET_NAME="coterie-provision-${TAG}-${TARGET_TRIPLE}.tar.gz"
ASSET_URL="https://github.com/${REPO}/releases/download/${TAG}/${ASSET_NAME}"
SHA_URL="${ASSET_URL}.sha256"

cd "$WORK_DIR"
echo "[provision.sh] downloading $ASSET_NAME..."
curl -sfL -o "$ASSET_NAME" "$ASSET_URL"
curl -sfL -o "${ASSET_NAME}.sha256" "$SHA_URL"
echo "[provision.sh] verifying checksum..."
sha256sum -c "${ASSET_NAME}.sha256"

echo "[provision.sh] extracting..."
tar -xzf "$ASSET_NAME"
test -x ./coterie-provision || {
    echo "[provision.sh] extracted tarball did not contain a coterie-provision binary" >&2
    exit 1
}

echo "[provision.sh] handing off to coterie-provision install..."
exec ./coterie-provision install "${PASSTHRU[@]}"
