#!/bin/sh
# release-deploy.sh — fetch a tagged Coterie release and apply it.
#
# As of openspec change a38 the in-place UPDATE logic — pre-update DB
# snapshot, post-restart /health check, and automatic rollback — lives
# in the testable `coterie-provision update` binary, NOT in this bash.
# This script is now a thin shim that preserves the historical
# positional-tag interface:
#
#   release-deploy.sh            # latest stable release
#   release-deploy.sh v1.2.3     # pin, or roll back, to a specific tag
#
# Behaviour:
#   * On an EXISTING install (a coterie binary is already present) it
#     delegates to the single hardened update path
#     (`coterie-provision update`). The positional tag becomes `--tag`.
#     That binary ships in the release tarball and is placed by the
#     first-install path below and by every update, so it is normally
#     right there. If it isn't — an install older than the release that
#     started shipping it — update.sh bootstraps a download of it, and
#     the fallback is announced, because it means the hardened path was
#     unavailable rather than unused.
#   * On a FIRST install (no binary yet) it performs the minimal
#     download + bootstrap, because there is no running service to
#     health-check nor database to snapshot. That path is what
#     `coterie-provision install` drives.
#
# Assumes: /opt/coterie exists; `curl`, `tar`, `sha256sum` installed
# (first install additionally needs `python3`, in Debian's base).

set -eu

REPO="IndustriousKraken/coterie"
# Overridable so the delegation logic below is testable without a real
# /opt/coterie. Operators have no reason to set it.
INSTALL_DIR="${COTERIE_INSTALL_DIR:-/opt/coterie}"
TAG="${1:-}"

# ---------------------------------------------------------------------
# Update path: an install already exists → hand off to the hardened,
# testable update flow so there is a single update code path.
# ---------------------------------------------------------------------
if [ -f "$INSTALL_DIR/coterie" ]; then
    if [ -n "$TAG" ]; then
        set -- --tag "$TAG"
    else
        set --
    fi
    if [ -x "$INSTALL_DIR/coterie-provision" ]; then
        echo "release-deploy.sh: delegating to coterie-provision update"
        exec "$INSTALL_DIR/coterie-provision" update "$@"
    elif command -v coterie-provision >/dev/null 2>&1; then
        echo "release-deploy.sh: delegating to coterie-provision update"
        exec coterie-provision update "$@"
    fi
    # No provision binary on the box. It ships in the release tarball and
    # is placed by the first-install path below and by every
    # `coterie-provision update`, so this means an install older than the
    # release that started shipping it. The bootstrap downloads the binary
    # and runs the same hardened update; it just costs a download, and the
    # copy it places is picked up next time. Say so — falling back
    # silently is how this went unnoticed.
    echo "release-deploy.sh: no executable coterie-provision at $INSTALL_DIR or on PATH" >&2
    SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd || echo .)"
    for cand in "$SCRIPT_DIR/update.sh" "$INSTALL_DIR/deploy/update.sh"; do
        [ -f "$cand" ] || continue
        # Older installs were placed without the exec bit; restore it so
        # the exec below can honour the shebang.
        [ -x "$cand" ] || chmod +x "$cand" 2>/dev/null || true
        if [ -x "$cand" ]; then
            echo "release-deploy.sh: falling back to the bootstrap updater $cand" >&2
            # exec'd directly, NOT via `sh`: update.sh is bash, /bin/sh is
            # dash on Debian, and dash rejects its `trap ... ERR` with
            # "bad trap". The shebang names the interpreter; honour it.
            exec "$cand" "$@"
        fi
        echo "release-deploy.sh: $cand is not executable and chmod failed" >&2
    done
    echo "ERROR: could not locate coterie-provision or update.sh to perform the update" >&2
    exit 1
fi

# ---------------------------------------------------------------------
# First-install bootstrap (no existing binary). There is no service to
# stop/restart and no database to snapshot, so this path stays inline.
# ---------------------------------------------------------------------
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

# Resolve the requested tag (latest if no arg).
if [ -n "$TAG" ]; then
    API_URL="https://api.github.com/repos/$REPO/releases/tags/$TAG"
else
    API_URL="https://api.github.com/repos/$REPO/releases/latest"
fi

echo "Querying GitHub for release: ${TAG:-latest}"
# Fetch and parse with Python rather than jq. GitHub's API occasionally
# returns release bodies with unescaped control bytes which strict JSON
# parsers reject; Python's json with strict=False tolerates them, and
# python3 is in Debian's base install.
RELEASE_JSON_FILE="$TMP_DIR/release.json"
curl -sfL "$API_URL" > "$RELEASE_JSON_FILE" || {
    echo "ERROR: couldn't fetch release info from $API_URL"
    exit 1
}

TAG="$(python3 -c "
import json
with open('$RELEASE_JSON_FILE') as f:
    data = json.load(f, strict=False)
print(data['tag_name'])
")"
echo "Resolved release: $TAG"

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
# The checksum asset is always the tarball's URL + .sha256. Deriving it
# (rather than searching assets for the first *.sha256) avoids matching
# the coterie-provision tarball's checksum, which GitHub can list first.
CHECKSUM_URL="${TARBALL_URL}.sha256"

