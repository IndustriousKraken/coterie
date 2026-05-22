#!/usr/bin/env bash
# Coterie provisioning wizard — fresh Debian 13 host to a running Coterie
# instance with TLS, first admin, and optional integrations.
#
# Usage:
#   sudo bash provision.sh                  # interactive
#   sudo bash provision.sh --dry-run        # print plan, do nothing
#   sudo bash provision.sh --help           # env-var list + flag reference
#
# Curl-and-bash one-liner:
#   curl -sfL https://raw.githubusercontent.com/IndustriousKraken/coterie/master/deploy/provision.sh \
#       -o /tmp/provision.sh && sudo bash /tmp/provision.sh
#
# Every interactive prompt has a matching COTERIE_PROVISION_* env var.
# Set them all to run non-interactively (suitable for IaC). Mixed mode
# (some vars set, others prompted) also works. Run with --help to see
# the full list.

set -euo pipefail

# ---------------------------------------------------------------------
# Globals
# ---------------------------------------------------------------------

REPO="IndustriousKraken/coterie"
INSTALL_DIR="/opt/coterie"
DATA_DIR="/var/lib/coterie"
CADDYFILE_DST="/etc/caddy/Caddyfile"
RELEASE_DEPLOY_DST="/usr/local/bin/coterie-release-deploy"
RELEASE_DEPLOY_URL="https://raw.githubusercontent.com/${REPO}/master/deploy/release-deploy.sh"

DRY_RUN=false
# Idempotency flags (set in preflight)
EXISTING_ENV=false
EXISTING_ADMIN=false
EXISTING_CADDYFILE=false

# Collected inputs (set in section 3)
ORG_NAME=""
PORTAL_DOMAIN=""
MARKETING_DOMAIN=""
CONTACT_EMAIL=""
SELECTED_VERSION=""
ADMIN_EMAIL=""
ADMIN_USERNAME=""
ADMIN_FULL_NAME=""
ADMIN_PASSWORD=""
ENABLE_STRIPE=false
STRIPE_MODE="live"            # "test" or "live"; default matches a24 baseline
STRIPE_PK=""
STRIPE_SK=""
STRIPE_WHSEC=""
PRELOAD_LIVE=false            # true if operator pre-loads live creds during test-mode wizard
STRIPE_LIVE_PK=""
STRIPE_LIVE_SK=""
STRIPE_LIVE_WHSEC=""
ENABLE_DISCORD=false
DISCORD_BOT_TOKEN=""
DISCORD_GUILD_ID=""
DISCORD_ANNOUNCE_CHANNEL=""
ENABLE_UNIFI=false
UNIFI_URL=""
UNIFI_USERNAME=""
UNIFI_PASSWORD=""
ENABLE_CADDY=true

# ---------------------------------------------------------------------
# Output helpers
# ---------------------------------------------------------------------

info()  { printf '\033[1;34m[provision]\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m[warn]\033[0m %s\n' "$*" >&2; }
fail()  { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; }

die() {
    local msg="${1:-provisioning failed}"
    local code="${2:-1}"
    fail "$msg"
    exit "$code"
}

on_error() {
    local exit_code=$1
    local line_no=$2
    echo ""
    fail "Provision failed: exit $exit_code at line $line_no"
    echo ""
    echo "Recovery options:"
    echo "  - Re-run the wizard; idempotency checks should skip steps that succeeded."
    echo "  - For a clean slate:"
    echo "      bash ${INSTALL_DIR}/deploy/uninstall.sh         # keep data + .env"
    echo "      bash ${INSTALL_DIR}/deploy/uninstall.sh --all   # nuke everything"
    echo "  - If ${INSTALL_DIR} is missing entirely:"
    echo "      curl -sfL https://raw.githubusercontent.com/${REPO}/master/deploy/uninstall.sh \\"
    echo "          -o /tmp/uninstall.sh && bash /tmp/uninstall.sh --all --yes"
    exit "$exit_code"
}
trap 'on_error $? $LINENO' ERR

# ---------------------------------------------------------------------
# Prompt helpers
# ---------------------------------------------------------------------

# prompt VAR_NAME "message" [default]
# Stores the result in the named global variable. If COTERIE_PROVISION_<VAR_NAME>
# is set, uses it directly with no prompt.
prompt() {
    local var_name="$1"
    local message="$2"
    local default="${3:-}"
    local env_name="COTERIE_PROVISION_${var_name}"
    local existing="${!env_name:-}"
    local value=""

    if [[ -n "$existing" ]]; then
        value="$existing"
        info "$var_name = $existing (from \$$env_name)"
    else
        local suffix=""
        if [[ -n "$default" ]]; then
            suffix=" [$default]"
        fi
        # shellcheck disable=SC2162
        read -r -p "${message}${suffix}: " value
        if [[ -z "$value" && -n "$default" ]]; then
            value="$default"
        fi
    fi

    printf -v "$var_name" '%s' "$value"
}

# prompt_secret VAR_NAME "message" — does not echo, confirms by re-prompt.
prompt_secret() {
    local var_name="$1"
    local message="$2"
    local env_name="COTERIE_PROVISION_${var_name}"
    local existing="${!env_name:-}"

    if [[ -n "$existing" ]]; then
        printf -v "$var_name" '%s' "$existing"
        info "$var_name = (set from \$$env_name)"
        return 0
    fi

    local first="" second=""
    while true; do
        # shellcheck disable=SC2162
        read -s -p "${message}: " first
        echo ""
        if [[ -z "$first" ]]; then
            echo "  (cannot be empty)"
            continue
        fi
        # shellcheck disable=SC2162
        read -s -p "${message} (confirm): " second
        echo ""
        if [[ "$first" == "$second" ]]; then
            printf -v "$var_name" '%s' "$first"
            return 0
        fi
        echo "  (mismatch — try again)"
    done
}

