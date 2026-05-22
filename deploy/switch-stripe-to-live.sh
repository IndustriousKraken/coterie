#!/usr/bin/env bash
# switch-stripe-to-live.sh — one-shot transition from Stripe TEST mode
# to LIVE mode on a Coterie box provisioned by deploy/provision.sh in
# test mode.
#
# What it does:
#   1. Refuses if already in live mode (idempotent) or if no test DB exists.
#   2. Loads live Stripe creds from /opt/coterie/.env.live, or prompts.
#   3. Validates credential shapes; optionally calls /v1/balance to
#      confirm the secret key is accepted by Stripe before any change.
#   4. Stops coterie.
#   5. Creates a fresh /var/lib/coterie/coterie.db with schema + migration
#      tracking copied from coterie-test.db, then copies admin row(s) via
#      ATTACH DATABASE.
#   6. Archives (default) or discards the test DB.
#   7. Atomically rewrites /opt/coterie/.env with live creds + live DB URL.
#   8. Cleans up /opt/coterie/.env.live if present.
#   9. Starts coterie and smoke-tests http://127.0.0.1:8080/health.
#  10. Prints a reminder to register the live-mode webhook in Stripe.
#
# Usage:
#   sudo bash switch-stripe-to-live.sh [--discard-test-db] [--yes] [--help]

set -euo pipefail

# ---------------------------------------------------------------------
# Globals
# ---------------------------------------------------------------------

INSTALL_DIR="/opt/coterie"
DATA_DIR="/var/lib/coterie"
ENV_FILE="$INSTALL_DIR/.env"
ENV_LIVE_FILE="$INSTALL_DIR/.env.live"
TEST_DB="$DATA_DIR/coterie-test.db"
LIVE_DB="$DATA_DIR/coterie.db"
SERVICE="coterie"
HEALTH_URL="http://127.0.0.1:8080/health"

DISCARD_TEST_DB=false
ASSUME_YES=false

LIVE_PK=""
LIVE_SK=""
LIVE_WHSEC=""

# ---------------------------------------------------------------------
# Output helpers
# ---------------------------------------------------------------------

info()  { printf '\033[1;34m[switchover]\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m[warn]\033[0m %s\n' "$*" >&2; }
fail()  { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; }

die() {
    local msg="${1:-switchover failed}"
    local code="${2:-1}"
    fail "$msg"
    exit "$code"
}

on_error() {
    local exit_code=$1
    local line_no=$2
    echo ""
    fail "switch-stripe-to-live failed: exit $exit_code at line $line_no"
    echo ""
    echo "If the service is stopped, restart it manually:"
    echo "    systemctl start $SERVICE"
    echo ""
    echo "Inspect logs:"
    echo "    journalctl -u $SERVICE -n 100 --no-pager"
    exit "$exit_code"
}
trap 'on_error $? $LINENO' ERR

# ---------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------

print_help() {
    cat <<'HELP'
switch-stripe-to-live.sh — transition Coterie from Stripe test mode to
live mode in one shot.

USAGE
    sudo bash switch-stripe-to-live.sh [FLAGS]

FLAGS
    --discard-test-db   Delete the test database after migration rather
                        than archiving it. Default is to rename it to
                        coterie-test-archive-YYYYMMDD-HHMMSS.db.
    --yes               Skip the final confirmation prompt.
    --help              Show this help and exit.

PRECONDITIONS
    - Must run as root.
    - /opt/coterie/.env must NOT already contain pk_live_ (idempotency).
    - /var/lib/coterie/coterie-test.db must exist (proves test mode).
    - Live credentials available, either:
        * pre-loaded at provision time in /opt/coterie/.env.live, or
        * supplied interactively when the script prompts.

WHAT IT DOES
    1. Validates idempotency (refuses if already live, or no test DB).
    2. Loads (or prompts for) live Stripe credentials; validates shapes.
    3. Optionally hits Stripe's /v1/balance to confirm the secret key.
    4. Stops coterie.
    5. Creates a fresh live DB with schema copied from the test DB,
       and migrates the admin row(s) across via ATTACH DATABASE.
    6. Archives (or discards) the test DB.
    7. Atomically rewrites .env with live creds + live DB URL.
    8. Removes .env.live if present.
    9. Starts coterie; smoke-tests /health returns 200.
   10. Prints webhook-registration reminder.
HELP
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --discard-test-db) DISCARD_TEST_DB=true; shift ;;
        --yes|-y)          ASSUME_YES=true; shift ;;
        --help|-h)         print_help; exit 0 ;;
        *) fail "unknown flag: $1"; print_help; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------
