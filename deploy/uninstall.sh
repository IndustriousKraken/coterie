#!/bin/sh
# Remove a Coterie install. Destructive — read carefully before running.
#
# Usage:
#   uninstall.sh                  # remove app files + systemd unit
#                                 # KEEP coterie user, /var/lib/coterie (data),
#                                 # and /opt/coterie/.env
#   uninstall.sh --data           # also wipe /var/lib/coterie (DB + uploads)
#   uninstall.sh --all            # full scorched earth: remove user + .env too
#   uninstall.sh --yes            # skip the y/N confirmation prompt
#
# Combine flags as needed:
#   uninstall.sh --all --yes      # automated full removal, no prompt
#
# Safe defaults:
#   - .env stays (it has secrets you don't want to regenerate)
#   - /var/lib/coterie stays (it has your DB and uploads)
#   - coterie system user stays (so re-install reuses it)
#
# After running with default flags, you can re-run release-deploy.sh and
# end up with the same data + .env, just with fresh app files.

set -eu

INSTALL_DIR="/opt/coterie"
DATA_DIR="/var/lib/coterie"
SERVICE_UNIT="/etc/systemd/system/coterie.service"
BACKUP_SERVICE="/etc/systemd/system/coterie-backup.service"
BACKUP_TIMER="/etc/systemd/system/coterie-backup.timer"
OPENRC_UNIT="/etc/init.d/coterie"
USER_NAME="coterie"

REMOVE_DATA=false
REMOVE_ALL=false
ASSUME_YES=false

while [ $# -gt 0 ]; do
    case "$1" in
        --data) REMOVE_DATA=true ;;
        --all)  REMOVE_DATA=true; REMOVE_ALL=true ;;
        --yes|-y) ASSUME_YES=true ;;
        -h|--help)
            sed -n '/^# /,/^$/p' "$0" | sed 's/^# //; s/^#$//'
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Run with --help for usage."
            exit 1
            ;;
    esac
    shift
done

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: must run as root (sudo bash $0)"
    exit 1
fi

# --- Inventory what's present ----------------------------------------
echo "Coterie uninstall — scanning current state:"
echo ""

scan_item() {
    label="$1"
    path="$2"
    if [ -e "$path" ]; then
        echo "  [present] $label ($path)"
    else
        echo "  [absent]  $label ($path)"
    fi
}

scan_item "App directory"        "$INSTALL_DIR"
scan_item ".env file"            "$INSTALL_DIR/.env"
scan_item "Data directory"       "$DATA_DIR"
scan_item "systemd unit"         "$SERVICE_UNIT"
scan_item "Backup service unit"  "$BACKUP_SERVICE"
scan_item "Backup timer"         "$BACKUP_TIMER"
scan_item "OpenRC init script"   "$OPENRC_UNIT"
if id "$USER_NAME" >/dev/null 2>&1; then
    echo "  [present] System user ($USER_NAME)"
else
    echo "  [absent]  System user ($USER_NAME)"
fi

echo ""
echo "Plan:"
echo "  - Stop and disable the coterie service (if running)"
echo "  - Remove $INSTALL_DIR (all app files)"
if [ "$REMOVE_DATA" = "false" ]; then
    echo "  - KEEP .env"
    echo "  - KEEP $DATA_DIR (your DB + uploads + backups)"
else
    echo "  - REMOVE .env"
    echo "  - REMOVE $DATA_DIR (your DB + uploads + backups will be DELETED)"
fi
echo "  - Remove systemd unit ($SERVICE_UNIT)"
echo "  - Remove backup service + timer (if present)"
echo "  - Remove OpenRC init script (if present)"
if [ "$REMOVE_ALL" = "false" ]; then
    echo "  - KEEP coterie system user"
else
    echo "  - REMOVE coterie system user"
fi
echo ""

# --- Confirmation ----------------------------------------------------
if [ "$ASSUME_YES" != "true" ]; then
    if [ "$REMOVE_DATA" = "true" ]; then
        echo "WARNING: --data was specified. /var/lib/coterie WILL BE DELETED."
        echo "  This includes the SQLite database, member uploads, and all backups."
        echo ""
    fi
    printf "Proceed? [y/N] "
    read -r REPLY
    case "$REPLY" in
        y|Y|yes|YES) ;;
        *) echo "Aborted."; exit 0 ;;
    esac
fi

# --- Stop the service ------------------------------------------------
if command -v systemctl >/dev/null 2>&1; then
    if systemctl list-unit-files coterie.service >/dev/null 2>&1; then
        systemctl stop coterie.service 2>/dev/null || true
        systemctl disable coterie.service 2>/dev/null || true
    fi
    if systemctl list-unit-files coterie-backup.timer >/dev/null 2>&1; then
        systemctl stop coterie-backup.timer 2>/dev/null || true
        systemctl disable coterie-backup.timer 2>/dev/null || true
    fi
fi

if command -v rc-service >/dev/null 2>&1; then
    rc-service coterie stop 2>/dev/null || true
    rc-update del coterie default 2>/dev/null || true
fi

# --- Remove service units --------------------------------------------
rm -f "$SERVICE_UNIT" "$BACKUP_SERVICE" "$BACKUP_TIMER" "$OPENRC_UNIT"

if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload
fi

# --- Remove app files ------------------------------------------------
# In default mode, save .env aside; everything else in $INSTALL_DIR goes.
if [ "$REMOVE_DATA" = "false" ] && [ -f "$INSTALL_DIR/.env" ]; then
    cp "$INSTALL_DIR/.env" "/tmp/coterie.env.saved.$$"
fi

rm -rf "$INSTALL_DIR"

# Restore .env if we saved it.
if [ "$REMOVE_DATA" = "false" ] && [ -f "/tmp/coterie.env.saved.$$" ]; then
    mkdir -p "$INSTALL_DIR"
    mv "/tmp/coterie.env.saved.$$" "$INSTALL_DIR/.env"
    if id "$USER_NAME" >/dev/null 2>&1; then
        chown "$USER_NAME:$USER_NAME" "$INSTALL_DIR/.env"
    fi
    chmod 0640 "$INSTALL_DIR/.env"
    echo ""
    echo "Preserved $INSTALL_DIR/.env"
fi

# --- Remove data dir (if asked) --------------------------------------
if [ "$REMOVE_DATA" = "true" ] && [ -d "$DATA_DIR" ]; then
    rm -rf "$DATA_DIR"
fi

# --- Remove user (if asked) ------------------------------------------
if [ "$REMOVE_ALL" = "true" ] && id "$USER_NAME" >/dev/null 2>&1; then
    if command -v userdel >/dev/null 2>&1; then
        userdel "$USER_NAME" 2>/dev/null || true
    elif command -v deluser >/dev/null 2>&1; then
        deluser "$USER_NAME" 2>/dev/null || true
    fi
fi

# --- Done ------------------------------------------------------------
echo ""
echo "Uninstall complete."
if [ "$REMOVE_DATA" = "false" ]; then
    echo ""
    echo "Preserved for re-install:"
    [ -f "$INSTALL_DIR/.env" ] && echo "  $INSTALL_DIR/.env"
    [ -d "$DATA_DIR" ] && echo "  $DATA_DIR (DB + uploads)"
    if id "$USER_NAME" >/dev/null 2>&1; then
        echo "  System user '$USER_NAME'"
    fi
    echo ""
    echo "Re-install:  bash release-deploy.sh"
fi
