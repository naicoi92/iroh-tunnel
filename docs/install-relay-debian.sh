#!/usr/bin/env bash
# install-relay-debian.sh — deploy a self-hosted iroh-relay on a Debian 12 LXC (public IP).
#
# Implements docs/self-hosted-relay.md §5: installs the iroh-relay binary from
# GitHub Releases (queries the API for the asset and verifies its sha256 digest),
# writes the TOML config (loopback binds, metrics, access.shared_token), installs
# a systemd service + hang-guard timer, and sets up Caddy (official apt repo,
# ACME TLS) reverse-proxying to 127.0.0.1:3340.
#
# Idempotent: safe to re-run — the access token is reused from the existing config.
#
# Usage:
#   ./install-relay-debian.sh [--domain relay.example.com] [--token <token>]
#       [--version v1.0.3 | --latest] [--acme-email <email>] [--apply-firewall]
#
# Options:
#   --domain <d>       Public hostname of the relay (A record -> LXC public IP).
#                      If omitted the script prompts (requires a TTY).
#   --token <t>        Access token (shared_token). If omitted and no existing
#                      config: prompt (Enter = generate via openssl rand -hex 32).
#   --version <v>      Pin a release version (default: v1.0.3).
#   --latest           Use the latest release instead of the pin.
#   --acme-email <e>   Email for the ACME account (optional).
#   --apply-firewall   Apply ufw rules: allow 80,443/tcp + SSH port, default deny.
#                      Without it the script only prints instructions (§5.7).
#   -h, --help         Show this help.
#
# Env:
#   IROH_RELAY_VERSION   Same as --version (flag wins).
set -euo pipefail

VERSION="${IROH_RELAY_VERSION:-v1.0.3}"
LATEST=0
DOMAIN=""
TOKEN=""
ACME_EMAIL=""
APPLY_FIREWALL=0

CONFIG_DIR=/etc/iroh-relay
CONFIG_FILE=$CONFIG_DIR/config.toml
STATE_DIR=/var/lib/iroh-relay
BIN_PATH=/usr/local/bin/iroh-relay
RELAY_PORT=3340
METRICS_PORT=9090

usage() { sed -n '2,29p' "$0" | sed 's/^# \{0,1\}//'; }

die() { echo "ERROR: $*" >&2; exit 1; }
log() { echo "==> $*"; }

# ---------------------------------------------------------------------------
# Args
# ---------------------------------------------------------------------------
while [ $# -gt 0 ]; do
  case "$1" in --domain | --token | --version | --acme-email) [ $# -ge 2 ] || die "$1 requires a value" ;; esac
  case "$1" in
    --domain) DOMAIN="${2:-}"; shift 2 ;;
    --token) TOKEN="${2:-}"; shift 2 ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    --latest) LATEST=1; shift ;;
    --acme-email) ACME_EMAIL="${2:-}"; shift 2 ;;
    --apply-firewall) APPLY_FIREWALL=1; shift ;;
    -h | --help) usage; exit 0 ;;
    *) die "unknown option: $1 (see --help)" ;;
  esac
done

# ---------------------------------------------------------------------------
# Preflight
# ---------------------------------------------------------------------------
[ "$(id -u)" -eq 0 ] || die "must run as root (try: sudo ./install-relay-debian.sh)"

. /etc/os-release
[ "${ID:-}" = "debian" ] || die "this script targets Debian (found: ${ID:-unknown}); see decision D4."
[ "${VERSION_ID:-}" = "12" ] || echo "WARN: tested on Debian 12, this is ${VERSION_ID:-?} — continuing." >&2

ARCH=$(uname -m)
case "$ARCH" in
  x86_64) TARGET=x86_64-unknown-linux-gnu ;;
  aarch64) TARGET=aarch64-unknown-linux-gnu ;;
  *) die "unsupported arch: $ARCH" ;;
esac

log "Debian ${VERSION_ID:-?} / $TARGET"

# ---------------------------------------------------------------------------
# Interactive prompts (only when input is missing and a TTY is available)
# ---------------------------------------------------------------------------
if [ -z "$DOMAIN" ]; then
  [ -t 0 ] || die "missing --domain (no TTY available to prompt)"
  while [ -z "$DOMAIN" ]; do
    read -r -p "Relay domain (e.g. relay.example.com): " DOMAIN
  done
fi

# Token: flag > existing config > prompt / generate
if [ -z "$TOKEN" ] && [ -f "$CONFIG_FILE" ]; then
  TOKEN=$(sed -n 's/^access\.shared_token = \["\([^"]*\)"\]$/\1/p' "$CONFIG_FILE" || true)
  [ -n "$TOKEN" ] && log "reusing token from existing config"