# Preconditions
# ---------------------------------------------------------------------

if [[ $EUID -ne 0 ]]; then
    die "must run as root (sudo bash switch-stripe-to-live.sh)"
fi

if [[ ! -f "$ENV_FILE" ]]; then
    die "$ENV_FILE not found — was Coterie provisioned on this host?"
fi

# Idempotency #1: .env already in live mode → exit 0
if grep -Eq '^COTERIE__STRIPE__PUBLISHABLE_KEY=pk_live_' "$ENV_FILE"; then
    info "Already in live mode (.env's publishable key starts with pk_live_); nothing to do."
    exit 0
fi

# Idempotency #2: no test DB → can't migrate
if [[ ! -f "$TEST_DB" ]]; then
    die "Not in test mode; no test DB to migrate from ($TEST_DB does not exist)."
fi

# Required tools
for cmd in sqlite3 systemctl curl; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        die "required command '$cmd' not found on PATH"
    fi
done

# ---------------------------------------------------------------------
# Load (or prompt for) live credentials
# ---------------------------------------------------------------------

# prompt_secret_twice MESSAGE — read silently, confirm by re-prompt.
# Writes the value to stdout.
prompt_secret_twice() {
    local message="$1"
    local first="" second=""
    while true; do
        # shellcheck disable=SC2162
        read -s -p "${message}: " first >&2
        echo "" >&2
        if [[ -z "$first" ]]; then
            echo "  (cannot be empty)" >&2
            continue
        fi
        # shellcheck disable=SC2162
        read -s -p "${message} (confirm): " second >&2
        echo "" >&2
        if [[ "$first" == "$second" ]]; then
            printf '%s' "$first"
            return 0
        fi
        echo "  (mismatch — try again)" >&2
    done
}

if [[ -f "$ENV_LIVE_FILE" ]]; then
    info "Loading pre-loaded live credentials from $ENV_LIVE_FILE..."
    # shellcheck disable=SC1090
    # We read the three credential lines without sourcing arbitrary
    # shell content. The file is operator-controlled but parsing
    # keeps us safe from accidental shell escape characters.
    LIVE_PK="$(grep -E '^COTERIE__STRIPE__PUBLISHABLE_KEY=' "$ENV_LIVE_FILE" | head -n1 | cut -d= -f2-)"
    LIVE_SK="$(grep -E '^COTERIE__STRIPE__SECRET_KEY='      "$ENV_LIVE_FILE" | head -n1 | cut -d= -f2-)"
    LIVE_WHSEC="$(grep -E '^COTERIE__STRIPE__WEBHOOK_SECRET=' "$ENV_LIVE_FILE" | head -n1 | cut -d= -f2-)"
    if [[ -z "$LIVE_PK" || -z "$LIVE_SK" || -z "$LIVE_WHSEC" ]]; then
        die "$ENV_LIVE_FILE is missing one or more required credentials"
    fi
else
    info "No $ENV_LIVE_FILE present — prompting for live credentials interactively."
    # PK is non-secret; prompt openly.
    # shellcheck disable=SC2162
    read -r -p "Stripe LIVE publishable key (pk_live_…): " LIVE_PK
    LIVE_SK="$(prompt_secret_twice "Stripe LIVE secret key (sk_live_…)")"
    LIVE_WHSEC="$(prompt_secret_twice "Stripe LIVE webhook signing secret (whsec_…)")"
fi

# ---------------------------------------------------------------------
# Validate credential prefixes
# ---------------------------------------------------------------------

if [[ "$LIVE_PK" != pk_live_* ]]; then
    die "Live publishable key must start with 'pk_live_' (got: '${LIVE_PK:0:10}…')"
fi
if [[ "$LIVE_SK" != sk_live_* ]]; then
    die "Live secret key must start with 'sk_live_' (got: '${LIVE_SK:0:10}…')"
fi
if [[ "$LIVE_WHSEC" != whsec_* ]]; then
    die "Live webhook signing secret must start with 'whsec_' (got: '${LIVE_WHSEC:0:10}…')"
fi
info "Live credential prefixes look good."

# ---------------------------------------------------------------------
# Optional Stripe API smoke test (D7)
# ---------------------------------------------------------------------

info "Validating live secret key against Stripe (GET /v1/balance)..."
if curl -sf -u "$LIVE_SK:" https://api.stripe.com/v1/balance > /dev/null 2>&1; then
    info "Stripe accepted the live secret key."
else
    die "Stripe rejected the live secret key — aborting before any modifications."
fi

# ---------------------------------------------------------------------
# Confirmation
# ---------------------------------------------------------------------