# prompt_yn VAR_NAME "message" [default y|n]
# Stores 'true' or 'false' in the named variable.
prompt_yn() {
    local var_name="$1"
    local message="$2"
    local default="${3:-n}"
    local env_name="COTERIE_PROVISION_${var_name}"
    local existing="${!env_name:-}"

    if [[ -n "$existing" ]]; then
        case "$existing" in
            true|yes|y|1)  printf -v "$var_name" '%s' "true" ;;
            false|no|n|0)  printf -v "$var_name" '%s' "false" ;;
            *) die "invalid value for \$$env_name: $existing (expected true/false)" ;;
        esac
        info "$var_name = ${!var_name} (from \$$env_name)"
        return 0
    fi

    local suffix
    if [[ "$default" == "y" ]]; then suffix="[Y/n]"; else suffix="[y/N]"; fi
    local reply=""
    while true; do
        # shellcheck disable=SC2162
        read -r -p "${message} ${suffix}: " reply
        if [[ -z "$reply" ]]; then reply="$default"; fi
        case "$reply" in
            y|Y|yes|YES) printf -v "$var_name" '%s' "true"; return 0 ;;
            n|N|no|NO)   printf -v "$var_name" '%s' "false"; return 0 ;;
            *) echo "  (please answer y or n)" ;;
        esac
    done
}

# validate_prefix VAR_NAME EXPECTED_PREFIX — die if the named variable's
# value does not start with EXPECTED_PREFIX. Used for Stripe credential
# checks in test mode (and live-creds-pre-load) so an obviously wrong
# value is caught before it lands on disk.
validate_prefix() {
    local var_name="$1"
    local expected="$2"
    local value="${!var_name}"
    if [[ "$value" != "${expected}"* ]]; then
        die "${var_name} must start with '${expected}' (got: '${value:0:10}…'). Refusing to continue."
    fi
}

# run CMD ARGS… — wrapper that respects DRY_RUN. Logs the command, then
# runs it unless DRY_RUN is true.
run() {
    if [[ "$DRY_RUN" == "true" ]]; then
        echo "  [dry-run] $*"
        return 0
    fi
    "$@"
}

# ---------------------------------------------------------------------
# --help / --dry-run parsing
# ---------------------------------------------------------------------

print_help() {
    cat <<'HELP'
Coterie provisioning wizard

USAGE
    sudo bash provision.sh [--dry-run] [--help]

FLAGS
    --dry-run     Print the plan + the .env / Caddyfile that would be
                  written, but do not modify the system.
    --help        Show this help and exit.

ENVIRONMENT VARIABLES
    Each interactive prompt checks for a matching env var first. Set
    them all to run unattended (suitable for IaC).

  Core
    COTERIE_PROVISION_ORG_NAME           Organization name (free text)
    COTERIE_PROVISION_PORTAL_DOMAIN      e.g. coterie.example.com
    COTERIE_PROVISION_MARKETING_DOMAIN   Optional, e.g. example.com
    COTERIE_PROVISION_CONTACT_EMAIL      Org contact for admin alerts
    COTERIE_PROVISION_VERSION            Coterie release tag (e.g. v1.0.0)
                                         Defaults to latest stable.

  First admin
    COTERIE_PROVISION_ADMIN_EMAIL
    COTERIE_PROVISION_ADMIN_USERNAME
    COTERIE_PROVISION_ADMIN_FULL_NAME
    COTERIE_PROVISION_ADMIN_PASSWORD     Sensitive — visible via /proc.
                                         Acceptable on single-user IaC boxes.

  Integrations (each gated by ENABLE_*; defaults false except CADDY=true)
    COTERIE_PROVISION_ENABLE_STRIPE      true/false
    COTERIE_PROVISION_STRIPE_MODE        test|live (default: live)
                                         test mode uses coterie-test.db;
                                         switch later with
                                         deploy/switch-stripe-to-live.sh.
    COTERIE_PROVISION_STRIPE_PK          Stripe publishable key (active mode)
    COTERIE_PROVISION_STRIPE_SK          Stripe secret key (active mode)
    COTERIE_PROVISION_STRIPE_WHSEC       Stripe webhook signing secret (active mode)
    COTERIE_PROVISION_PRELOAD_LIVE       true/false — in test mode, also
                                         collect live creds and stash in
                                         /opt/coterie/.env.live for later.
    COTERIE_PROVISION_STRIPE_LIVE_PK     Pre-loaded live publishable key
    COTERIE_PROVISION_STRIPE_LIVE_SK     Pre-loaded live secret key
    COTERIE_PROVISION_STRIPE_LIVE_WHSEC  Pre-loaded live webhook signing secret

    COTERIE_PROVISION_ENABLE_DISCORD     true/false
    COTERIE_PROVISION_DISCORD_BOT_TOKEN
    COTERIE_PROVISION_DISCORD_GUILD_ID
    COTERIE_PROVISION_DISCORD_ANNOUNCE_CHANNEL

    COTERIE_PROVISION_ENABLE_UNIFI       true/false
    COTERIE_PROVISION_UNIFI_URL          Controller URL
    COTERIE_PROVISION_UNIFI_USERNAME
    COTERIE_PROVISION_UNIFI_PASSWORD

    COTERIE_PROVISION_ENABLE_CADDY       true/false (default true)

  Idempotency
    COTERIE_PROVISION_OVERWRITE_ENV      true/false — clobber existing
                                         /opt/coterie/.env without prompting

EXAMPLES
    # Interactive (recommended for first run)
    sudo bash provision.sh

    # Dry run — print the plan
    sudo bash provision.sh --dry-run

    # Fully scripted
    COTERIE_PROVISION_ORG_NAME="Neon Temple" \
    COTERIE_PROVISION_PORTAL_DOMAIN="coterie.example.com" \
    COTERIE_PROVISION_CONTACT_EMAIL="ops@example.com" \
    COTERIE_PROVISION_ADMIN_EMAIL="rab@example.com" \
    COTERIE_PROVISION_ADMIN_USERNAME="rab" \
    COTERIE_PROVISION_ADMIN_FULL_NAME="R. Beverly" \
    COTERIE_PROVISION_ADMIN_PASSWORD='hunter2-correct-horse' \
    COTERIE_PROVISION_ENABLE_STRIPE=false \
    COTERIE_PROVISION_ENABLE_CADDY=true \
        sudo -E bash provision.sh

Recovery if something goes wrong:
    bash /opt/coterie/deploy/uninstall.sh           # keep data + .env
    bash /opt/coterie/deploy/uninstall.sh --all     # nuke everything
HELP
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run) DRY_RUN=true; shift ;;
        --help|-h) print_help; exit 0 ;;
        *) fail "unknown flag: $1"; print_help; exit 1 ;;
    esac
done

