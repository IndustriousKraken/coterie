#!/usr/bin/env bash
# Coterie restore — put a backup bundle produced by deploy/backup.sh
# back in place: database AND both upload roots.
#
# Usage:
#   sudo bash deploy/restore.sh /var/lib/coterie/backups/daily/coterie-2026-07-31.tar.gz
#   sudo bash deploy/restore.sh --data-dir /srv/coterie <bundle>
#
# Order is deliberate, because a restore is usually run under time
# pressure by someone who has not done one before:
#
#   1. verify the bundle BEFORE anything is touched — a truncated or
#      unreadable bundle must fail with the instance still running
#   2. stop the service
#   3. move the existing database and BOTH upload roots aside; never
#      delete them, so a restore of the wrong snapshot is itself
#      reversible
#   4. extract the bundle into place
#   5. chown to the service account
#   6. PRAGMA integrity_check
#   7. start the service — only if the check passed
#
# The displaced data lands in {data_dir}/pre-restore-<timestamp>/ and
# the path is printed at the end. Delete it once the restore is
# confirmed good; nothing else will.
#
# Env (all optional):
#   COTERIE__SERVER__DATA_DIR      data dir (default /var/lib/coterie);
#                                  --data-dir overrides it
#   COTERIE_DB                     override the DB path
#   COTERIE__SERVER__UPLOADS_DIR   override the public uploads root
#   COTERIE_SERVICE                systemd unit name (default coterie)
#   COTERIE_SERVICE_USER           owner of the data (default coterie)
#   COTERIE_SYSTEMCTL              systemctl binary (default systemctl)
#   SQLITE3                        sqlite3 binary (default sqlite3)
#   COTERIE_RESTORE_ALLOW_NONROOT  set to 1 to restore into a data dir
#                                  you already own (test harnesses);
#                                  service management and chown are then
#                                  best-effort. Not for production use.

set -euo pipefail

BUNDLE=""
DATA_DIR_OVERRIDE=""

usage() {
    sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --data-dir)
            [[ $# -ge 2 ]] || { echo "ERROR: --data-dir needs a value" >&2; exit 2; }
            DATA_DIR_OVERRIDE="$2"
            shift 2
            ;;
        -*)
            echo "ERROR: unknown option $1" >&2
            exit 2
            ;;
        *)
            [[ -z "$BUNDLE" ]] || { echo "ERROR: only one bundle may be given" >&2; exit 2; }
            BUNDLE="$1"
            shift
            ;;
    esac
done

DATA_DIR="${DATA_DIR_OVERRIDE:-${COTERIE__SERVER__DATA_DIR:-/var/lib/coterie}}"
DB="${COTERIE_DB:-$DATA_DIR/coterie.db}"
UPLOADS_DIR="${COTERIE__SERVER__UPLOADS_DIR:-$DATA_DIR/uploads}"
PRIVATE_UPLOADS_DIR="$DATA_DIR/private-uploads"
SERVICE="${COTERIE_SERVICE:-coterie}"
SERVICE_USER="${COTERIE_SERVICE_USER:-coterie}"
SYSTEMCTL="${COTERIE_SYSTEMCTL:-systemctl}"
SQLITE3="${SQLITE3:-sqlite3}"

log() {
    echo "[coterie-restore $(date -Iseconds)] $*"
}

die() {
    echo "[coterie-restore] ERROR: $*" >&2
    exit 1
}

# --- 0. who are we ----------------------------------------------------
# Refuse up front rather than failing halfway through with a confusing
# permissions error, with the service already stopped.
if [[ "$(id -u)" -ne 0 ]]; then
    if [[ "${COTERIE_RESTORE_ALLOW_NONROOT:-0}" != "1" ]]; then
        die "must run as root (it stops $SERVICE, writes $DATA_DIR, and chowns to $SERVICE_USER).
       Re-run with: sudo bash $0 ${BUNDLE:-BUNDLE_PATH}
       To restore into a data dir you already own, set COTERIE_RESTORE_ALLOW_NONROOT=1."
    fi
    log "WARNING: running as $(id -un), not root — chown will be skipped"
fi

[[ -n "$BUNDLE" ]] || die "no bundle given. Try: $0 --help"

# --- 1. verify (before anything is touched) ---------------------------
[[ -f "$BUNDLE" && -r "$BUNDLE" ]] || die "bundle not readable: $BUNDLE"

log "Verifying $BUNDLE"
# Listing the archive walks the whole gzip stream, so a truncated or
# corrupt bundle fails here — with the service still running and the
# existing data untouched.
MEMBERS=$(tar -tzf "$BUNDLE") || die "bundle is unreadable, truncated, or not a coterie bundle: $BUNDLE"

grep -qx 'coterie.db' <<<"$MEMBERS" \
    || die "bundle has no coterie.db member — is this a coterie backup bundle?"
for root in uploads private-uploads; do
    grep -qE "^$root/?$" <<<"$MEMBERS" \
        || die "bundle has no $root/ member. A database-only artifact is not a complete
       backup: its rows reference attachment and image files it does not carry."
done