if [[ "$DISCARD_TEST_DB" == "true" ]]; then
    test_db_action="DELETE the test DB ($TEST_DB)"
else
    test_db_action="archive the test DB to coterie-test-archive-<timestamp>.db"
fi

echo ""
echo "============================================================"
echo "About to switch Coterie from TEST mode to LIVE mode."
echo ""
echo "  Stop service:       systemctl stop $SERVICE"
echo "  Create live DB:     $LIVE_DB (schema copied from test DB)"
echo "  Migrate admin row:  ATTACH test DB → INSERT admins → DETACH"
echo "  Test DB:            $test_db_action"
echo "  Rewrite .env:       test creds → live creds, test DB → live DB"
if [[ -f "$ENV_LIVE_FILE" ]]; then
    echo "  Remove env.live:    rm $ENV_LIVE_FILE"
fi
echo "  Start service:      systemctl start $SERVICE"
echo "  Smoke test:         curl $HEALTH_URL"
echo "============================================================"
echo ""

if [[ "$ASSUME_YES" != "true" ]]; then
    if [[ -t 0 ]]; then
        confirm=""
        # shellcheck disable=SC2162
        read -r -p "Proceed? [y/N]: " confirm
        case "$confirm" in
            y|Y|yes|YES) ;;
            *) info "Aborted — no changes made."; exit 0 ;;
        esac
    else
        die "Non-interactive shell and --yes not given. Refusing to proceed without confirmation."
    fi
fi

# ---------------------------------------------------------------------
# Stop service
# ---------------------------------------------------------------------

info "Stopping $SERVICE..."
if ! systemctl stop "$SERVICE"; then
    die "systemctl stop $SERVICE failed — investigate before retrying."
fi

# ---------------------------------------------------------------------
# Build the live DB: schema from test DB, migrations table copy, admin
# rows copy.
# ---------------------------------------------------------------------

if [[ -f "$LIVE_DB" ]]; then
    # Should not happen given idempotency check #1, but guard anyway.
    die "$LIVE_DB already exists — refusing to overwrite. Investigate manually."
fi

info "Creating fresh $LIVE_DB with schema copied from test DB..."
sqlite3 "$TEST_DB" ".schema" | sqlite3 "$LIVE_DB"

info "Copying _sqlx_migrations tracker and admin row(s) from test DB..."
sqlite3 "$LIVE_DB" <<SQL
ATTACH DATABASE '$TEST_DB' AS test;
INSERT INTO _sqlx_migrations SELECT * FROM test._sqlx_migrations;
INSERT INTO members SELECT * FROM test.members WHERE is_admin = 1;
DETACH DATABASE test;
SQL

# Confirm at least one admin made it across.
admin_count="$(sqlite3 "$LIVE_DB" "SELECT COUNT(*) FROM members WHERE is_admin = 1;")"
if [[ "$admin_count" -lt 1 ]]; then
    die "No admin rows migrated to the live DB. Aborting — the .env is still test mode."
fi
info "Migrated $admin_count admin row(s) to $LIVE_DB."

# Ownership/permissions: coterie user must own the DB so the service
# can write to it after restart.
chown coterie:coterie "$LIVE_DB"
chmod 0640 "$LIVE_DB"

# ---------------------------------------------------------------------
# Archive or discard the test DB
# ---------------------------------------------------------------------

if [[ "$DISCARD_TEST_DB" == "true" ]]; then
    info "Discarding $TEST_DB (--discard-test-db)..."
    rm -f "$TEST_DB"
    # Also remove WAL/SHM sidecar files if they're hanging around.
    rm -f "${TEST_DB}-wal" "${TEST_DB}-shm"
else
    archive_name="$DATA_DIR/coterie-test-archive-$(date +%Y%m%d-%H%M%S).db"
    info "Archiving $TEST_DB → $archive_name"
    mv "$TEST_DB" "$archive_name"
    # The WAL/SHM sidecars are tied to the original file; move them too
    # to keep the archive self-consistent.
    [[ -f "${TEST_DB}-wal" ]] && mv "${TEST_DB}-wal" "${archive_name}-wal"
    [[ -f "${TEST_DB}-shm" ]] && mv "${TEST_DB}-shm" "${archive_name}-shm"
    chown coterie:coterie "$archive_name" "${archive_name}-wal" "${archive_name}-shm" 2>/dev/null || true
fi

# ---------------------------------------------------------------------
# Atomic .env rewrite
# ---------------------------------------------------------------------

info "Rewriting $ENV_FILE with live credentials..."