if [[ "$DRY_RUN" == "true" ]]; then
    info "DRY-RUN MODE — no changes will be made."
fi

# ---------------------------------------------------------------------
# 2. Preflight checks
# ---------------------------------------------------------------------

info "Running preflight checks..."

# 2.1 Root
if [[ $EUID -ne 0 ]]; then
    die "must run as root (sudo bash provision.sh)"
fi

# 2.2 Debian
if [[ ! -r /etc/os-release ]]; then
    die "/etc/os-release missing — can't identify distro. See deploy/DEPLOY-ALPINE.md or DEPLOY-AWS.md."
fi
# shellcheck disable=SC1091
. /etc/os-release
if [[ "${ID:-}" != "debian" ]]; then
    fail "this wizard targets Debian (detected ID=${ID:-unknown})."
    fail "For Ubuntu/other distros, see deploy/DEPLOY-DIGITALOCEAN.md (manual steps work on Ubuntu)."
    fail "For Alpine, see deploy/DEPLOY-ALPINE.md."
    exit 1
fi
info "Detected Debian ${VERSION_ID:-?}"

# 2.3 Volume mount warning
if [[ -d "$DATA_DIR" ]]; then
    # mountpoint may not be installed on a truly minimal Debian image;
    # treat its absence as "can't verify" rather than failing.
    if command -v mountpoint >/dev/null 2>&1; then
        if ! mountpoint -q "$DATA_DIR"; then
            warn "$DATA_DIR exists but is not a mountpoint."
            warn "If you intended to attach a block volume here, mount it BEFORE provisioning."
            warn "(Continuing — local-disk storage works fine for small deployments.)"
        fi
    fi
fi

# 2.4 Idempotency detection
if [[ -f "$INSTALL_DIR/.env" ]]; then
    EXISTING_ENV=true
    info "Detected existing $INSTALL_DIR/.env (will prompt before overwriting)."
fi

if [[ -f "$DATA_DIR/coterie.db" ]] && command -v sqlite3 >/dev/null 2>&1; then
    if sqlite3 "$DATA_DIR/coterie.db" \
            "SELECT 1 FROM members WHERE is_admin = 1 LIMIT 1;" 2>/dev/null \
            | grep -q 1; then
        EXISTING_ADMIN=true
        info "Detected existing admin row — create_admin step will be skipped."
    fi
fi

if [[ -f "$CADDYFILE_DST" ]]; then
    EXISTING_CADDYFILE=true
    info "Detected existing $CADDYFILE_DST (will prompt before overwriting)."
fi

# ---------------------------------------------------------------------
# 3. Collect inputs
# ---------------------------------------------------------------------

echo ""
info "=== Step 1: Organization details ==="

prompt ORG_NAME       "Organization name"
prompt PORTAL_DOMAIN  "Portal domain (e.g. coterie.example.com)"

if [[ -z "$ORG_NAME" || -z "$PORTAL_DOMAIN" ]]; then
    die "organization name and portal domain are required"
fi

