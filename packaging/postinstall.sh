#!/bin/sh
# postinstall script for the iroh-tunnel .deb package (ADR-0002).
#
# Runs after dpkg unpacks the files. We keep it minimal and idempotent so
# `apt upgrade` does the right thing on a reinstall:
#   1. make sure the config directory exists (the package ships a /etc/iroh-tunnel
#      dir, but belt-and-suspenders in case an admin removed it);
#   2. reload systemd so the installed unit is picked up.
#
# We deliberately do NOT enable/start the service here: enabling on install can
# surprise users (autostart of a tunnel that has not been configured yet), and
# Debian policy discourages starting daemons from postinst without debconf.
# Users enable it with:  systemctl enable --now iroh-tunnel.service

set -e

CONFIG_DIR="/etc/iroh-tunnel"

if [ ! -d "$CONFIG_DIR" ]; then
    mkdir -p "$CONFIG_DIR"
    chmod 0755 "$CONFIG_DIR"
fi

# Reload systemd if it is available (no-op in containers / non-systemd hosts).
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload || true
fi

exit 0