fi
if [ -z "$TOKEN" ]; then
  if [ -t 0 ]; then
    read -r -p "Access token (Enter = generate): " TOKEN
  fi
  if [ -z "$TOKEN" ]; then
    if command -v openssl >/dev/null 2>&1; then
      TOKEN=$(openssl rand -hex 32)
    else
      TOKEN=$(od -An -tx1 -N32 /dev/urandom | tr -d ' \n')
    fi
    log "generated a new token (openssl rand -hex 32)"
  fi
fi
# TOML string safety: the token lands inside double quotes in the config
case "$TOKEN" in
  *'"'*) die "token contains \" — cannot be written to TOML safely" ;;
esac
[ -n "$TOKEN" ] || die "empty token (the server refuses to start)"

# ---------------------------------------------------------------------------
# Deps
# ---------------------------------------------------------------------------
log "installing deps (curl, ca-certificates, jq, tar)"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq curl ca-certificates jq tar >/dev/null

# ---------------------------------------------------------------------------
# Binary: query the GitHub API -> download -> verify digest -> install
# ---------------------------------------------------------------------------
if [ "$LATEST" -eq 1 ]; then
  API_URL="https://api.github.com/repos/n0-computer/iroh/releases/latest"
else
  API_URL="https://api.github.com/repos/n0-computer/iroh/releases/tags/$VERSION"
fi
log "querying release: $API_URL"
REL_JSON=$(curl -fsSL "$API_URL") || die "could not fetch release info (check version/network)"
VERSION=$(jq -r '.tag_name' <<<"$REL_JSON")

ASSET_NAME="iroh-relay-${VERSION}-${TARGET}.tar.gz"
ASSET_URL=$(jq -r --arg n "$ASSET_NAME" '.assets[] | select(.name == $n) | .browser_download_url' <<<"$REL_JSON")
ASSET_DIGEST=$(jq -r --arg n "$ASSET_NAME" '.assets[] | select(.name == $n) | .digest // empty' <<<"$REL_JSON")
[ -n "$ASSET_URL" ] || die "asset $ASSET_NAME not found in release $VERSION"

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

log "downloading $ASSET_NAME"
curl -fsSL -o "$TMP_DIR/$ASSET_NAME" "$ASSET_URL"

if [ -n "$ASSET_DIGEST" ]; then
  log "verifying sha256 digest (${ASSET_DIGEST%%:*})"
  echo "${ASSET_DIGEST#sha256:}  $TMP_DIR/$ASSET_NAME" | sha256sum -c - || die "digest mismatch — aborting install"
else
  echo "WARN: asset has no digest in the API — skipping checksum verification." >&2
fi

tar -xzf "$TMP_DIR/$ASSET_NAME" -C "$TMP_DIR"
BIN_FOUND=$(find "$TMP_DIR" -type f -name iroh-relay | head -n 1)
[ -n "$BIN_FOUND" ] || die "iroh-relay binary not found inside the archive"
install -m 0755 "$BIN_FOUND" "$BIN_PATH"
log "installed $BIN_PATH ($($BIN_PATH --version 2>/dev/null || echo "$VERSION"))"

# ---------------------------------------------------------------------------
# Config (loopback binds — the binary's defaults are [::], NOT loopback)
# ---------------------------------------------------------------------------
log "writing $CONFIG_FILE"
mkdir -p "$CONFIG_DIR" "$STATE_DIR"
cat > "$CONFIG_FILE" <<EOF
# Managed by install-relay-debian.sh — see docs/self-hosted-relay.md §5.5
http_bind_addr = "127.0.0.1:${RELAY_PORT}"      # default is [::] — must set loopback
enable_metrics = true
metrics_bind_addr = "127.0.0.1:${METRICS_PORT}" # default is [::]:9090 — loopback, separate tunnel

access.shared_token = ["${TOKEN}"]
EOF
chmod 0600 "$CONFIG_FILE"

# ---------------------------------------------------------------------------
# systemd: service + hang-guard (§5.5)
# ---------------------------------------------------------------------------
log "installing systemd units"
cat > /etc/systemd/system/iroh-relay.service <<'EOF'
[Unit]
Description=iroh-relay (self-host)
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/usr/local/bin/iroh-relay --config-path=/etc/iroh-relay/config.toml --dev
Restart=always
RestartSec=3
StartLimitIntervalSec=0
CPUQuota=200%
MemoryMax=1G
AmbientCapabilities=
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/lib/iroh-relay

[Install]
WantedBy=multi-user.target
EOF