# 3.2a Version selection
echo ""
info "=== Step 2: Coterie version ==="
select_version() {
    if [[ -n "${COTERIE_PROVISION_VERSION:-}" ]]; then
        SELECTED_VERSION="$COTERIE_PROVISION_VERSION"
        info "Version: $SELECTED_VERSION (from \$COTERIE_PROVISION_VERSION)"
        return 0
    fi

    local releases_json
    releases_json="$(mktemp)"
    if ! curl -sfL "https://api.github.com/repos/${REPO}/releases?per_page=10" -o "$releases_json"; then
        warn "couldn't fetch releases list from GitHub — defaulting to latest stable."
        rm -f "$releases_json"
        SELECTED_VERSION=""
        return 0
    fi

    local stable_tags
    stable_tags="$(python3 -c "
import json, sys
try:
    with open('${releases_json}') as f:
        rels = json.load(f, strict=False)
except Exception as e:
    sys.exit(0)
stable = [r['tag_name'] for r in rels if not r.get('prerelease')]
for t in stable[:5]:
    print(t)
" || true)"

    local all_tags
    all_tags="$(python3 -c "
import json, sys
try:
    with open('${releases_json}') as f:
        rels = json.load(f, strict=False)
except Exception as e:
    sys.exit(0)
for r in rels:
    flag = 'pre' if r.get('prerelease') else 'stable'
    print(f\"{r['tag_name']}\\t{flag}\")
" || true)"

    rm -f "$releases_json"

    if [[ -z "$stable_tags" ]]; then
        warn "no stable releases found — release-deploy.sh will pick latest available."
        SELECTED_VERSION=""
        return 0
    fi

    # shellcheck disable=SC2206
    local stable_arr=($stable_tags)
    local default_tag="${stable_arr[0]}"

    echo "Recent stable releases:"
    local i=1
    for tag in "${stable_arr[@]}"; do
        if [[ $i -eq 1 ]]; then
            echo "  $i) $tag  (default — latest stable)"
        else
            echo "  $i) $tag"
        fi
        i=$((i + 1))
    done
    echo "  $i) show all releases (including prereleases)"
    local all_option=$i

    local choice=""
    # shellcheck disable=SC2162
    read -r -p "Pick a release [1]: " choice
    if [[ -z "$choice" ]]; then choice=1; fi

    if [[ "$choice" == "$all_option" ]]; then
        echo ""
        echo "All recent releases:"
        local j=1
        local all_arr=()
        while IFS=$'\t' read -r tag flag; do
            [[ -z "$tag" ]] && continue
            if [[ "$flag" == "pre" ]]; then
                echo "  $j) $tag  [prerelease]"
            else
                echo "  $j) $tag"
            fi
            all_arr+=("$tag")
            j=$((j + 1))
        done <<< "$all_tags"
        # shellcheck disable=SC2162
        read -r -p "Pick a release [1]: " choice
        if [[ -z "$choice" ]]; then choice=1; fi
        if ! [[ "$choice" =~ ^[0-9]+$ ]] || (( choice < 1 || choice > ${#all_arr[@]} )); then
            die "invalid choice"
        fi
        SELECTED_VERSION="${all_arr[$((choice - 1))]}"
    elif [[ "$choice" =~ ^[0-9]+$ ]] && (( choice >= 1 && choice <= ${#stable_arr[@]} )); then
        SELECTED_VERSION="${stable_arr[$((choice - 1))]}"
    else
        warn "invalid choice — using default ($default_tag)"
        SELECTED_VERSION="$default_tag"
    fi
    info "Selected version: $SELECTED_VERSION"
}
select_version

# 3.3 / 3.4
prompt MARKETING_DOMAIN "Marketing domain (optional, e.g. example.com)"
prompt CONTACT_EMAIL    "Org contact email (for admin alerts)"

# 3.5 Admin credentials
echo ""
info "=== Step 3: First admin ==="
if [[ "$EXISTING_ADMIN" == "true" ]]; then
    info "Skipping admin-credential prompts (admin already exists in DB)."
else
    prompt ADMIN_EMAIL     "Admin email"
    prompt ADMIN_USERNAME  "Admin username"
    prompt ADMIN_FULL_NAME "Admin full name"
    prompt_secret ADMIN_PASSWORD "Admin password (min 8 chars)"
    if [[ -z "$ADMIN_EMAIL" || -z "$ADMIN_USERNAME" || -z "$ADMIN_FULL_NAME" || -z "$ADMIN_PASSWORD" ]]; then
        die "admin email, username, full name, and password are required"
    fi
    if (( ${#ADMIN_PASSWORD} < 8 )); then
        warn "admin password is shorter than 8 characters — continuing anyway."
    fi
fi

# 3.6 Stripe
echo ""
info "=== Step 4: Integrations ==="
prompt_yn ENABLE_STRIPE "Enable Stripe payments?" "n"
if [[ "$ENABLE_STRIPE" == "true" ]]; then
    # a25: test-or-live mode choice. Default 'live' preserves a24 behavior.
    # When 'test', a separate coterie-test.db is used so test charges don't
    # pollute the eventual live database. See deploy/switch-stripe-to-live.sh.
    prompt STRIPE_MODE "Stripe mode (test/live)" "live"
    case "$STRIPE_MODE" in
        test|live) ;;
        *) die "STRIPE_MODE must be 'test' or 'live' (got: '$STRIPE_MODE')" ;;
    esac

    if [[ "$STRIPE_MODE" == "test" ]]; then
        prompt        STRIPE_PK     "Stripe TEST publishable key (pk_test_…)"
        validate_prefix STRIPE_PK "pk_test_"
        prompt_secret STRIPE_SK     "Stripe TEST secret key (sk_test_…)"
        validate_prefix STRIPE_SK "sk_test_"
        prompt_secret STRIPE_WHSEC  "Stripe TEST webhook signing secret (whsec_…)"
        validate_prefix STRIPE_WHSEC "whsec_"

        prompt_yn PRELOAD_LIVE "Do you also have live credentials to pre-load for later switchover?" "n"
        if [[ "$PRELOAD_LIVE" == "true" ]]; then
            prompt        STRIPE_LIVE_PK    "Stripe LIVE publishable key (pk_live_…)"
            validate_prefix STRIPE_LIVE_PK "pk_live_"
            prompt_secret STRIPE_LIVE_SK    "Stripe LIVE secret key (sk_live_…)"
            validate_prefix STRIPE_LIVE_SK "sk_live_"
            prompt_secret STRIPE_LIVE_WHSEC "Stripe LIVE webhook signing secret (whsec_…)"
            validate_prefix STRIPE_LIVE_WHSEC "whsec_"
        fi
    else
        # Live mode: behavior matches a24 baseline.
        prompt        STRIPE_PK     "Stripe publishable key (pk_live_…)"
        prompt_secret STRIPE_SK     "Stripe secret key (sk_live_…)"
        prompt_secret STRIPE_WHSEC  "Stripe webhook signing secret (whsec_…)"
    fi
fi

# 3.7 Discord
prompt_yn ENABLE_DISCORD "Enable Discord integration?" "n"
if [[ "$ENABLE_DISCORD" == "true" ]]; then
    prompt_secret DISCORD_BOT_TOKEN       "Discord bot token"
    prompt        DISCORD_GUILD_ID        "Discord guild ID"
    prompt        DISCORD_ANNOUNCE_CHANNEL "Discord announcements channel ID"
fi

# 3.8 UniFi
prompt_yn ENABLE_UNIFI "Enable UniFi Access integration?" "n"
if [[ "$ENABLE_UNIFI" == "true" ]]; then
    prompt        UNIFI_URL       "UniFi controller URL"
    prompt        UNIFI_USERNAME  "UniFi username"
    prompt_secret UNIFI_PASSWORD  "UniFi password"
fi

# 3.9 Caddy
prompt_yn ENABLE_CADDY "Install and configure Caddy (recommended)?" "y"

# 3.10 Confirmation summary
echo ""
echo "=============================================================="
echo "Provisioning plan"
echo "=============================================================="
echo "  Organization:       $ORG_NAME"
echo "  Portal domain:      https://$PORTAL_DOMAIN"
if [[ -n "$MARKETING_DOMAIN" ]]; then
    echo "  Marketing domain:   https://$MARKETING_DOMAIN"
fi
echo "  Contact email:      $CONTACT_EMAIL"
if [[ -n "$SELECTED_VERSION" ]]; then
    echo "  Coterie version:    $SELECTED_VERSION"
else
    echo "  Coterie version:    (latest stable from GitHub)"
fi
if [[ "$EXISTING_ADMIN" == "true" ]]; then
    echo "  Admin:              (already exists — keeping)"
else
    echo "  Admin:              $ADMIN_EMAIL ($ADMIN_USERNAME)"
fi
if [[ "$ENABLE_STRIPE" == "true" ]]; then
    echo "  Stripe:             $ENABLE_STRIPE (mode: $STRIPE_MODE)"
    if [[ "$STRIPE_MODE" == "test" && "$PRELOAD_LIVE" == "true" ]]; then
        echo "                      (pre-loading live creds for switchover)"
    fi
else
    echo "  Stripe:             $ENABLE_STRIPE"
fi
echo "  Discord:            $ENABLE_DISCORD"
echo "  UniFi:              $ENABLE_UNIFI"
echo "  Caddy + TLS:        $ENABLE_CADDY"
echo "=============================================================="
echo ""

if [[ "$DRY_RUN" == "true" ]]; then
    info "Dry-run complete — no changes were made. Re-run without --dry-run to provision."
    exit 0
fi

if [[ -z "${COTERIE_PROVISION_ASSUME_YES:-}" ]]; then
    # Only prompt for confirmation if we're actually interactive.
    if [[ -t 0 ]]; then
        confirm=""
        # shellcheck disable=SC2162
        read -r -p "Proceed with provisioning? [y/N]: " confirm
        case "$confirm" in
            y|Y|yes|YES) ;;
            *) info "Aborted."; exit 0 ;;
        esac
    else
        info "Non-interactive shell — proceeding (set COTERIE_PROVISION_ASSUME_YES=true to silence this hint)."
    fi
fi

# ---------------------------------------------------------------------
# 4. System dependencies
# ---------------------------------------------------------------------

echo ""
info "=== Installing system dependencies ==="

export DEBIAN_FRONTEND=noninteractive

run apt-get update
run apt-get install -y --no-install-recommends \
    curl python3 tar sqlite3 ca-certificates openssl

if [[ "$ENABLE_CADDY" == "true" ]]; then
    info "Installing Caddy from Cloudsmith..."
    if ! command -v caddy >/dev/null 2>&1; then
        run apt-get install -y --no-install-recommends \
            debian-keyring debian-archive-keyring apt-transport-https gnupg
        if [[ "$DRY_RUN" != "true" ]]; then
            curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
                | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
            curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
                | tee /etc/apt/sources.list.d/caddy-stable.list > /dev/null
        fi
        run apt-get update
        run apt-get install -y caddy
    else
        info "Caddy already installed — skipping repo setup."
    fi
fi

# ---------------------------------------------------------------------
# 5. Pull and install Coterie
# ---------------------------------------------------------------------

echo ""
info "=== Installing Coterie ==="

# 5.1: fetch release-deploy.sh
if [[ ! -x "$RELEASE_DEPLOY_DST" ]]; then
    info "Fetching release-deploy.sh from GitHub..."
    if [[ "$DRY_RUN" != "true" ]]; then
        curl -sfL "$RELEASE_DEPLOY_URL" -o "$RELEASE_DEPLOY_DST"
        chmod +x "$RELEASE_DEPLOY_DST"
    else
        echo "  [dry-run] curl -sfL $RELEASE_DEPLOY_URL -o $RELEASE_DEPLOY_DST && chmod +x ..."
    fi
else
    info "release-deploy.sh already present at $RELEASE_DEPLOY_DST"
fi

# 5.2: run it
if [[ -n "$SELECTED_VERSION" ]]; then
    run "$RELEASE_DEPLOY_DST" "$SELECTED_VERSION"
else
    run "$RELEASE_DEPLOY_DST"
fi

# 5.3: verify binaries are in place
if [[ "$DRY_RUN" != "true" ]]; then
    if [[ ! -x "$INSTALL_DIR/coterie" ]]; then
        die "expected $INSTALL_DIR/coterie after release-deploy, but it's missing or not executable"
    fi
    if [[ ! -x "$INSTALL_DIR/create_admin" ]]; then
        die "expected $INSTALL_DIR/create_admin after release-deploy, but it's missing or not executable. \
This release may pre-date the create_admin bootstrap binary — pick a newer tag and re-run."
    fi
    info "Verified $INSTALL_DIR/coterie and $INSTALL_DIR/create_admin are present and executable."
fi

# ---------------------------------------------------------------------
# 6. Generate /opt/coterie/.env
# ---------------------------------------------------------------------

echo ""
info "=== Generating /opt/coterie/.env ==="

generate_env() {
    local template="$INSTALL_DIR/.env.example"
    if [[ ! -f "$template" ]]; then
        die "$template missing — release-deploy.sh should have placed it"
    fi

    local session_secret
    session_secret="$(openssl rand -hex 32)"

    local base_url="https://${PORTAL_DOMAIN}"
    local cors_origins=""
    if [[ -n "$MARKETING_DOMAIN" ]]; then
        cors_origins="https://${MARKETING_DOMAIN},https://www.${MARKETING_DOMAIN}"
    fi

    local target="$INSTALL_DIR/.env"

    # Start from the template, then transform line-by-line.
    local tmp
    tmp="$(mktemp)"

    while IFS= read -r line || [[ -n "$line" ]]; do
        case "$line" in
            "COTERIE__SERVER__BASE_URL="*)
                printf '%s\n' "COTERIE__SERVER__BASE_URL=${base_url}"
                ;;
            "# COTERIE__SERVER__CORS_ORIGINS="*|"COTERIE__SERVER__CORS_ORIGINS="*)
                if [[ -n "$cors_origins" ]]; then
                    printf '%s\n' "COTERIE__SERVER__CORS_ORIGINS=${cors_origins}"
                else
                    printf '%s\n' "$line"
                fi
                ;;
            "COTERIE__AUTH__SESSION_SECRET="*)
                printf '%s\n' "COTERIE__AUTH__SESSION_SECRET=${session_secret}"
                ;;
            "# COTERIE__SERVER__DATA_DIR="*|"COTERIE__SERVER__DATA_DIR="*)
                printf '%s\n' "COTERIE__SERVER__DATA_DIR=${DATA_DIR}"
                ;;
            "COTERIE__DATABASE__URL="*)
                # a25: in test mode the wizard targets a separate test DB
                # so verification data doesn't pollute the eventual live DB.
                if [[ "$ENABLE_STRIPE" == "true" && "$STRIPE_MODE" == "test" ]]; then
                    printf '%s\n' "COTERIE__DATABASE__URL=sqlite:///var/lib/coterie/coterie-test.db?mode=rwc"
                else
                    printf '%s\n' "$line"
                fi
                ;;
            "COTERIE__STRIPE__ENABLED="*)
                printf '%s\n' "COTERIE__STRIPE__ENABLED=${ENABLE_STRIPE}"
                ;;
            "# COTERIE__STRIPE__PUBLISHABLE_KEY="*|"COTERIE__STRIPE__PUBLISHABLE_KEY="*)
                if [[ "$ENABLE_STRIPE" == "true" ]]; then
                    printf '%s\n' "COTERIE__STRIPE__PUBLISHABLE_KEY=${STRIPE_PK}"
                else
                    printf '%s\n' "$line"
                fi
                ;;
            "# COTERIE__STRIPE__SECRET_KEY="*|"COTERIE__STRIPE__SECRET_KEY="*)
                if [[ "$ENABLE_STRIPE" == "true" ]]; then
                    printf '%s\n' "COTERIE__STRIPE__SECRET_KEY=${STRIPE_SK}"
                else
                    printf '%s\n' "$line"
                fi
                ;;
            "# COTERIE__STRIPE__WEBHOOK_SECRET="*|"COTERIE__STRIPE__WEBHOOK_SECRET="*)
                if [[ "$ENABLE_STRIPE" == "true" ]]; then
                    printf '%s\n' "COTERIE__STRIPE__WEBHOOK_SECRET=${STRIPE_WHSEC}"
                else
                    printf '%s\n' "$line"
                fi
                ;;
            "# COTERIE__INTEGRATIONS__DISCORD__ENABLED="*|"COTERIE__INTEGRATIONS__DISCORD__ENABLED="*)
                printf '%s\n' "COTERIE__INTEGRATIONS__DISCORD__ENABLED=${ENABLE_DISCORD}"
                ;;
            "# COTERIE__INTEGRATIONS__DISCORD__BOT_TOKEN="*|"COTERIE__INTEGRATIONS__DISCORD__BOT_TOKEN="*)
                if [[ "$ENABLE_DISCORD" == "true" ]]; then
                    printf '%s\n' "COTERIE__INTEGRATIONS__DISCORD__BOT_TOKEN=${DISCORD_BOT_TOKEN}"
                else
                    printf '%s\n' "$line"
                fi
                ;;
            "# COTERIE__INTEGRATIONS__DISCORD__GUILD_ID="*|"COTERIE__INTEGRATIONS__DISCORD__GUILD_ID="*)
                if [[ "$ENABLE_DISCORD" == "true" ]]; then
                    printf '%s\n' "COTERIE__INTEGRATIONS__DISCORD__GUILD_ID=${DISCORD_GUILD_ID}"
                else
                    printf '%s\n' "$line"
                fi
                ;;
            "# COTERIE__INTEGRATIONS__DISCORD__ANNOUNCEMENTS_CHANNEL_ID="*|"COTERIE__INTEGRATIONS__DISCORD__ANNOUNCEMENTS_CHANNEL_ID="*)
                if [[ "$ENABLE_DISCORD" == "true" ]]; then
                    printf '%s\n' "COTERIE__INTEGRATIONS__DISCORD__ANNOUNCEMENTS_CHANNEL_ID=${DISCORD_ANNOUNCE_CHANNEL}"
                else
                    printf '%s\n' "$line"
                fi
                ;;
            "# COTERIE__INTEGRATIONS__UNIFI__ENABLED="*|"COTERIE__INTEGRATIONS__UNIFI__ENABLED="*)
                printf '%s\n' "COTERIE__INTEGRATIONS__UNIFI__ENABLED=${ENABLE_UNIFI}"
                ;;
            "# COTERIE__INTEGRATIONS__UNIFI__CONTROLLER_URL="*|"COTERIE__INTEGRATIONS__UNIFI__CONTROLLER_URL="*)
                if [[ "$ENABLE_UNIFI" == "true" ]]; then
                    printf '%s\n' "COTERIE__INTEGRATIONS__UNIFI__CONTROLLER_URL=${UNIFI_URL}"
                else
                    printf '%s\n' "$line"
                fi
                ;;
            "# COTERIE__INTEGRATIONS__UNIFI__USERNAME="*|"COTERIE__INTEGRATIONS__UNIFI__USERNAME="*)
                if [[ "$ENABLE_UNIFI" == "true" ]]; then
                    printf '%s\n' "COTERIE__INTEGRATIONS__UNIFI__USERNAME=${UNIFI_USERNAME}"
                else
                    printf '%s\n' "$line"
                fi
                ;;
            "# COTERIE__INTEGRATIONS__UNIFI__PASSWORD="*|"COTERIE__INTEGRATIONS__UNIFI__PASSWORD="*)
                if [[ "$ENABLE_UNIFI" == "true" ]]; then
                    printf '%s\n' "COTERIE__INTEGRATIONS__UNIFI__PASSWORD=${UNIFI_PASSWORD}"
                else
                    printf '%s\n' "$line"
                fi
                ;;
            *)
                printf '%s\n' "$line"
                ;;
        esac
    done < "$template" > "$tmp"

    # Append a provisioning record so operators can see what wrote this file.
    {
        printf '\n# ---------------------------------------------------------------------\n'
        printf '# Provisioning record (informational — safe to remove)\n'
        printf '# ---------------------------------------------------------------------\n'
        printf '# Provisioned: %s\n' "$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
        printf '# Org: %s\n' "$ORG_NAME"
        printf '# Contact: %s\n' "$CONTACT_EMAIL"
    } >> "$tmp"

    mv "$tmp" "$target"
    chown coterie:coterie "$target"
    chmod 0640 "$target"
}

