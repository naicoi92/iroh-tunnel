<div align="center">

# iroh-tunnel

**P2P port-forwarding tunnels over [Iroh](https://iroh.computer) — no public IP,
no port forwarding, no relay to rent.**

[![snapshot](https://github.com/naicoi92/iroh-tunnel/actions/workflows/snapshot.yml/badge.svg)](https://github.com/naicoi92/iroh-tunnel/actions/workflows/snapshot.yml)
[![release](https://img.shields.io/github/v/release/naicoi92/iroh-tunnel)](https://github.com/naicoi92/iroh-tunnel/releases)
[![rust](https://img.shields.io/badge/rust-1.91%2B-dea584?logo=rust)](Cargo.toml)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](LICENSE-MIT)
[![homebrew](https://img.shields.io/badge/homebrew-naicoi92%2Ftap-FF7A59?logo=homebrew&logoColor=white)](https://github.com/naicoi92/homebrew-tap)

Run **`serve`** next to any local service — a database, an SSH daemon, a dev
server — and it becomes reachable through Iroh under a `node_id`. Run
**`access`** anywhere else and that service shows up on `localhost`, as if it
ran on your own machine.

</div>

> **Status:** `v0.2.0` — the multiplexing release. TCP tunneling is
> production-shaped; UDP framing exists in the codebase but is **not yet wired
> into the run path**. Binaries are not Cosign-signed yet — Cosign/SBOM/AUR/
> macOS-Intel land in later releases.

---

## Table of contents

- [Why iroh-tunnel?](#why-iroh-tunnel)
- [How it works](#how-it-works)
- [Install](#install)
- [Quick start](#quick-start)
- [How it compares](#how-it-compares)
- [Use cases](#use-cases)
- [Configuration](#configuration)
- [Multiplexing (0.2.0)](#multiplexing-020)
- [Run as a system service](#run-as-a-system-service)
- [Docker & Kubernetes](#docker--kubernetes)
- [Operations](#operations)
- [FAQ / Troubleshooting](#faq--troubleshooting)
- [Security notes](#security-notes)
- [Project layout](#project-layout)
- [Releases](#releases)
- [Contributing](#contributing)
- [Acknowledgments](#acknowledgments)
- [License](#license)

---

## Why iroh-tunnel?

- **Zero network setup.** No public IP, no port forwarding, no DNS, no rented
  relay. Iroh's relay network handles NAT traversal; the data path is direct
  P2P QUIC whenever the networks allow it.
- **True peer-to-peer.** Traffic is end-to-end encrypted between the two
  nodes; relays only help punch holes and carry encrypted bytes.
- **One connection, many channels.** Since 0.2.0 each service keeps a single
  long-lived Iroh connection and opens one QUIC stream per local connection —
  the handshake is paid once, not per channel.
- **Identity, not accounts.** A `node_id` is a self-certifying TLS identity.
  No sign-up, no tokens, no vendor lock-in.
- **One static binary.** No daemon, no runtime deps. Linux `amd64`/`arm64`
  (glibc **and** musl), macOS `arm64`, and `riscv64` for embedded boards.
- **Fits any init.** First-class service management for systemd, launchd,
  and BusyBox/SysV init (buildroot-class devices).
- **Observable by default.** Peer connect/disconnect logs at INFO, an
  atomic `status.json` for monitoring, and meaningful exit codes.

## How it works

```
   access host (your laptop)                 serve host (behind NAT)
  ┌───────────────────────┐                ┌───────────────────────┐
  │                       │  one Iroh P2P  │                       │
  │  psql ──▶ :55432 ─────┼─(node_id,QUIC)─┼────▶ :5432 ── postgres│
  │        access         │   connection   │         serve         │
  │                       │  one stream per│                       │
  │                       │  channel (0.2) │                       │
  └───────────────────────┘                └───────────────────────┘
     any local client connects to          the local service stays
     localhost, as usual                   bound to localhost, too
```

- **`serve`** runs next to a local service (e.g. a DB, dev server, SSH) and
  publishes it into Iroh under a `node_id`.
- **`access`** dials that `node_id` from anywhere and binds the remote service
  to a local port — as if it ran on your machine.

---

## Install

### macOS (Homebrew)

```sh
brew tap naicoi92/tap https://github.com/naicoi92/homebrew-tap
brew install --cask iroh-tunnel
iroh-tunnel --version          # → iroh-tunnel 0.2.0
```

The cask removes the macOS quarantine attribute automatically, so Gatekeeper
will not block the unsigned binary.

### Linux (`.deb` / `.apk`)

Packages ship a systemd unit at `/usr/lib/systemd/system/iroh-tunnel.service`
and run under a hardened `DynamicUser`. The `.deb` is built against **glibc**
(Debian/Ubuntu/Fedora/...); the `.apk` is built against **musl** for Alpine
hosts (LXC, bare-metal) and will not run on a glibc system, and vice versa.

```sh
# Debian / Ubuntu (.deb, glibc)
curl -LO https://github.com/naicoi92/iroh-tunnel/releases/download/v0.2.0/iroh-tunnel_0.2.0_amd64.deb
sudo dpkg -i iroh-tunnel_0.2.0_amd64.deb   # or _arm64.deb on ARM
iroh-tunnel --version

# Alpine (.apk, musl)
curl -LO https://github.com/naicoi92/iroh-tunnel/releases/download/v0.2.0/iroh-tunnel_0.2.0_amd64.apk
sudo apk add --allow-untrusted iroh-tunnel_0.2.0_amd64.apk   # or _arm64.apk on ARM
```

### Docker

```sh
docker run --rm ghcr.io/naicoi92/iroh-tunnel:v0.2.0 --version
# → iroh-tunnel 0.2.0
```

Tags: `v0.2.0`, `v0.1`, `latest`. Platforms: `linux/amd64`, `linux/arm64`.

<details>
<summary><strong>Build from source</strong></summary>

Requires Rust 1.91+ (matches `rust-version` in `Cargo.toml`).

```sh
git clone https://github.com/naicoi92/iroh-tunnel && cd iroh-tunnel
cargo run --release -- --help
```

</details>

---

## Quick start

Two terminals, two machines — or two windows on one machine to try it out.

**1. On the serve host** (the machine with the local service, e.g. postgres
on `:5432`):

```sh
# Generate a config + a fresh secret key (this prints/uses a NEW node_id).
iroh-tunnel serve config keygen

# Add the service you want to expose.
iroh-tunnel serve config add --name postgres --protocol tcp --port 5432

# Run it — note the NodeId it prints. That's the address clients dial.
iroh-tunnel serve config show
iroh-tunnel serve run        # prints:  NodeId: <hex>
```

**2. On the access host** (your laptop, anywhere):

```sh
# Pin this access node's identity: generates a secret_key so the access NodeId
# is stable across restarts (the serve side will see the same peer every time).
iroh-tunnel access config keygen

# Point at the serve NodeId you just copied, bind it to a local port.
iroh-tunnel access config add \
  --name postgres \
  --protocol tcp \
  --node_id <PASTE_NODE_ID_HERE> \
  --host 127.0.0.1 \
  --port 55432

iroh-tunnel access run
# prints: NodeId: <your access node's hex>
```

**3. Use it like a local service:**

```sh
psql -h 127.0.0.1 -p 55432    # hits the remote postgres via the tunnel
```

Both sides log peer connect/disconnect at INFO (shown by default), each
carrying the **remote** NodeId so you can correlate activity across the two
hosts:

```
# serve side:                    # access side:
INFO peer=<access id> … peer connected      INFO peer=<serve id> … connected to serve peer
INFO peer=<access id> … peer disconnected   INFO peer=<serve id> … disconnected from serve peer
```

If you skip `access config keygen`, a key is generated automatically on the
first `run` and persisted to the config — so the NodeId is still stable, just
not chosen by you up front.

---

## How it compares

A quick orientation, not a benchmark — check each project's current docs
before deciding.

|                              | **iroh-tunnel**      | ngrok                | cloudflared tunnel    | Tailscale            | frp                  | bore                 |
|------------------------------|----------------------|----------------------|-----------------------|----------------------|----------------------|----------------------|
| Transport model              | P2P QUIC, relay fallback | hosted TCP/HTTP tunnels | Cloudflare edge       | WireGuard mesh       | proxy via `frps`     | TCP relay            |
| Needs your own public-IP server | no               | no                   | no                    | no                   | **yes**              | yes (or their cloud) |
| Forwards arbitrary TCP       | yes                  | yes                  | via WARP client       | L3 routes (any IP)   | yes                  | yes                  |
| UDP                          | groundwork only, not wired yet | limited, paid tiers | no                    | L3 routes (any IP)   | yes                  | no                   |
| Identity / auth              | `node_id`, no accounts | account + token    | Cloudflare account + domain | SSO / MagicDNS  | shared token         | shared key           |
| Client side needs            | the same static binary | a URL (or their CLI for raw TCP) | `cloudflared` / WARP | join the tailnet | `frpc` config        | the `bore` client    |

Where iroh-tunnel is **not** the right tool: you need HTTP-edge features
(TLS certs, WAF, custom domains) — use cloudflared/ngrok; you need a full
L3 network between many machines — use Tailscale; you already run a public
server and want classic port-mapping — frp fits.

## Use cases

- **Reach a dev database from anywhere.** Postgres/Redis/MySQL on your work
  machine or CI box, consumed from your laptop — the walkthrough above.
- **SSH into a homelab behind CGNAT.**
  `serve config add --name ssh --protocol tcp --port 22` on the box,
  then `access ... --port 2222` on the laptop, and
  `ssh -p 2222 user@127.0.0.1`.
- **Expose an in-cluster Kubernetes service** to your workstation without
  Ingress or `port-forward` sessions that die — see
  [Docker & Kubernetes](#docker--kubernetes).
- **Embedded boards.** Static `riscv64` musl binaries and a BusyBox/SysV init
  backend mean a buildroot device (e.g. Sipeed NanoKVM) can publish its web
  UI or SSH to you, wherever it boots.
- **Chatty clients.** DB pools, HTTP/2 sessions, parallel SSH channels — the
  multiplexed transport keeps them all on one connection; see below.

---

## Configuration

If you omit `--config`, the file is read from the OS config dir:

| OS      | Path                                        |
|---------|---------------------------------------------|
| Linux   | `~/.config/iroh-tunnel/{serve,access}.toml` |
| macOS   | `~/Library/Application Support/iroh-tunnel/{serve,access}.toml` |

Status file (written by `run`, for monitoring): `~/.local/state/iroh-tunnel/status.json`
(Linux) or `~/Library/Application Support/iroh-tunnel/status.json` (macOS).

Sample files ship in [`examples/`](examples/) — `serve.toml` and `access.toml`
with a throwaway demo key so the compose demo runs as-is.

## Multiplexing (0.2.0)

With `multiplex = true` (default, per service on the access side), each
service keeps **one long-lived Iroh connection** between access and serve,
and every local TCP connection rides its own QUIC bidirectional stream on
that connection. The relay session and QUIC/TLS handshake are paid once, not
once per channel.

```toml
# access.toml
[[services]]
name = "postgres"
node_id = "…"
protocol = "tcp"
host = "127.0.0.1"
port = 5433
multiplex = true   # default; false = one connection per channel (pre-0.2.0)
```

**Rollout: upgrade serve before access.** There is deliberately no protocol
negotiation — the ALPN is unchanged. A 0.2.0+ serve is fully
backward-compatible with every access version, so upgrading serve nodes
first has no caveat. The reverse is not supported: a multiplexing access
against a pre-0.2.0 serve would hang its second stream (the old serve
accepts exactly one stream per connection) — if you must run a new access
against an old serve, set `multiplex = false` on that service.

**When it helps:** many parallel channels to one serve node (a DB pool, many
HTTP clients, SSH sessions). **When to turn it off:** if you want hard
isolation between channels (with `false`, a channel's failure domain is its
own connection), or while a serve peer is not yet upgraded.

**Operational notes:**

- Both roles pin a 5 s QUIC keep-alive, and keep noq's own concurrent
  bidirectional-stream budget by default (100 per connection). Override per
  node only when a measured workload needs more —

  ```toml
  [node]
  max_concurrent_streams = 512
  ```

  The budget is headroom, not a requirement. Worst-case buffer memory scales
  with `max_concurrent_streams × stream_receive_window`, so raising it has a
  memory cost; when all slots are busy, a new channel's `open_bi` is
  flow-control blocked until another stream closes.
- A dead multiplexed connection surfaces as EOF on its channels (correct
  port-forward semantics); the next channel dials a fresh connection.
- In the serve's `status.json`, `active_connections` counts **active
  streams** (in-flight pipes), which is the operator-meaningful number once
  one connection can carry many channels.

---

## Run as a system service

`iroh-tunnel service ...` manages a systemd unit (Linux) or launchd plist
(macOS) generated from the config, so the tunnel comes back after reboot.

By default the service installs at **user scope** — no privileges needed:
`systemctl --user` on Linux, a per-user LaunchAgent on macOS. This matches
how iroh-tunnel is normally used on a desktop (the service runs as the same
user that owns the config under `$HOME`). Pass `--system` for a system-wide
daemon (LaunchDaemon / `/etc/systemd/system`) on servers / headless hosts;
that requires `sudo`.

```sh
# Per-user (default, no sudo) — Linux systemd --user or macOS LaunchAgent
iroh-tunnel access service install
iroh-tunnel access service start
iroh-tunnel access service status

# System-wide (--system, requires sudo) — for servers / headless hosts
sudo iroh-tunnel serve service install --system    # /etc/systemd/system/iroh-tunnel-serve.service
# or on macOS: /Library/LaunchDaemons/dev.iroh-tunnel-serve.plist
sudo iroh-tunnel serve service start --system
```

Subcommands: `install`, `uninstall`, `start`, `stop`, `restart`, `status`.
Each accepts `--system` to target the system-wide daemon; the default is the
per-user service.

**Non-systemd Linux (BusyBox/SysV init, e.g. Sipeed NanoKVM / buildroot
devices):** `service install` detects at runtime that the host does not run
systemd and installs an `/etc/init.d/S96iroh-tunnel-<role>` script instead
(boot-order 96: after networking). The same six subcommands work; scope is
always system-wide there (BusyBox init has no per-user services), and
installing requires root.

---

## Docker & Kubernetes

### Prebuilt image (multi-arch, from ghcr.io)

```sh
docker run --rm \
  -v "$PWD/serve.toml:/etc/iroh-tunnel/serve.toml:ro" \
  ghcr.io/naicoi92/iroh-tunnel:v0.2.0 \
  serve run --config /etc/iroh-tunnel/serve.toml
```

### Compose demo — tunnel an nginx through Iroh

The repo ships a working demo in `docker-compose.yml`:

```sh
# 1. Start the service + the serve side.
docker compose up -d web serve

# 2. Read the serve NodeId from logs.
docker compose logs serve | grep NodeId
# → NodeId: e8bb34671b11ca03ca88f7f8b500b07ab3086be8cfd168b3d041d3f478837de9

# 3. Paste that NodeId into examples/access.toml (node_id field), then:
docker compose up access
curl http://127.0.0.1:8080        # nginx default page, tunneled via Iroh
```

### Kubernetes — expose an in-cluster `postgres` Service

iroh-tunnel is a single static binary with a config file — run it as either a
**Deployment** (the `serve` side, exposing an in-cluster Service) or a sidecar.

<details>
<summary><strong>Minimal serve Deployment (ConfigMap + Deployment)</strong></summary>

```yaml
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: iroh-tunnel-serve
data:
  serve.toml: |
    # secret_key left empty: the pod generates one on first run.
    # For a stable node_id across restarts, generate a key with
    # `iroh-tunnel serve config keygen` and store it in a Secret instead.
    [node]
    secret_key = ""

    [[services]]
    name = "postgres"
    protocol = "tcp"
    host = "postgres.default.svc.cluster.local"
    port = 5432
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: iroh-tunnel-serve
  labels:
    app: iroh-tunnel
spec:
  replicas: 1
  selector:
    matchLabels:
      app: iroh-tunnel
  template:
    metadata:
      labels:
        app: iroh-tunnel
    spec:
      containers:
        - name: iroh-tunnel
          image: ghcr.io/naicoi92/iroh-tunnel:v0.2.0
          args: ["serve", "run", "--config", "/etc/iroh-tunnel/serve.toml"]
          volumeMounts:
            - name: config
              mountPath: /etc/iroh-tunnel
              readOnly: true
          # The NodeId is printed to stdout; grab it with:
          #   kubectl logs deploy/iroh-tunnel-serve | grep NodeId
      volumes:
        - name: config
          configMap:
            name: iroh-tunnel-serve
```

Apply, then read the NodeId clients will dial:

```sh
kubectl apply -f iroh-tunnel-serve.yaml
kubectl logs deploy/iroh-tunnel-serve | grep NodeId
```

</details>

**Notes for K8s:**

- **Stable `node_id`:** the example regenerates a key on each pod start, so
  the `node_id` changes on rollout. For a stable id, generate a key once
  (`iroh-tunnel serve config keygen`), put it in a `Secret`, and mount it
  into the config — or use a `StatefulSet` + a `volumeClaimTemplate` for a
  writable config the pod persists.
- **No host networking needed:** Iroh dials out to its relay network, so the
  pod only needs normal egress. You do **not** need a `Service` or `Ingress`
  in front of the iroh-tunnel pod.
- **Health:** the binary writes `status.json` to the state dir. A future task
  adds an HTTP `/healthz` probe; for now, a simple `exec` probe on
  `iroh-tunnel --version` suffices for liveness.

---

## Operations

### CLI reference

```
iroh-tunnel <ROLE> <COMMAND>

Roles:    serve | access
Commands:
  run       Run in the foreground
  config    Manage config (keygen | add | remove | list | show | edit | path)
  service   Manage systemd/launchd service (install | start | stop | restart | status | uninstall)

Flags:    -v / -vv   increase logging (debug/trace)  ·  -q quiet (errors only)  ·  --color auto|always|never
```

### Exit codes

| Code | Meaning    |
|------|------------|
| `0`  | success    |
| `1`  | general    |
| `2`  | config     |
| `3`  | permission |
| `4`  | iroh       |
| `5`  | service    |

### Logging

The default log level is **info**, so peer connect/disconnect and "endpoint
ready" notices show without any flag. Use `-q` for errors-only, or
`-v`/`-vv` for debug/trace. `RUST_LOG` overrides everything (e.g.
`RUST_LOG=iroh_tunnel=debug iroh-tunnel serve run`).

### Monitoring

`run` writes `status.json` (see [Configuration](#configuration) for the
path) atomically — `node_id`, `home_relay`, uptime, and
`active_connections` (which counts in-flight **streams**, the
operator-meaningful number under multiplexing).

---

## FAQ / Troubleshooting

**The serve `NodeId` changes after every restart.**
The config has no persisted `secret_key`. Run `serve config keygen` once —
it generates and stores a key so the `node_id` is stable for the life of the
config. The same applies to `access config keygen`.

**The `.apk` package won't run on Debian/Ubuntu (or the `.deb` fails on Alpine).**
The `.apk` is musl, the `.deb` is glibc — they are not interchangeable.
Pick the one matching the host libc.

**A second connection just hangs against an old serve node.**
You are running a 0.2.0+ access with `multiplex = true` against a pre-0.2.0
serve. Upgrade the serve node first, or set `multiplex = false` on that
service. See [Multiplexing](#multiplexing-020).

**Is UDP supported?**
Not yet. The framing codec (`[len][payload]`) and UDP pipe are in the
codebase, but they are not wired into the run path. TCP is the supported
protocol today.

**How do I see what the tunnel is doing?**
`-v`/`-vv` for debug/trace, `RUST_LOG=iroh_tunnel=debug` for full control,
`status.json` for a monitoring-friendly snapshot, and the
[exit codes](#exit-codes) for scripting. Both roles log the *remote* NodeId
on connect/disconnect so the two sides can be correlated.

**Can I run serve and access on the same machine?**
Yes — every quick-start command works in two terminals on one host. The
configs are separate (`serve.toml` / `access.toml`) and the NodeIds are
distinct.

---

## Security notes

- Traffic between the two nodes is end-to-end encrypted QUIC/TLS; Iroh relays
  only see encrypted bytes and are used for NAT traversal, not as a trusted
  middleman.
- The `access` side pins the serve `node_id` (a self-certifying TLS identity),
  so it always reaches the intended serve node.
- The `serve` side has **no peer allowlist**: any peer that learns the
  `node_id` and the service name can connect and reach the exposed local
  service. Treat the `node_id` as a bearer capability and keep secret keys
  out of shared configs and logs.

For reporting vulnerabilities and the full policy, see
[SECURITY.md](SECURITY.md).

---

## Project layout

| Path                       | Purpose                                                |
|----------------------------|--------------------------------------------------------|
| `src/cli.rs`               | clap CLI surface (`<role> <command>`)                  |
| `src/serve.rs`             | serve role: publish local services into Iroh           |
| `src/access.rs`            | access role: dial a remote node_id to a local port     |
| `src/role_run.rs`          | shared run skeleton: dial with retry/backoff, watchers |
| `src/pipe.rs`              | byte-copy pipes between Iroh streams and local sockets |
| `src/endpoint.rs`          | Iroh endpoint construction + transport tuning          |
| `src/proto.rs`             | ALPN protocol constants (`iroh-tunnel/{name}`)         |
| `src/config.rs`, `src/config_cmd.rs` | config model + `config` subcommands           |
| `src/service/`             | service backends: `systemd`, `launchd`, BusyBox init   |
| `src/status.rs`            | atomic `status.json` writer                            |
| `tests/`                   | integration tests (network suite, `--ignored`)         |
| `.goreleaser.yaml`         | Linux release pipeline (binaries, Docker, .deb/.apk)   |
| `.goreleaser.macos.yaml`   | macOS release pipeline (darwin binary, Homebrew cask)  |
| `packaging/`               | systemd unit + deb postinstall used by nFPM            |
| `examples/`                | sample `serve.toml` / `access.toml`                    |

## Releases

One git tag `vX.Y.Z` produces, via [GoReleaser](https://goreleaser.com):

- **Binaries:** `linux/amd64`, `linux/arm64` (glibc + musl), `darwin/arm64`
  (+ checksums), `riscv64gc` musl
- **Linux packages:** `.deb` (glibc) + `.apk` (musl/Alpine) — amd64 + arm64
- **Docker:** `ghcr.io/naicoi92/iroh-tunnel` multi-arch (amd64 + arm64)
- **Homebrew cask:** published to [`naicoi92/homebrew-tap`](https://github.com/naicoi92/homebrew-tap)

See the [Releases page](https://github.com/naicoi92/iroh-tunnel/releases).

## Contributing

Bug reports and pull requests are welcome — see
[CONTRIBUTING.md](CONTRIBUTING.md) for the development setup, the checks CI
runs, and the compatibility rules. For security issues, follow
[SECURITY.md](SECURITY.md) instead of opening a public issue.

## Acknowledgments

- [iroh](https://github.com/n0-computer/iroh) and the n0 team — the P2P
  transport, relay network, and QUIC stack (noq) this project builds on.
- [GoReleaser](https://goreleaser.com) and its maintainers — the cross-platform
  release pipeline.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option.