env_new="$INSTALL_DIR/.env.new"
# Read line-by-line; rewrite the Stripe credential lines and the
# DATABASE URL. Anything else passes through verbatim.
while IFS= read -r line || [[ -n "$line" ]]; do
    case "$line" in
        COTERIE__STRIPE__PUBLISHABLE_KEY=*)
            printf '%s\n' "COTERIE__STRIPE__PUBLISHABLE_KEY=${LIVE_PK}"
            ;;
        COTERIE__STRIPE__SECRET_KEY=*)
            printf '%s\n' "COTERIE__STRIPE__SECRET_KEY=${LIVE_SK}"
            ;;
        COTERIE__STRIPE__WEBHOOK_SECRET=*)
            printf '%s\n' "COTERIE__STRIPE__WEBHOOK_SECRET=${LIVE_WHSEC}"
            ;;
        COTERIE__DATABASE__URL=*)
            # Swap any reference to the test DB filename to the live one.
            # Covers absolute paths (sqlite:///var/.../coterie-test.db) and
            # the bare form (sqlite://coterie-test.db).
            printf '%s\n' "${line//coterie-test.db/coterie.db}"
            ;;
        *)
            printf '%s\n' "$line"
            ;;
    esac
done < "$ENV_FILE" > "$env_new"

chown coterie:coterie "$env_new"
chmod 0640 "$env_new"
mv "$env_new" "$ENV_FILE"
info ".env rewritten."

# Clean up .env.live — its credentials are now in .env; leaving a second
# plaintext copy on disk is a needless security risk.
if [[ -f "$ENV_LIVE_FILE" ]]; then
    info "Removing $ENV_LIVE_FILE (credentials are now in .env)..."
    rm -f "$ENV_LIVE_FILE"
fi

# ---------------------------------------------------------------------
# Start service + smoke test
# ---------------------------------------------------------------------

info "Starting $SERVICE..."
systemctl start "$SERVICE"

waited=0
while (( waited < 30 )); do
    if systemctl is-active --quiet "$SERVICE"; then
        break
    fi
    sleep 1
    waited=$((waited + 1))
done

if ! systemctl is-active --quiet "$SERVICE"; then
    fail "$SERVICE did not reach active state within 30 seconds."
    fail ".env is now in live mode but the service is down — investigate:"
    journalctl -u "$SERVICE" -n 50 --no-pager || true
    die "service failed to start after switchover"
fi
info "$SERVICE active after ${waited}s."

info "Smoke-testing $HEALTH_URL ..."
health_resp="$(mktemp)"
if ! curl -fsS -i "$HEALTH_URL" -o "$health_resp"; then
    fail "curl $HEALTH_URL failed."
    cat "$health_resp" >&2 || true
    rm -f "$health_resp"
    journalctl -u "$SERVICE" -n 50 --no-pager >&2 || true
    die "smoke test failed"
fi

first_line="$(head -n1 "$health_resp")"
if [[ "$first_line" != *"200"* ]]; then
    fail "/health did not return 200. Response head:"
    head -n5 "$health_resp" >&2
    rm -f "$health_resp"
    journalctl -u "$SERVICE" -n 50 --no-pager >&2 || true
    die "smoke test failed (unexpected status)"
fi

if grep -qi 'Location:.*/setup' "$health_resp"; then
    fail "/health redirected to /setup — admin migration may have failed."
    rm -f "$health_resp"
    journalctl -u "$SERVICE" -n 50 --no-pager >&2 || true
    die "smoke test failed (admin row not visible to server)"
fi
rm -f "$health_resp"
info "Smoke test OK."

# ---------------------------------------------------------------------
# Success summary + webhook reminder (D9)
# ---------------------------------------------------------------------

# Try to extract the portal URL from .env for the reminder message.
portal_url="$(grep -E '^COTERIE__SERVER__BASE_URL=' "$ENV_FILE" | head -n1 | cut -d= -f2- || true)"
if [[ -z "$portal_url" ]]; then
    portal_url="https://<your-portal-domain>"
fi

cat <<EOF

============================================================
Switched to Stripe LIVE mode.

IMPORTANT: verify the LIVE-mode webhook endpoint is registered in
your Stripe dashboard:

  Stripe dashboard → toggle to LIVE mode → Developers → Webhooks
  → confirm an endpoint exists for:
       ${portal_url}/api/payments/webhook/stripe
  → confirm the signing secret matches the whsec_ value you just
       supplied to this script

Without a live-mode webhook registered, real charges will go through
Stripe but Coterie will never hear about them — dues will never
extend, payments will never advance from Pending.
============================================================
EOF

info "Done."