# a25: if the operator pre-loaded live credentials in test mode, stash
# them in /opt/coterie/.env.live for switch-stripe-to-live.sh to source.
# Same permissions as .env (0640, coterie:coterie).
write_env_live() {
    local target="$INSTALL_DIR/.env.live"
    local tmp
    tmp="$(mktemp)"
    {
        printf '# Coterie live-mode Stripe credentials, pre-loaded by provision.sh.\n'
        printf '# Read by deploy/switch-stripe-to-live.sh at switchover time.\n'
        printf '# Safe to delete if you would rather provide live creds interactively.\n'
        printf '\n'
        printf 'COTERIE__STRIPE__PUBLISHABLE_KEY=%s\n' "$STRIPE_LIVE_PK"
        printf 'COTERIE__STRIPE__SECRET_KEY=%s\n' "$STRIPE_LIVE_SK"
        printf 'COTERIE__STRIPE__WEBHOOK_SECRET=%s\n' "$STRIPE_LIVE_WHSEC"
    } > "$tmp"
    mv "$tmp" "$target"
    chown coterie:coterie "$target"
    chmod 0640 "$target"
}

if [[ "$EXISTING_ENV" == "true" ]]; then
    overwrite_env=false
    if [[ "${COTERIE_PROVISION_OVERWRITE_ENV:-}" == "true" ]]; then
        overwrite_env=true
        info "Existing .env — overwriting (COTERIE_PROVISION_OVERWRITE_ENV=true)."
    elif [[ -t 0 ]]; then
        # shellcheck disable=SC2162
        read -r -p "$INSTALL_DIR/.env already exists. Overwrite? [y/N]: " ans
        case "$ans" in y|Y|yes|YES) overwrite_env=true ;; esac
    else
        warn "Existing .env present, non-interactive shell, and \$COTERIE_PROVISION_OVERWRITE_ENV not set — keeping existing .env."
    fi
    if [[ "$overwrite_env" == "true" ]]; then
        generate_env
    fi