cat > /etc/systemd/system/iroh-relay-health.service <<'EOF'
[Unit]
Description=iroh-relay hang-guard (healthz check)

[Service]
Type=oneshot
ExecStart=/bin/bash -c 'curl -sf --max-time 5 http://127.0.0.1:3340/healthz || systemctl restart iroh-relay'
EOF

cat > /etc/systemd/system/iroh-relay-health.timer <<'EOF'
[Unit]
Description=iroh-relay hang-guard timer (30s)

[Timer]
OnCalendar=*:*:30

[Install]
WantedBy=timers.target
EOF

systemctl daemon-reload
systemctl enable --now iroh-relay.service iroh-relay-health.timer

log "waiting for /healthz (max 15s)"
HEALTH_OK=0
for _ in $(seq 1 15); do
  if curl -sf --max-time 2 "http://127.0.0.1:${RELAY_PORT}/healthz" >/dev/null 2>&1; then
    HEALTH_OK=1
    break
  fi
  sleep 1
done
[ "$HEALTH_OK" -eq 1 ] || { journalctl -u iroh-relay -n 30 --no-pager >&2 || true; die "iroh-relay did not come up on /healthz (logs above)"; }
log "relay is alive: 127.0.0.1:${RELAY_PORT}/healthz = 200"

# ---------------------------------------------------------------------------
# Caddy: TLS terminator (§5.6)
# ---------------------------------------------------------------------------
log "installing Caddy (official apt repo)"
apt-get install -y -qq debian-keyring debian-archive-keyring apt-transport-https gnupg >/dev/null
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg 2>/dev/null
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' > /etc/apt/sources.list.d/caddy-stable.list
apt-get update -qq
apt-get install -y -qq caddy >/dev/null

log "writing /etc/caddy/Caddyfile"
{
  [ -n "$ACME_EMAIL" ] && echo "email ${ACME_EMAIL}"
  cat <<EOF
${DOMAIN} {
    # automatic ACME TLS (HTTP-01 — needs :80+:443 reachable from the internet)
    reverse_proxy 127.0.0.1:${RELAY_PORT}
}
EOF
} > /etc/caddy/Caddyfile

systemctl reload caddy 2>/dev/null || systemctl restart caddy
systemctl enable caddy >/dev/null 2>&1 || true

# ---------------------------------------------------------------------------
# Firewall (§5.7): 80/443 + SSH; 3340/9090 stay closed (already loopback)
# ---------------------------------------------------------------------------
SSH_PORT=$(sed -n 's/^Port \([0-9][0-9]*\)$/\1/p' /etc/ssh/sshd_config 2>/dev/null | tail -n 1)
SSH_PORT=${SSH_PORT:-22}

if [ "$APPLY_FIREWALL" -eq 1 ]; then
  log "applying ufw (allow ${SSH_PORT},80,443/tcp; default deny incoming)"
  apt-get install -y -qq ufw >/dev/null
  ufw allow "${SSH_PORT}/tcp" comment 'ssh admin'
  ufw allow 80/tcp comment 'caddy http + acme'
  ufw allow 443/tcp comment 'caddy https'
  # udp/7824: only when QUIC addr discovery is enabled (decision D3 — currently OFF)
  ufw --force enable
else
  cat <<FIREWALL

==> Firewall (not applied — re-run with --apply-firewall, or configure manually):
    ufw allow ${SSH_PORT}/tcp && ufw allow 80/tcp && ufw allow 443/tcp && ufw --force enable
    (3340/9090 are bound to 127.0.0.1 — no rules needed. udp/7824 only if D3 is enabled.)
    If pve-firewall is enabled on the container: mirror the rules at the PVE layer.

FIREWALL
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
cat <<SUMMARY

=====================================================================
 iroh-relay deployed (version ${VERSION})

 URL          : https://${DOMAIN}   (DNS A record -> the LXC's public IP)
 Health local : curl http://127.0.0.1:${RELAY_PORT}/healthz
 Health public: curl -sI https://${DOMAIN}/healthz   (once DNS + ACME are done)
 Metrics      : curl http://127.0.0.1:${METRICS_PORT}/metrics

 Access token (keep secret — both client sides need it):
   ${TOKEN}

 Client wiring (docs/self-hosted-relay.md §"Wiring the clients"):
   - Rust    : RelayMode::Custom(url) + RelayMap::with_auth_token(token)
   - Browser : https://${DOMAIN}/relay?token=<token>   (the relay accepts ?token=)

 Verification checklist: docs/self-hosted-relay.md §"Verification checklist"
=====================================================================
SUMMARY
