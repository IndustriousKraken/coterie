#!/usr/bin/env bash
# Coterie backup — SQLite VACUUM INTO + retention sweep + optional S3 push.
#
# What it does:
#   1. VACUUM INTO a timestamped file (consistent live snapshot, no WAL)
#   2. gzip it
#   3. Promote the file to weekly/monthly slots on Sundays / day 1
#   4. Sweep retention (default 7 daily + 4 weekly + 12 monthly)
#   5. Optionally push the new file to an S3-compatible bucket
#
# Why VACUUM INTO and not `cp` of the .db file:
#   In WAL mode the live file is incomplete without its `-wal` and
#   `-shm` siblings. `VACUUM INTO` produces a single self-contained
#   file in one atomic SQLite operation. Restoring is then a simple
#   file copy back into place — no replay, no WAL surgery.
#
# Schedule with the systemd timer (deploy/coterie-backup.timer) or any
# crond. The script is idempotent — running it twice in the same day
# just overwrites that day's snapshot.
#
# Required env (with sensible defaults):
#   COTERIE_DB              path to the live coterie.db
#                           default: /var/lib/coterie/data/coterie.db
#   COTERIE_BACKUP_DIR      where backups go (created if missing)
#                           default: /var/lib/coterie/backups
#
# Optional env (offsite push):
#   COTERIE_BACKUP_S3_URI   s3://bucket/path-prefix/  to enable upload
#   AWS_ENDPOINT_URL_S3     for non-AWS S3-compat (Backblaze, Wasabi,
#                           DO Spaces, Cloudflare R2)
#   plus the usual AWS credential env vars or AWS_PROFILE
#
# Optional env (retention):
#   COTERIE_KEEP_DAILY      default 7
#   COTERIE_KEEP_WEEKLY     default 4
#   COTERIE_KEEP_MONTHLY    default 12

set -euo pipefail

DB="${COTERIE_DB:-/var/lib/coterie/data/coterie.db}"
BACKUP_DIR="${COTERIE_BACKUP_DIR:-/var/lib/coterie/backups}"
S3_URI="${COTERIE_BACKUP_S3_URI:-}"

KEEP_DAILY="${COTERIE_KEEP_DAILY:-7}"
KEEP_WEEKLY="${COTERIE_KEEP_WEEKLY:-4}"
KEEP_MONTHLY="${COTERIE_KEEP_MONTHLY:-12}"

DATE=$(date +%F)              # 2026-04-27
DOW=$(date +%u)               # 1..7, Mon=1, Sun=7
DOM=$(date +%d)               # 01..31
WEEK=$(date +%G-W%V)          # ISO week, e.g. 2026-W17
MONTH=$(date +%Y-%m)          # 2026-04

DAILY_DIR="$BACKUP_DIR/daily"
WEEKLY_DIR="$BACKUP_DIR/weekly"
MONTHLY_DIR="$BACKUP_DIR/monthly"

log() {
    echo "[coterie-backup $(date -Iseconds)] $*"
}

# --- preflight --------------------------------------------------------
if [[ ! -f "$DB" ]]; then
    log "ERROR: database not found at $DB"
    exit 1
fi

mkdir -p "$DAILY_DIR" "$WEEKLY_DIR" "$MONTHLY_DIR"

# --- snapshot ---------------------------------------------------------
DAILY_FILE="$DAILY_DIR/coterie-$DATE.db"
log "Backing up $DB -> $DAILY_FILE"

# VACUUM INTO writes a fresh file. Failing midway leaves no partial
# .db behind because VACUUM INTO only renames after success.
sqlite3 "$DB" "VACUUM INTO '$DAILY_FILE.tmp'"
mv "$DAILY_FILE.tmp" "$DAILY_FILE"
gzip -f "$DAILY_FILE"
DAILY_GZ="$DAILY_FILE.gz"
log "Snapshot ready: $DAILY_GZ ($(stat -c%s "$DAILY_GZ" 2>/dev/null || stat -f%z "$DAILY_GZ") bytes)"

# --- promote to weekly / monthly slots --------------------------------
# Sunday → weekly. Use ISO week so the filename sorts correctly.
if [[ "$DOW" == "7" ]]; then
    WEEKLY_GZ="$WEEKLY_DIR/coterie-$WEEK.db.gz"
    cp -f "$DAILY_GZ" "$WEEKLY_GZ"
    log "Promoted to weekly: $WEEKLY_GZ"
fi

# 1st of month → monthly.
if [[ "$DOM" == "01" ]]; then
    MONTHLY_GZ="$MONTHLY_DIR/coterie-$MONTH.db.gz"
    cp -f "$DAILY_GZ" "$MONTHLY_GZ"
    log "Promoted to monthly: $MONTHLY_GZ"
fi

# --- retention sweep --------------------------------------------------
sweep() {
    local dir="$1"
    local keep="$2"
    # ls -1t sorts newest first; tail strips the keep-newest from the head
    # and feeds the rest to xargs rm. No-op when there are <= keep files.
    if [[ -d "$dir" ]]; then
        ls -1t "$dir" 2>/dev/null \
            | tail -n +"$((keep + 1))" \
            | while read -r f; do
                log "Pruning $dir/$f"
                rm -f -- "$dir/$f"
            done
    fi
}

sweep "$DAILY_DIR"   "$KEEP_DAILY"
sweep "$WEEKLY_DIR"  "$KEEP_WEEKLY"
sweep "$MONTHLY_DIR" "$KEEP_MONTHLY"

# --- offsite push -----------------------------------------------------
if [[ -n "$S3_URI" ]]; then
    if ! command -v aws >/dev/null 2>&1; then
        log "ERROR: COTERIE_BACKUP_S3_URI set but 'aws' CLI not found"
        exit 1
    fi
    # Strip trailing slash on S3_URI so we don't end up with a //
    DEST="${S3_URI%/}/daily/$(basename "$DAILY_GZ")"
    log "Uploading to $DEST"
    aws s3 cp "$DAILY_GZ" "$DEST" --only-show-errors
    log "Upload complete"
fi

log "Done."