else
    generate_env
fi

# a25: stash pre-loaded live credentials for switch-stripe-to-live.sh.
if [[ "$ENABLE_STRIPE" == "true" && "$STRIPE_MODE" == "test" && "$PRELOAD_LIVE" == "true" ]]; then
    info "Writing pre-loaded live credentials to $INSTALL_DIR/.env.live (chmod 0640)..."
    write_env_live
fi

# ---------------------------------------------------------------------
# 7. Bootstrap the first admin
# ---------------------------------------------------------------------

echo ""
info "=== Creating first admin ==="

if [[ "$EXISTING_ADMIN" == "true" ]]; then
    info "Admin already exists — skipping create_admin."
else
    # 7.1: write password to a chmod-600 tempfile
    pwfile="$(mktemp /tmp/coterie-bootstrap.XXXXXX.pw)"
    chmod 600 "$pwfile"
    # 7.3: guarantee shred-on-exit regardless of how we leave the block
    cleanup_pwfile() {
        if [[ -f "$pwfile" ]]; then
            shred -u "$pwfile" 2>/dev/null || rm -f "$pwfile"
        fi
    }
    trap 'cleanup_pwfile' EXIT
    printf '%s' "$ADMIN_PASSWORD" > "$pwfile"
    chown coterie:coterie "$pwfile"

    # 7.2: invoke create_admin. Run as the coterie user so the SQLite
    # file ends up owned by coterie, not root.
    set +e
    sudo -u coterie -H -E \
        "$INSTALL_DIR/create_admin" \
            --email      "$ADMIN_EMAIL" \
            --username   "$ADMIN_USERNAME" \
            --full-name  "$ADMIN_FULL_NAME" \
            --password-file "$pwfile"
    ca_exit=$?
    set -e

    cleanup_pwfile
    trap - EXIT

    # 7.4: exit 2 → admin already exists (race or re-run), keep going
    # 7.5: other non-zero → die
    case $ca_exit in
        0) info "Admin created." ;;
        2) warn "create_admin reports admin already exists — continuing." ;;
        *) die "create_admin failed with exit code $ca_exit" ;;
    esac
