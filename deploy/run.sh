#!/bin/sh
# Coterie launcher — sources .env then exec's the binary.
#
# Used by deploy/coterie.openrc on Alpine because OpenRC has no direct
# EnvironmentFile= equivalent. Also fine to invoke manually for ad-hoc
# debugging, e.g.:
#   sudo -u coterie /opt/coterie/run.sh

set -eu

ENV_FILE=${COTERIE_ENV_FILE:-/opt/coterie/.env}
BIN=${COTERIE_BIN:-/opt/coterie/coterie}

if [ -f "$ENV_FILE" ]; then
    # `set -a` exports every variable that's assigned. `. "$ENV_FILE"`
    # sources the file in the current shell so the assignments stick.
    # `set +a` restores normal behavior — exec inherits the exports.
    set -a
    # shellcheck disable=SC1090
    . "$ENV_FILE"
    set +a
fi

exec "$BIN"