if [ -z "$TARBALL_URL" ]; then
    echo "ERROR: couldn't find release tarball in $TAG"
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

echo "First install detected (no existing binary at $INSTALL_DIR/coterie)."

# Place the binaries. Anything in $INSTALL_DIR that's NOT in the release
# stays (specifically: .env stays; /var/lib/coterie data is untouched).
mkdir -p "$INSTALL_DIR"
# create_admin is required: `coterie-provision install` runs it to bootstrap
# the first admin, and asserts it is present. Ships in the tarball alongside
# coterie/seed, so place it the same way.
install -m 0755 "$STAGE_DIR/coterie"      "$INSTALL_DIR/coterie.new"
install -m 0755 "$STAGE_DIR/seed"         "$INSTALL_DIR/seed.new"
install -m 0755 "$STAGE_DIR/create_admin" "$INSTALL_DIR/create_admin.new"
mv "$INSTALL_DIR/coterie.new"      "$INSTALL_DIR/coterie"
mv "$INSTALL_DIR/seed.new"         "$INSTALL_DIR/seed"
mv "$INSTALL_DIR/create_admin.new" "$INSTALL_DIR/create_admin"
# coterie-provision is the hardened update path every later run of this
# script delegates to; without it on disk that delegation is unreachable
# and updates fall through to the bootstrap. Guarded because tarballs
# from before it shipped in the main archive don't carry it.
if [ -f "$STAGE_DIR/coterie-provision" ]; then
    install -m 0755 "$STAGE_DIR/coterie-provision" "$INSTALL_DIR/coterie-provision.new"
    mv "$INSTALL_DIR/coterie-provision.new" "$INSTALL_DIR/coterie-provision"
fi

# Static, migrations: replace wholesale.
rm -rf "$INSTALL_DIR/static" "$INSTALL_DIR/migrations"
cp -r "$STAGE_DIR/static"      "$INSTALL_DIR/"
cp -r "$STAGE_DIR/migrations"  "$INSTALL_DIR/"

# Keep .env.example current so operators can diff against their live
# .env. NEVER touches .env itself.
if [ -f "$STAGE_DIR/.env.example" ]; then
    cp -f "$STAGE_DIR/.env.example" "$INSTALL_DIR/.env.example"
fi
# deploy/ scripts kept up to date too (copied individually so operators
# can pin local changes if needed).
mkdir -p "$INSTALL_DIR/deploy"
cp -f "$STAGE_DIR/deploy/"*.sh        "$INSTALL_DIR/deploy/" 2>/dev/null || true
cp -f "$STAGE_DIR/deploy/"*.service   "$INSTALL_DIR/deploy/" 2>/dev/null || true
cp -f "$STAGE_DIR/deploy/"*.openrc    "$INSTALL_DIR/deploy/" 2>/dev/null || true
cp -f "$STAGE_DIR/deploy/"*.timer     "$INSTALL_DIR/deploy/" 2>/dev/null || true
cp -f "$STAGE_DIR/deploy/Caddyfile.example" "$INSTALL_DIR/deploy/" 2>/dev/null || true
# cp preserves the source mode; these are run directly (update.sh is
# exec'd by this script, backup.sh by the systemd unit) so the exec bit
# has to be there regardless of what the tarball carried.
chmod 0755 "$INSTALL_DIR/deploy/"*.sh 2>/dev/null || true

# Record the version.
cp "$STAGE_DIR/VERSION" "$INSTALL_DIR/VERSION"

# Run install.sh (creates coterie user, /var/lib/coterie, systemd unit).
# install.sh chowns $INSTALL_DIR itself but doesn't recurse, so we chown
# the contents afterwards.
if [ ! -f "$INSTALL_DIR/deploy/install.sh" ]; then
    echo "ERROR: $INSTALL_DIR/deploy/install.sh not found"
    echo "Cannot complete first-install bootstrap."
    exit 1
fi
echo "Running install.sh to set up user, dirs, and systemd unit..."
bash "$INSTALL_DIR/deploy/install.sh"

if id coterie >/dev/null 2>&1; then
    chown -R coterie:coterie "$INSTALL_DIR"
fi

cat <<EOF

============================================================
First-install bootstrap complete. Next steps:

  1. Create /opt/coterie/.env from .env.example and fill in
     required settings (database URL, session secret, Stripe
     keys, etc.). See docs/deploy/SETUP.md for the field reference.

       cp /opt/coterie/.env.example /opt/coterie/.env
       chown coterie:coterie /opt/coterie/.env
       chmod 0640 /opt/coterie/.env

  2. Configure Caddy (see deploy/Caddyfile.example) and DNS.

  3. Start the service:
       systemctl start coterie
       systemctl enable coterie    # start on boot
       systemctl status coterie

  4. Visit https://your-domain/setup to create the first admin.

For subsequent updates, re-run this script (or update.sh) — it
delegates to the hardened 'coterie-provision update' path.
============================================================
EOF

echo "Installed Coterie $TAG"