fi

# ---------------------------------------------------------------------
# 8. Configure Caddy
# ---------------------------------------------------------------------

if [[ "$ENABLE_CADDY" == "true" ]]; then
    echo ""
    info "=== Configuring Caddy ==="

    caddy_template="$INSTALL_DIR/deploy/Caddyfile.example"
    if [[ ! -f "$caddy_template" ]]; then
        die "$caddy_template missing — release-deploy.sh should have placed it"
    fi

    write_caddyfile=true
    if [[ "$EXISTING_CADDYFILE" == "true" ]]; then
        if [[ -t 0 ]]; then
            # shellcheck disable=SC2162
            read -r -p "$CADDYFILE_DST already exists. Overwrite? [y/N]: " ans
            case "$ans" in y|Y|yes|YES) ;; *) write_caddyfile=false ;; esac
        else
            warn "Existing $CADDYFILE_DST and non-interactive shell — keeping existing Caddyfile."
            write_caddyfile=false
        fi
    fi

    if [[ "$write_caddyfile" == "true" ]]; then
        caddy_tmp="$(mktemp)"

        # 8.2: portal domain substitution.
        # 8.3: marketing block — substitute or remove.
        python3 - "$caddy_template" "$PORTAL_DOMAIN" "$MARKETING_DOMAIN" > "$caddy_tmp" <<'PYEOF'
import sys, re

template_path, portal, marketing = sys.argv[1], sys.argv[2], sys.argv[3]
with open(template_path) as f:
    src = f.read()

# Portal domain substitution.
src = src.replace("coterie.example.com", portal)

if marketing:
    src = src.replace("example.com, www.example.com",
                      f"{marketing}, www.{marketing}")
    src = src.replace("/var/www/example.com", f"/var/www/{marketing}")
else:
    # Remove the second site block. We find the marker line and drop
    # everything from the preceding section header onwards.
    marker = "# Public marketing / signup site (optional)"
    idx = src.find(marker)
    if idx != -1:
        # Walk back to the preceding section divider so we also drop
        # the "# ---" line and the blank line before the section.
        prefix = src[:idx]
        last_divider = prefix.rfind("# --")
        if last_divider != -1:
            src = src[:last_divider].rstrip() + "\n"

sys.stdout.write(src)
PYEOF

        # 8.4: log dir BEFORE writing the Caddyfile (the real bug we hit manually).
        info "Creating /var/log/caddy and fixing ownership (the bug we hit manually)..."
        mkdir -p /var/log/caddy
        chown -R caddy:caddy /var/log/caddy

        # 8.5: write the Caddyfile
        mkdir -p "$(dirname "$CADDYFILE_DST")"
        mv "$caddy_tmp" "$CADDYFILE_DST"
        chown root:root "$CADDYFILE_DST"
        chmod 0644 "$CADDYFILE_DST"

        # 8.6: validate
        info "Validating Caddyfile..."
        if ! caddy validate --config "$CADDYFILE_DST"; then
            die "Caddyfile validation failed. Edit $CADDYFILE_DST manually and re-run, or run with --dry-run to inspect the planned config."
        fi

        # 8.7: reload (or start if not yet running)
        if systemctl is-active --quiet caddy; then
            info "Reloading Caddy..."
            systemctl reload caddy
        else
            info "Starting Caddy..."
            systemctl enable --now caddy
        fi

        # 8.8: confirm active
        if ! systemctl is-active --quiet caddy; then
            warn "Caddy is not active. Recent logs:"
            journalctl -u caddy -n 30 --no-pager || true
            die "Caddy failed to start. Inspect 'journalctl -u caddy' and fix the Caddyfile."
        fi
        info "Caddy is active."
    fi
fi

