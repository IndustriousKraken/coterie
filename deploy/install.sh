#!/usr/bin/env bash
# Coterie installer — idempotent first-time setup for a Linux host.
#
# What it does:
#   - Creates the `coterie` system user (no shell, no home)
#   - Creates /opt/coterie and /var/lib/coterie/{data,backups}
#   - Installs the systemd unit and reloads systemd
#   - Does NOT start the service yet (you still need to drop in .env)
#
# What it does NOT do:
#   - Build the binary. Either run `make release` and copy
#     `target/release/coterie` to /opt/coterie/, or use the Docker
#     image and skip this script entirely.
#   - Configure Caddy. See deploy/Caddyfile.example.
#   - Generate secrets. See post-install instructions printed at end.
#
# Usage:
#   sudo bash deploy/install.sh
#
# Re-running is safe — every step checks for existing state.

set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "ERROR: Must run as root (sudo bash deploy/install.sh)" >&2
    exit 1
fi

USER_NAME=coterie
GROUP_NAME=coterie
APP_DIR=/opt/coterie
DATA_DIR=/var/lib/coterie
SERVICE_FILE_SRC="$(dirname "$(readlink -f "$0")")/coterie.service"
SERVICE_FILE_DST=/etc/systemd/system/coterie.service

# --- 1. System user ---------------------------------------------------
if ! id -u "$USER_NAME" >/dev/null 2>&1; then
    echo "Creating system user: $USER_NAME"
    useradd --system \
        --no-create-home \
        --home-dir "$APP_DIR" \
        --shell /usr/sbin/nologin \
        --user-group \
        "$USER_NAME"
else
    echo "User $USER_NAME already exists — skipping"
fi

# --- 2. Directories ---------------------------------------------------
echo "Ensuring directories exist"
install -d -m 0755 -o "$USER_NAME" -g "$GROUP_NAME" "$APP_DIR"
install -d -m 0750 -o "$USER_NAME" -g "$GROUP_NAME" "$DATA_DIR"
install -d -m 0750 -o "$USER_NAME" -g "$GROUP_NAME" "$DATA_DIR/data"
install -d -m 0750 -o "$USER_NAME" -g "$GROUP_NAME" "$DATA_DIR/backups"

# Symlink so the systemd unit's working directory layout matches.
# /opt/coterie/data → /var/lib/coterie/data keeps the binary and its
# data on the same conceptual mount-point while still allowing data
# to live on a separate filesystem (block storage, EBS, etc.) if you
# bind-mount it.
if [[ ! -e "$APP_DIR/data" ]]; then
    ln -s "$DATA_DIR/data" "$APP_DIR/data"
    echo "Linked $APP_DIR/data -> $DATA_DIR/data"
fi

# --- 3. systemd unit --------------------------------------------------
if [[ ! -f "$SERVICE_FILE_SRC" ]]; then
    echo "ERROR: Cannot find $SERVICE_FILE_SRC" >&2
    exit 1
fi

if ! cmp -s "$SERVICE_FILE_SRC" "$SERVICE_FILE_DST"; then
    echo "Installing systemd unit"
    install -m 0644 "$SERVICE_FILE_SRC" "$SERVICE_FILE_DST"
    systemctl daemon-reload
else
    echo "systemd unit unchanged — skipping"
fi

# --- 4. Final guidance ------------------------------------------------
cat <<EOF

Coterie scaffolding is in place. Remaining steps:

  1. Drop the binary in $APP_DIR/coterie (chmod 0755, owned by $USER_NAME)

  2. Copy and edit .env:
       sudo cp .env.example $APP_DIR/.env
       sudo chown $USER_NAME:$GROUP_NAME $APP_DIR/.env
       sudo chmod 0640 $APP_DIR/.env
       sudo $EDITOR $APP_DIR/.env

     Generate the session secret with:
       openssl rand -hex 32

  3. Set the data directory in .env:
       COTERIE__SERVER__DATA_DIR=$DATA_DIR/data
       COTERIE__DATABASE__URL=sqlite://coterie.db

  4. Enable + start:
       sudo systemctl enable --now coterie
       sudo journalctl -u coterie -f

  5. (Optional) Install daily backups:
       sudo cp deploy/coterie-backup.service /etc/systemd/system/
       sudo cp deploy/coterie-backup.timer   /etc/systemd/system/
       sudo systemctl enable --now coterie-backup.timer

  6. (Optional) Reverse proxy with Caddy. See deploy/Caddyfile.example.

EOF
