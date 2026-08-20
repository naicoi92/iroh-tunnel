# Self-hosting an iroh relay (Debian 12 LXC, public IP + domain)

How to run your own `iroh-relay` natively — binary + systemd inside a Proxmox LXC
that has its own public IP (no NAT) — and terminate TLS with Caddy.

**Why:** every iroh endpoint uses n0's public relay farm by default. Self-hosting
removes that shared variable from your traffic path: you control the bandwidth,
the behavior, and can actually measure throughput instead of guessing. Two peers
still prefer direct (hole-punched) connections — the relay is signaling + fallback.

Fact sources: `iroh-relay/README.md` @ n0-computer/iroh, docs.rs `iroh_relay::server`,
and `iroh-relay/src/main.rs` — cross-checked against release **v1.0.3** (re-verified
2026-08-20: v1.0.3 is still latest; asset names and per-asset sha256 digests confirmed
via the GitHub Releases API; `*-unknown-linux-musl` variants exist too).

## Verified facts (no guessing)

| Fact | Value | Source |
|---|---|---|
| Prebuilt binary | `iroh-relay` in GitHub Releases n0-computer/iroh (v1.0.3) — `*-unknown-linux-gnu` (glibc) | releases page + API |
| Relay path + protocol | `/relay` = **WebSocket upgrade, binary frames**; server WS-pings every **15s** | docs.rs + protos/relay.rs |
| HTTP port | **3340** (`--dev` = plain HTTP, no TLS) | README |
| Health endpoint | **`/healthz`** (HTTP 200) | docs.rs |
| Network probes | `/ping`, `/generate_204` | docs.rs |
| QUIC addr discovery | port **7824/udp**, needs `enable_quic_addr_discovery` + its own TLS — separate from the data path (WS/3340) | README |
| Metrics | `enable_metrics = true`; separate HTTP server on **9090** `/metrics` (`metrics_bind_addr`) | main.rs |
| **Bind defaults** | `http_bind_addr` defaults to **`[::]`** (all interfaces — `--dev` only changes the port 80→3340, it does NOT bind loopback); `metrics_bind_addr` defaults to **`[::]:9090`**. Loopback must be set explicitly. | main.rs v1.0.3 |
| Config | TOML via `--config-path` (short `-c`); dev flag `--dev` | main.rs |
| Access control | `everyone` (default) / allowlist–denylist / `shared_token` (Bearer header or `?token=` query) / HTTP callout | README |
| Token env | `IROH_RELAY_ACCESS_TOKEN` (overrides config, single token) | README |
| Client token | Rust `RelayConfig::with_auth_token` / `RelayMap::with_auth_token` | docs.rs |

> The bind-default row is a real footgun on a public IP: without explicit loopback
> addresses in the config, the relay listens on every interface.

## Topology

```
Internet ──► LXC public IP (e.g. 203.0.113.10) — Debian 12, systemd
              ├─ :80/:443   Caddy (ACME TLS) ──► 127.0.0.1:3340
              ├─ :3340/tcp  iroh-relay --dev (bound to 127.0.0.1 — never exposed directly)
              ├─ :9090/tcp  metrics (bound to 127.0.0.1)
              └─ :7824/udp  QUIC addr discovery (optional D3 — open in firewall, no NAT)

[iroh clients]  relay URL = https://relay.<domain>
```

- **Caddy is the TLS terminator** (automatic ACME): clients see a standard
  `https://` URL. The relay itself runs `--dev` plain HTTP on loopback.
- DNS: `relay.<domain>` A record → the LXC's public IP.

## Decisions

| # | Decision | Choice | Why |
|---|---|---|---|
| D1 | Traffic gateway | **Caddy** (default) | zero-config ACME, streaming without buffering; swap later *with measurements*, not before |
| D2 | Access control | **`shared_token` mandatory** (public IP = internet-facing) | an open relay on the internet will be abused |
| D3 | QUIC addr discovery | Off initially | minimum viable step; enable later if needed |
| D4 | LXC OS | **Debian 12** | glibc binary compat + official Caddy apt repo |
| D5 | Metrics | On (9090, loopback) | measure real throughput instead of assuming |

## Deploy — automated via `install-relay-debian.sh` (in this directory)

The script asks for the DOMAIN (+ TOKEN, Enter = generate) and then: installs the
binary (queries the GitHub API for the asset, verifies its sha256 digest), writes
the token config, installs systemd + a hang-guard timer, installs Caddy from the
official apt repo + writes the Caddyfile, and prints firewall instructions
(`--apply-firewall` applies them via ufw). Sections below are the reference for
what the script does (for cross-checking / manual overrides).