# ---------------------------------------------------------------------
# 9. Start Coterie
# ---------------------------------------------------------------------

echo ""
info "=== Starting Coterie ==="

systemctl enable --now coterie

# 9.2/9.3: wait up to 30s for active
waited=0
while (( waited < 30 )); do
    if systemctl is-active --quiet coterie; then
        break
    fi
    sleep 1
    waited=$((waited + 1))
done

if ! systemctl is-active --quiet coterie; then
    fail "Coterie did not become active within 30 seconds. Recent journal:"
    journalctl -u coterie -n 50 --no-pager || true
    die "service start failed"
fi
info "Coterie is active (after ${waited}s)."

# ---------------------------------------------------------------------
# 10. Smoke test
# ---------------------------------------------------------------------

echo ""
info "=== Smoke test ==="

dump_diagnostics() {
    echo ""
    echo "--- systemctl status coterie ---"
    systemctl status coterie --no-pager || true
    echo "--- journalctl -u coterie -n 50 ---"
    journalctl -u coterie -n 50 --no-pager || true
    if [[ "$ENABLE_CADDY" == "true" ]]; then
        echo "--- journalctl -u caddy -n 30 ---"
        journalctl -u caddy -n 30 --no-pager || true
    fi
}

# 10.1: direct HTTP to the app.
# We curl with -i so we can inspect status + body. A 200 with JSON is
# the pass case; a 303 to /setup means the admin row isn't visible to
# the server (would indicate ordering bug).
health_resp="$(mktemp)"
if ! curl -fsS -i http://127.0.0.1:8080/health -o "$health_resp"; then
    fail "curl http://127.0.0.1:8080/health failed."
    cat "$health_resp" || true
    rm -f "$health_resp"
    dump_diagnostics
    die "smoke test failed"
fi

first_line="$(head -n1 "$health_resp")"
if [[ "$first_line" != *"200"* ]]; then
    fail "health endpoint did not return 200. Response head:"
    head -n5 "$health_resp" >&2
    rm -f "$health_resp"
    dump_diagnostics
    die "smoke test failed (unexpected status)"
fi

if grep -qi "Location:.*/setup" "$health_resp"; then
    fail "health endpoint redirected to /setup — the admin row isn't visible."
    rm -f "$health_resp"
    dump_diagnostics
    die "smoke test failed (admin row not loaded)"
fi
rm -f "$health_resp"
info "Direct health check OK."

# 10.2: Caddy chain — -k because the SNI for 127.0.0.1 won't match the cert.
if [[ "$ENABLE_CADDY" == "true" ]]; then
    if curl -fsSk https://127.0.0.1/health > /dev/null 2>&1; then
        info "Caddy proxy chain OK."
    else
        warn "Caddy proxy chain didn't respond to https://127.0.0.1/health."
        warn "This often just means DNS isn't pointing at this box yet — Caddy hasn't issued a cert for ${PORTAL_DOMAIN}."
        warn "Once you point DNS, the first inbound HTTPS request triggers cert issuance."
    fi
fi

# ---------------------------------------------------------------------
# 10.5 Stripe test-mode verification checklist (a25)
# ---------------------------------------------------------------------

# When the operator chose test mode, print the verification checklist
# (per design.md D8) immediately before the final summary. In live mode
# the checklist is suppressed to match the a24 baseline output exactly.
if [[ "$ENABLE_STRIPE" == "true" && "$STRIPE_MODE" == "test" ]]; then
    cat <<EOF

============================================================
Stripe TEST MODE — verification checklist
============================================================
Coterie is running in Stripe TEST mode with a separate test database
($DATA_DIR/coterie-test.db). Use this time to verify Stripe wiring
before switching to live.

Test card to use: 4242 4242 4242 4242, any future expiry, any 3-digit
CVC, any ZIP.

Suggested verification steps:

  [ ] Sign up a test member via your public site or directly via
      Coterie's signup form (if exposed).
  [ ] Make a test donation through /portal/donate (logged in as
      admin) or via the public donate flow.
  [ ] Confirm each test charge appears in your Stripe dashboard's
      TEST MODE payments view.
  [ ] Confirm \`journalctl -u coterie\` shows the webhook events
      arriving cleanly (look for "Webhook event received").
  [ ] Confirm the receipt email arrived at the address you used.

When satisfied, switch to live mode:

  sudo bash $INSTALL_DIR/deploy/switch-stripe-to-live.sh

This will: stop Coterie, archive coterie-test.db, create a fresh
coterie.db, copy your admin row across, prompt for (or load) your
live Stripe credentials, rewrite .env, and start Coterie back up.
============================================================
EOF
fi

# ---------------------------------------------------------------------
# 11. Final summary
# ---------------------------------------------------------------------

echo ""
echo "============================================================"
echo "Coterie installation complete."
echo ""
echo "  Org name:         $ORG_NAME"
echo "  Portal URL:       https://${PORTAL_DOMAIN}"
echo "  Admin email:      ${ADMIN_EMAIL:-<existing admin retained>}"
echo "  Service status:   $(systemctl is-active coterie)"
echo ""
echo "Next steps:"
echo "  1. Point DNS for ${PORTAL_DOMAIN} at this box's public IP."
if [[ "$ENABLE_CADDY" == "true" ]]; then
    echo "     Caddy will auto-provision a TLS cert on the first inbound"
    echo "     HTTPS request (usually under 30 seconds)."
fi
if [[ "$ENABLE_STRIPE" == "true" ]]; then
    echo ""
    echo "  2. Register a Stripe webhook:"
    echo "     URL:    https://${PORTAL_DOMAIN}/api/payments/webhook/stripe"
    echo "     Events: see deploy/STRIPE-SETUP.md"
fi
echo ""
if [[ "$ENABLE_STRIPE" == "true" ]]; then
    echo "  3. Log in: visit https://${PORTAL_DOMAIN}/login"
else
    echo "  2. Log in: visit https://${PORTAL_DOMAIN}/login"
fi
echo "     Username: ${ADMIN_USERNAME:-<from existing admin>}"
echo ""
echo "Recovery if needed:"
echo "  bash ${INSTALL_DIR}/deploy/uninstall.sh         # keep data + .env"
echo "  bash ${INSTALL_DIR}/deploy/uninstall.sh --all   # nuke everything"
echo "============================================================"