# Every member must land under the staging directory. This extracts as
# root, and the bundle is input from a trust boundary — it may have come
# back from offsite storage, or off a host someone else has had. GNU and
# BusyBox tar both refuse `..` members themselves, but they refuse them
# *during extraction*, which is after the service is stopped and the
# existing data displaced. Checking here fails while the instance is
# still up, and does not depend on which tar the host has.
while IFS= read -r member; do
    case "$member" in
        /*|..|../*|*/..|*/../*)
            die "bundle contains a member that would write outside the restore
       directory: $member
       This is not an artifact deploy/backup.sh produces. Treat the bundle
       as tampered with and restore from another copy."
            ;;
    esac
done <<<"$MEMBERS"

log "Bundle looks complete (database + uploads + private-uploads)"

# --- 2. stage, then stop the service ----------------------------------
# Staging lives beside the data so the moves into place are renames on
# one filesystem rather than copies across two.
mkdir -p "$DATA_DIR"
STAGE=$(mktemp -d "$DATA_DIR/.restore-XXXXXX")
trap 'rm -rf "$STAGE"' EXIT

if command -v "$SYSTEMCTL" >/dev/null 2>&1; then
    log "Stopping $SERVICE"
    if ! "$SYSTEMCTL" stop "$SERVICE" 2>/dev/null; then
        # A unit that does not exist yet is the fresh-host case and is
        # fine. A unit that exists and is still running is not: swapping
        # the SQLite file under a live process corrupts WAL state.
        if "$SYSTEMCTL" is-active --quiet "$SERVICE" 2>/dev/null; then
            die "could not stop $SERVICE and it is still active — refusing to restore under a live instance"
        fi
        log "$SERVICE not running (or not installed) — continuing"
    fi
else
    log "No $SYSTEMCTL on this host — skipping service stop"
fi

# --- 3. move the existing state aside (never delete) ------------------
DISPLACED="$DATA_DIR/pre-restore-$(date +%Y%m%dT%H%M%S)"
DISPLACED_ANY=0
# Created lazily: restoring onto a clean host displaces nothing, and an
# empty pre-restore-* directory would just be litter to explain later.
displace() {
    local path="$1"
    [[ -e "$path" ]] || return 0
    mkdir -p "$DISPLACED"
    DISPLACED_ANY=1
    mv "$path" "$DISPLACED/$(basename "$path")"
    log "Moved $path -> $DISPLACED/$(basename "$path")"
}
displace "$DB"
displace "$DB-wal"
displace "$DB-shm"
displace "$UPLOADS_DIR"
displace "$PRIVATE_UPLOADS_DIR"

# --- 4. extract into place --------------------------------------------
log "Extracting bundle"
tar -xzf "$BUNDLE" -C "$STAGE"
mkdir -p "$(dirname "$DB")" "$(dirname "$UPLOADS_DIR")" "$(dirname "$PRIVATE_UPLOADS_DIR")"
mv "$STAGE/coterie.db" "$DB"
mv "$STAGE/uploads" "$UPLOADS_DIR"
mv "$STAGE/private-uploads" "$PRIVATE_UPLOADS_DIR"

# --- 5. ownership ------------------------------------------------------
if [[ "$(id -u)" -eq 0 ]]; then
    log "chown -R $SERVICE_USER:$SERVICE_USER on the restored data"
    chown -R "$SERVICE_USER:$SERVICE_USER" "$DB" "$UPLOADS_DIR" "$PRIVATE_UPLOADS_DIR"
    chmod 0640 "$DB"
else
    log "Skipping chown (not root)"
fi

# --- 6. integrity check ------------------------------------------------
log "Running PRAGMA integrity_check"
# No pipeline here on purpose: a corrupt database prints many lines, and
# `| head -1` under `set -o pipefail` would kill the script on SIGPIPE
# right before it could print the message that matters.
CHECK_OUT=$("$SQLITE3" "$DB" 'PRAGMA integrity_check;' 2>&1 || echo "integrity_check could not run")
CHECK="${CHECK_OUT%%$'\n'*}"
CHECK="${CHECK//[[:space:]]/}"
if [[ "$CHECK" != "ok" ]]; then
    # Only point at the displaced data when there is some: on a
    # fresh-host restore nothing was displaced and pre-restore-* was
    # never created, so naming it would send an operator who is already
    # having a bad day looking for a directory that does not exist.
    if [[ "$DISPLACED_ANY" -eq 1 ]]; then
        RECOVERY="The data this restore displaced is
       intact at $DISPLACED — move it back to undo, or try an older bundle."
    else
        RECOVERY="Nothing was displaced (the data dir was empty before
       this restore), so there is nothing to move back — re-copy the bundle
       in case the transfer was bad, or try an older one."
    fi
    die "integrity_check on the restored database said: $CHECK
       $SERVICE has NOT been started. $RECOVERY"
fi
log "integrity_check: ok"

# --- 7. start the service ----------------------------------------------
if command -v "$SYSTEMCTL" >/dev/null 2>&1; then
    log "Starting $SERVICE"
    "$SYSTEMCTL" start "$SERVICE" || die "restored data is in place but $SERVICE failed to start — check: journalctl -u $SERVICE -n 50"
fi

cat <<EOF

Restore complete.

  Bundle:    $BUNDLE
  Database:  $DB
  Uploads:   $UPLOADS_DIR
             $PRIVATE_UPLOADS_DIR
EOF

if [[ "$DISPLACED_ANY" -eq 1 ]]; then
    cat <<EOF

  The data that was there before is NOT deleted. It is in:

      $DISPLACED

  To reverse this restore, stop $SERVICE and move those entries back.
  Once you have confirmed the restore is good, delete that directory —
  nothing else will.

EOF
else
    echo "
  Nothing was displaced — the data dir was empty before this restore.
"
fi