```bash
# From a checkout of this repo — copy the script into the LXC and run as root:
scp docs/install-relay-debian.sh root@<LXC-IP>:/tmp/
ssh root@<LXC-IP> 'bash /tmp/install-relay-debian.sh --domain relay.<domain>'
# Extra flags: --acme-email <email> · --apply-firewall (ufw) · --version v1.0.3 | --latest · --token <t>
# Re-runs are idempotent — the token is reused from the existing config.
```

### 5.1 Create the LXC + install the binary

```bash
# PVE host — Debian 12 LXC with a direct public IP (adjust storage/template/static IP):
pct create 8120 local:vztmpl/debian-12-standard_12.7-1_amd64.tar.zst \
  --hostname iroh-relay --memory 1024 --cores 2 \
  --net0 name=eth0,bridge=vmbr0,ip=203.0.113.10/24,gw=203.0.113.1 \
  --unprivileged 1 --rootfs local-lvm:8
pct start 8120 && pct enter 8120

# Inside the LXC — binary from the release (the script resolves the asset + digest via the API):
apt-get update && apt-get install -y curl ca-certificates
curl -fsSL -o /tmp/r.tar.gz \
  "https://github.com/n0-computer/iroh/releases/download/v1.0.3/iroh-relay-v1.0.3-x86_64-unknown-linux-gnu.tar.gz"
tar -xzf /tmp/r.tar.gz -C /tmp && install -m 0755 /tmp/iroh-relay*/iroh-relay /usr/local/bin/
```

### 5.2 Debian or Alpine?

**Debian 12.** Release binaries are built for `*-unknown-linux-gnu` (glibc) and run
as-is. `*-unknown-linux-musl` variants do exist (re-verified 2026-08-20), but an LXC
is already lightweight — the footprint saving is not worth the compat risk. Debian
also gets Caddy's **official apt repo** (systemd unit + auto-restart included).

### 5.3 Choosing the traffic gateway (performance)

| Gateway | WS/binary relaying perf | TLS/ACME | Ops | Notes |
|---|---|---|---|---|
| No gateway (relay does its own TLS) | Max (0 hops) | **Manual** — `cert_mode Manual` + certbot renew hook | hands-on | skip if you want automated certs |
| **Caddy** ✅ default | Good — unbuffered streaming; small overhead inside the relay budget (2-CPU cap) | **Auto ACME** | lowest | change only if measurements say so |
| nginx (http) | Highest of the common ones | certbot + hooks | medium — easy to get wrong (`proxy_buffering off`, upgrade headers, timeouts) | when numbers demand it |
| HAProxy (tcp L4) | Very high | ACME out-of-band | medium; loses HTTP routing (metrics auth) | when numbers demand it |

**Principle:** decide **data-driven** — run Caddy + the 9090 metrics; only if
measurements show proxy CPU as the bottleneck, switch to nginx/HAProxy L4 or drop
the gateway. Don't optimize early without numbers.

### 5.4 UDP (QUIC address discovery) — when enabled

UDP 7824 is a **separate** service (endpoints learn their public address via QUIC),
independent from the data path (WS/3340). Caddy does not proxy UDP — expose it
directly:

- No NAT in the LXC → just **open `7824/udp` in the firewall** (pve-firewall or iptables inside the LXC).
- The relay needs its own TLS for QUIC: `enable_quic_addr_discovery = true` +
  `[tls] cert_mode = "Manual"` + rcgen/ACME certs (see the iroh-relay README,
  "dev mode with QUIC address discovery").
- Phase 1: **OFF** (hole-punching works regardless; discovery only improves address accuracy).

### 5.5 config.toml + systemd (auto-restart)

```toml
# /etc/iroh-relay/config.toml
http_bind_addr = "127.0.0.1:3340"    # IMPORTANT: default is [::] (all interfaces) — set loopback
enable_metrics = true
metrics_bind_addr = "127.0.0.1:9090" # default is [::]:9090 — set loopback; scrape via a separate tunnel

access.shared_token = ["<long-random-token>"]   # public IP → never "everyone"
# or via env: IROH_RELAY_ACCESS_TOKEN=<token> (overrides the config)
```

> Loopback binds are the **last** layer of defense (the firewall is the primary
> one). Without explicit `http_bind_addr`/`metrics_bind_addr` the relay listens
> on `[::]:3340` + `[::]:9090` — directly on the public IP.

```ini
# /etc/systemd/system/iroh-relay.service
[Unit]
Description=iroh-relay (self-host)
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/usr/local/bin/iroh-relay --config-path=/etc/iroh-relay/config.toml --dev
Restart=always
RestartSec=3
StartLimitIntervalSec=0        # never refuse restart due to rate-limiting
CPUQuota=200%
MemoryMax=1G
AmbientCapabilities=
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/lib/iroh-relay

[Install]
WantedBy=multi-user.target
```

```bash
mkdir -p /var/lib/iroh-relay && systemctl daemon-reload && systemctl enable --now iroh-relay
```

