# iroh-tunnel

P2P port-forwarding tunnel (TCP/UDP) over [Iroh](https://iroh.computer).
Expose a local service to the internet via an Iroh `node_id` — no public IP,
port forwarding, or relay server required.

> **Status:** `v0.1.0-rc.1` — prerelease. Core serve/access tunneling works;
> the binary is **not yet Cosign-signed** (deferred). Cosign/SBOM/AUR/macOS-Intel
> land in later releases.

---

## How it works

```
┌─────────┐                                  ┌──────────┐
│  serve  │ ──── Iroh P2P (node_id) ───────► │  access  │
│ (behind │   exposes local service          │ exposes  │
│  NAT)   │                                  │ on local │
└─────────┘                                  │  port    │
                                             └──────────┘
```

- **`serve`** runs next to a local service (e.g. a DB, dev server) and publishes
  it into Iroh under a `node_id`.
- **`access`** dials that `node_id` from anywhere and binds the remote service
  to a local port — as if it ran on your machine.

No public IP, no port forwarding, no relay to rent. Iroh's relay network
handles NAT traversal.

---

## Install

### macOS (Homebrew) — recommended

```sh
brew tap naicoi92/tap https://github.com/naicoi92/homebrew-tap
brew install --cask iroh-tunnel
iroh-tunnel --version          # → iroh-tunnel 0.1.0-rc.1
```

The cask removes the macOS quarantine attribute automatically, so Gatekeeper
will not block the unsigned binary.

### Linux (`.deb` / `.apk`)

Packages ship a systemd unit at `/usr/lib/systemd/system/iroh-tunnel.service`
and run under a hardened `DynamicUser`.

```sh
# Debian / Ubuntu (.deb)
curl -LO https://github.com/naicoi92/iroh-tunnel/releases/download/v0.1.0-rc.1/iroh-tunnel_0.1.0-rc.1_amd64.deb
sudo dpkg -i iroh-tunnel_0.1.0-rc.1_amd64.deb   # or _arm64.deb on ARM
iroh-tunnel --version

# Alpine (.apk)
curl -LO https://github.com/naicoi92/iroh-tunnel/releases/download/v0.1.0-rc.1/iroh-tunnel_0.1.0-rc.1_amd64.apk
sudo apk add --allow-untrusted iroh-tunnel_0.1.0-rc.1_amd64.apk
```

### Docker (prebuilt multi-arch image)

```sh
docker run --rm ghcr.io/naicoi92/iroh-tunnel:v0.1.0-rc.1 --version
# → iroh-tunnel 0.1.0-rc.1
```

Tags: `v0.1.0-rc.1`, `v0.1.0`, `v0.1`, `latest`. Platforms: `linux/amd64`,
`linux/arm64`.

### Build from source

Requires Rust 1.91+ (matches `rust-version` in `Cargo.toml`).

```sh
git clone https://github.com/naicoi92/iroh-tunnel && cd iroh-tunnel
cargo run --release -- --help
```

---

## Quick start (macOS / Linux)

### 1. On the **serve** host (the machine with the local service)

```sh
# Generate a config + a fresh secret key (this prints/uses a NEW node_id).
iroh-tunnel serve config keygen

# Add the service you want to expose (e.g. a local postgres on :5432).
iroh-tunnel serve config add --name postgres --protocol tcp --port 5432

# Show the config + note the NodeId printed when you run it.
iroh-tunnel serve config show
iroh-tunnel serve run        # prints:  NodeId: <hex>
```

`serve run` prints the **NodeId** (hex) — copy it. That's the address clients
dial.

### 2. On the **access** host (your laptop, anywhere)

```sh
# Point at the serve NodeId you just copied, bind it to a local port.
iroh-tunnel access config add \
  --name postgres \
  --protocol tcp \
  --node_id <PASTE_NODE_ID_HERE> \
  --host 127.0.0.1 \
  --port 55432

iroh-tunnel access run
# now:  psql -h 127.0.0.1 -p 55432   # hits the remote postgres via the tunnel
```

### Default config locations

If you omit `--config`, the file is read from the OS config dir:

| OS      | Path                                      |
|---------|-------------------------------------------|
| Linux   | `~/.config/iroh-tunnel/{serve,access}.toml` |
| macOS   | `~/Library/Application Support/iroh-tunnel/{serve,access}.toml` |

Status file (written by `run`, for monitoring): `~/.local/state/iroh-tunnel/status.json` (Linux) or `~/Library/Application Support/iroh-tunnel/status.json` (macOS).

---

## Run as a system service

`iroh-tunnel service ...` manages a systemd unit (Linux) or launchd plist
(macOS) generated from the config, so the tunnel comes back after reboot.

```sh
# Linux (systemd, system-wide)
sudo iroh-tunnel serve service install             # /etc/systemd/system/iroh-tunnel-serve.service
iroh-tunnel serve service start
iroh-tunnel serve service status

# Linux (systemd --user, no sudo)
iroh-tunnel access service install --user
iroh-tunnel access service start --user

# macOS (launchd, user-level LaunchAgent)
iroh-tunnel serve service install --user
launchctl load ~/Library/LaunchAgents/iroh-tunnel-serve.plist
```

Subcommands: `install`, `uninstall`, `start`, `stop`, `restart`, `status`.
The `--user` flag selects user-level scope (LaunchAgent / `systemctl --user`).

---

## Docker

### Prebuilt image (multi-arch, from ghcr.io)

```sh
docker run --rm \
  -v "$PWD/serve.toml:/etc/iroh-tunnel/serve.toml:ro" \
  ghcr.io/naicoi92/iroh-tunnel:v0.1.0-rc.1 \
  serve run --config /etc/iroh-tunnel/serve.toml
```

### Compose demo (tunnel an nginx through Iroh)

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

`examples/serve.toml` and `examples/access.toml` ship with a throwaway demo
key so the compose demo runs as-is; for a real deployment generate your own
with `serve config keygen`.

---

## Kubernetes

iroh-tunnel is a single static binary with a config file — run it as either a
**Deployment** (the `serve` side, exposing an in-cluster Service) or a sidecar.
Below is a minimal `serve` Deployment that publishes a ClusterIP Service.

### Example: expose an in-cluster `postgres` Service

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
          image: ghcr.io/naicoi92/iroh-tunnel:v0.1.0-rc.1
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

### Notes for K8s

- **Stable `node_id`:** the example regenerates a key on each pod start, so the
  `node_id` changes on rollout. For a stable id, generate a key once
  (`iroh-tunnel serve config keygen`), put it in a `Secret`, and mount it into
  the config — or use a `StatefulSet` + a `volumeClaimTemplate` for a writable
  config the pod persists.
- **No host networking needed:** Iroh dials out to its relay network, so the
  pod only needs normal egress. You do **not** need a `Service` or `Ingress`
  in front of the iroh-tunnel pod.
- **Health:** the binary writes `status.json` to the state dir. A future task
  adds an HTTP `/healthz` probe; for now, a simple `exec` probe on
  `iroh-tunnel --version` suffices for liveness.

---

## CLI reference

```
iroh-tunnel <ROLE> <COMMAND>

Roles:    serve | access
Commands:
  run       Run in the foreground
  config    Manage config (keygen | add | remove | list | show | edit | path)
  service   Manage systemd/launchd service (install | start | stop | restart | status | uninstall)

Flags:    -v / -vv / -vvv   increase logging  ·  -q quiet  ·  --color auto|always|never
```

Exit codes: `0` success · `1` general · `2` config · `3` permission · `4` iroh · `5` service.

Logging respects `RUST_LOG` (e.g. `RUST_LOG=iroh_tunnel=debug iroh-tunnel serve run`).

---

## Project layout

| Path                       | Purpose                                                |
|----------------------------|--------------------------------------------------------|
| `src/cli.rs`               | clap CLI surface (`<role> <command>`)                  |
| `src/serve.rs`             | serve role: publish local services into Iroh           |
| `src/access.rs`            | access role: dial a remote node_id to a local port     |
| `src/service.rs`           | systemd/launchd unit generation + `systemctl` wrappers |
| `src/status.rs`            | atomic `status.json` writer                            |
| `.goreleaser.yaml`         | Linux release pipeline (binaries, Docker, .deb/.apk)   |
| `.goreleaser.macos.yaml`   | macOS release pipeline (darwin binary, Homebrew cask)   |
| `packaging/`               | systemd unit + deb postinstall used by nFPM            |
| `examples/`                | sample `serve.toml` / `access.toml`                    |

---

## Releases

One git tag `vX.Y.Z` produces, via [GoReleaser](https://goreleaser.com):

- **Binaries:** `linux/amd64`, `linux/arm64`, `darwin/arm64` (+ checksums)
- **Linux packages:** `.deb` + `.apk` (amd64 + arm64)
- **Docker:** `ghcr.io/naicoi92/iroh-tunnel` multi-arch (amd64 + arm64)
- **Homebrew cask:** published to [`naicoi92/homebrew-tap`](https://github.com/naicoi92/homebrew-tap)

See the [Releases page](https://github.com/naicoi92/iroh-tunnel/releases).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
