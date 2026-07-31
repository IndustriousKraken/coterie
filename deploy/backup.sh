#!/usr/bin/env bash
# Coterie backup — one bundle per run: vacuumed DB + both upload roots.
#
# What it does:
#   1. VACUUM INTO a staging file (consistent live snapshot, no WAL)
#   2. tar+gzip that snapshot together with uploads/ and private-uploads/
#      into a single timestamped bundle
#   3. Promote the bundle to weekly/monthly slots on Sundays / day 1
#   4. Sweep retention (default 7 daily + 4 weekly + 12 monthly)
#   5. Optionally push the new bundle to an S3-compatible bucket
#
# Why the uploads are in there:
#   The database references files it does not contain — submission rows
#   name attachment paths, event and announcement rows name image paths.
#   A database-only restore brings back rows pointing at files that do
#   not exist, which shows up as broken images and missing attachments
#   rather than as an error. One bundle restores to a working instance,
#   which is also what makes it usable as a migration artifact.
#
#   Any NEW upload root added later must be added here in the same
#   change. Omitting one fails silently, at restore time.
#
# Why VACUUM INTO and not `cp` of the .db file:
#   In WAL mode the live file is incomplete without its `-wal` and
#   `-shm` siblings. `VACUUM INTO` produces a single self-contained
#   file in one atomic SQLite operation. Restoring is then extraction
#   and a move — no replay, no WAL surgery.
#
# Growth ceiling (deliberate, known):
#   The uploads are re-archived IN FULL on every run. That is fine at a
#   few megabytes and wasteful once an org accumulates hundreds of
#   megabytes of event images across 7 daily + 4 weekly + 12 monthly
#   slots. When that day comes the upgrade path is either a `--db-only`
#   mode for the frequent runs plus a less frequent full bundle, or
#   hardlink deduplication of unchanged upload files between slots.
#   Not built now: correct and simple beats clever and speculative.
#
# Where to put the backups:
#   COTERIE_BACKUP_DIR defaults to {data_dir}/backups — the SAME DISK as
#   the data it protects, so one disk failure takes both. Point it at a
#   separate volume where the host has one (and/or set
#   COTERIE_BACKUP_S3_URI for offsite).
#
# Schedule with the systemd timer (deploy/coterie-backup.timer) or any
# crond. The script is idempotent — running it twice in the same day
# just overwrites that day's bundle. Running it by hand is supported and
# produces exactly the same artifact as the scheduled run (that is the
# point: the migration path and the recovery path are one code path).
#
# Required env (with sensible defaults):
#   COTERIE__SERVER__DATA_DIR  the instance's data dir — DB and upload
#                              roots are derived from it
#                              default: /var/lib/coterie
#   COTERIE_BACKUP_DIR         where bundles go (created if missing)
#                              default: {data_dir}/backups
#
# Optional env (paths, for non-default layouts):
#   COTERIE_DB                     override the DB path
#   COTERIE__SERVER__UPLOADS_DIR   override the public uploads root
#   SQLITE3                        sqlite3 binary (default: sqlite3)
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

DATA_DIR="${COTERIE__SERVER__DATA_DIR:-/var/lib/coterie}"
DB="${COTERIE_DB:-$DATA_DIR/coterie.db}"
BACKUP_DIR="${COTERIE_BACKUP_DIR:-$DATA_DIR/backups}"
# Both roots are derived from the data dir (config's `uploads_path()` /
# `private_uploads_path()`), so an instance on a non-default
# COTERIE__SERVER__DATA_DIR still gets its files backed up. The public
# root has a config override; the private one does not.
UPLOADS_DIR="${COTERIE__SERVER__UPLOADS_DIR:-$DATA_DIR/uploads}"
PRIVATE_UPLOADS_DIR="$DATA_DIR/private-uploads"
SQLITE3="${SQLITE3:-sqlite3}"
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

# Staging lives under the backup dir (same filesystem as the bundle, so
# the final `mv` is atomic) but OUTSIDE daily/weekly/monthly, so the
# retention sweep never sees it.
STAGE=$(mktemp -d "$BACKUP_DIR/.stage-XXXXXX")
trap 'rm -rf "$STAGE"' EXIT

# --- snapshot ---------------------------------------------------------
BUNDLE="$DAILY_DIR/coterie-$DATE.tar.gz"
log "Backing up $DB + uploads -> $BUNDLE"

"$SQLITE3" "$DB" "VACUUM INTO '$STAGE/coterie.db'"

# The upload roots go into the bundle under fixed member names
# (`uploads/`, `private-uploads/`) regardless of where they live on
# disk, so restore knows what it is looking at. Symlink + tar
# --dereference is what makes an out-of-tree uploads root archive under
# the right name. A missing root becomes an empty directory in the
# bundle — a fresh instance has neither, and that is normal, not an
# error.
for pair in "uploads:$UPLOADS_DIR" "private-uploads:$PRIVATE_UPLOADS_DIR"; do
    name="${pair%%:*}"
    src="${pair#*:}"
    if [[ -d "$src" ]]; then
        ln -s "$src" "$STAGE/$name"
    else
        log "No $name root at $src — bundling an empty one"
        mkdir -p "$STAGE/$name"
    fi
done

# NEVER archive the backup directory into a bundle. Its default sits
# INSIDE the data dir, so any future "just tar the whole data dir"
# shortcut would nest every backup inside the next one and grow without
# bound. We name the two upload roots explicitly, which already excludes
# it; this exclude is the belt to that pair of braces, so the invariant
# survives someone widening the enumeration later.
TAR_ARGS=(--exclude=".stage-*")
if [[ "$BACKUP_DIR" == "$DATA_DIR"/* ]]; then
    TAR_ARGS+=("--exclude=${BACKUP_DIR#"$DATA_DIR"/}")
fi

# -h follows the upload-root symlinks staged above. Uploads are
# server-generated files, so there are no symlinks inside them to
# accidentally flatten.
tar -czhf "$BUNDLE.tmp" "${TAR_ARGS[@]}" \
    -C "$STAGE" coterie.db uploads private-uploads
mv "$BUNDLE.tmp" "$BUNDLE"
log "Bundle ready: $BUNDLE ($(stat -c%s "$BUNDLE" 2>/dev/null || stat -f%z "$BUNDLE") bytes)"

# --- promote to weekly / monthly slots --------------------------------
# Sunday → weekly. Use ISO week so the filename sorts correctly.
if [[ "$DOW" == "7" ]]; then
    WEEKLY_BUNDLE="$WEEKLY_DIR/coterie-$WEEK.tar.gz"
    cp -f "$BUNDLE" "$WEEKLY_BUNDLE"
    log "Promoted to weekly: $WEEKLY_BUNDLE"
fi

# 1st of month → monthly.
if [[ "$DOM" == "01" ]]; then
    MONTHLY_BUNDLE="$MONTHLY_DIR/coterie-$MONTH.tar.gz"
    cp -f "$BUNDLE" "$MONTHLY_BUNDLE"
    log "Promoted to monthly: $MONTHLY_BUNDLE"
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
    DEST="${S3_URI%/}/daily/$(basename "$BUNDLE")"
    log "Uploading to $DEST"
    aws s3 cp "$BUNDLE" "$DEST" --only-show-errors
    log "Upload complete"
fi

log "Done."