**Hang-guard (optional but recommended)** — catches a live-but-hung process that
`Restart=always` cannot cover:

```ini
# /etc/systemd/system/iroh-relay-health.service  (oneshot, driven by a 30s timer)
[Service]
Type=oneshot
ExecStart=/bin/bash -c 'curl -sf --max-time 5 http://127.0.0.1:3340/healthz || systemctl restart iroh-relay'
```
```ini
# /etc/systemd/system/iroh-relay-health.timer
[Timer]
OnCalendar=*:*:30
[Install]
WantedBy=timers.target
```
(Semantics note: do NOT use `ExecCondition=curl ...` + `ExecStart=restart` — the
logic inverts: when health is OK ExecCondition passes → endless restarts every
30s, and a hung process gets skipped. The right pattern: curl OK → exit 0 doing
nothing; curl fail → `||` triggers the restart.)

### 5.6 Caddy (TLS + domain)

```bash
# Official apt repo (docs.caddyserver.com — install on Debian):
apt install -y debian-keyring debian-archive-keyring apt-transport-https gnupg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' > /etc/apt/sources.list.d/caddy-stable.list
apt update && apt install caddy
```

```caddyfile
# /etc/caddy/Caddyfile
relay.<domain> {
    # automatic ACME TLS (needs :80+:443 reachable for HTTP-01)
    reverse_proxy 127.0.0.1:3340   # WS upgrade handled automatically; the 15s server pings keep it alive
    # (optional) route /metrics/* { basic_auth; reverse_proxy 127.0.0.1:9090 }
}
```

`systemctl reload caddy` after changes. The Caddy systemd unit from the package
already has `Restart` + boot enablement.

### 5.7 Firewall (public IP)

```bash
# Inside the LXC (nftables/ufw — your choice) — default-deny inbound, loopback free:
#  ALLOW tcp/80,443     (Caddy + ACME)
#  ALLOW udp/7824       (only when D3 is enabled)
#  ALLOW tcp/<ssh-port> (admin IPs only; key-only auth)
#  3340 + 9090: already bound to 127.0.0.1 — no rules needed.
```

If pve-firewall is enabled on the container, mirror the same rules at the PVE layer.

### 5.8 Deployment checklist

1. Create the LXC (5.1) + run the script (quick start above) → the script itself verifies `127.0.0.1:3340/healthz` = 200.
2. DNS A record `relay.<domain>` → LXC IP → `curl -sI https://relay.<domain>/healthz` = 200 (ACME issues the cert on the first hit after DNS resolves).
3. Firewall (5.7 — `--apply-firewall` or manual) + SSH hardening.
4. `systemctl reboot` the LXC once to verify everything comes up by itself (relay + hang-guard timer + Caddy).
5. Wire up the clients (next section) + run the verification checklist.

## Wiring the clients

### Rust clients (iroh 1.x)

```rust
// point the endpoint at your relay instead of the default n0 farm:
let relay_map = RelayMap::from_url(url).with_auth_token(token);
let endpoint = iroh::Endpoint::builder()
    .relay_mode(iroh::RelayMode::Custom(relay_map))
    .bind()
    .await?;
```

(API per docs.rs `iroh` 1.x — `RelayMap::from_url` + `with_auth_token`.)

### Browser / wasm clients

Use the plain HTTPS URL `https://relay.<domain>` wherever a relay list is
configured. The relay accepts the shared token either as
`Authorization: Bearer <token>` or as a `?token=` query parameter on the URL.

## Verification checklist (after deploy)

- [ ] `curl -sI https://relay.<domain>/healthz` = 200 (Caddy TLS + relay alive)
- [ ] Client logs: endpoints register through the new relay (no more n0 hostnames)
- [ ] Connections succeed through the new relay; throughput measurable via metrics 9090
- [ ] `systemctl restart iroh-relay` + LXC `reboot` → clients reconnect, all services come back
- [ ] From the internet: only 80/443 (and 7824/udp if enabled) open; 3340/9090 unreachable
- [ ] Without a token → the relay refuses access (shared_token works)

## Risks / open items

- **Release asset names** — resolved: the script queries the GitHub API for
  `iroh-relay-<ver>-<target>-unknown-linux-gnu.tar.gz` and verifies the asset's
  sha256 `digest`. Default pin v1.0.3; `--latest` for the newest release.
- **Idle timeout** — the server WS-pings every 15s, safe with Caddy defaults; if
  you later put another LB/cloud in front, verify its idle timeout is >15s.
- **QUIC discovery (D3)** — needs its own TLS + UDP firewall opening; separate effort.
- **SPOF** — one relay is one failure point (acceptable: it's signaling + fallback
  only; the direct path stays alive when the relay dies).
- IPv6 — if the LXC has public IPv6, consider an AAAA record + Caddy listen; add later if needed.
