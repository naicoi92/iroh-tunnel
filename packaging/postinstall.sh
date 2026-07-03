#!/bin/sh
# postinstall script for the iroh-tunnel .deb package (ADR-0002).
#
# Runs after dpkg unpacks the files. We keep it minimal and idempotent so
# `apt upgrade` does the right thing on a reinstall:
#   1. make sure the config directory exists (the package ships a /etc/iroh-tunnel
#      dir, but belt-and-suspenders in case an admin removed it);
#   2. install the `it` convenience alias -> iroh-tunnel;
#   3. reload systemd so the installed unit is picked up.
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

# Install the `it` convenience alias as a symlink to the iroh-tunnel binary,
# so users can type `it serve run` instead of `iroh-tunnel serve run`.
#
# nFPM's default bindir is /usr/local/bin on recent versions, but older
# releases placed binaries in /usr/bin; check both so the symlink always sits
# next to the real binary. The symlink target is the basename only (not an
# absolute path) so it survives a package relocation or bindir change.
for BIN in /usr/local/bin/iroh-tunnel /usr/bin/iroh-tunnel; do
    [ -f "$BIN" ] || continue
    ALIAS="$(dirname "$BIN")/it"
    # Don't clobber an existing `it` the user may already have on PATH.
    # `-e` alone misses dangling symlinks (e.g. left over from a botched
    # uninstall); `-L` covers those too so we skip the `ln -s` and don't trip
    # "File exists" under `set -e`.
    if [ ! -e "$ALIAS" ] && [ ! -L "$ALIAS" ]; then
        ln -s iroh-tunnel "$ALIAS"
    fi
    break
done

# Reload systemd if it is available (no-op in containers / non-systemd hosts).
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload || true
fi

exit 0
