<div align="center">

# iroh-tunnel

**Peer-to-peer port forwarding over [Iroh](https://iroh.computer).
No public IP, no port forwarding, no server to rent.**

[![ci](https://github.com/naicoi92/iroh-tunnel/actions/workflows/ci.yml/badge.svg)](https://github.com/naicoi92/iroh-tunnel/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/naicoi92/iroh-tunnel)](https://github.com/naicoi92/iroh-tunnel/releases)
[![rust](https://img.shields.io/badge/rust-1.91%2B-dea584?logo=rust)](https://github.com/naicoi92/iroh-tunnel/blob/main/Cargo.toml)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](LICENSE-MIT)

</div>

Run **`serve`** next to a service (a database, an SSH daemon, a dev server)
and it becomes reachable through Iroh under a NodeId. Run **`access`** on any
other machine and the service appears on `localhost` there.

```sh
# Host with the service
iroh-tunnel serve -n postgres -t 127.0.0.1:5432
# NodeId: 1aa27080cc69...

# Your laptop
iroh-tunnel access -n postgres -l 127.0.0.1:15432 -N 1aa27080cc69...
psql -h 127.0.0.1 -p 15432
```

> **Status:** pre-1.0. TCP is the supported protocol; UDP forwarding is
> [experimental](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/udp.md).

## Features

- **Zero network setup.** Iroh's relays handle NAT traversal and the data
  moves to a direct QUIC path whenever the networks allow it.
- **End-to-end encrypted.** QUIC/TLS 1.3 between the two nodes; relays only
  carry encrypted bytes.
- **Identity, not accounts.** A NodeId is a self-certifying public key: no
  sign-up, no tokens, no vendor.
- **One connection, many channels.** Each service keeps one Iroh connection
  and gives every local client its own QUIC stream.
- **Small self-contained binaries** for Linux (glibc, or static musl; amd64, arm64,
  riscv64), macOS (Apple silicon) and Windows (x64, ARM64).
- **Service management** for systemd, launchd, BusyBox/SysV init and
  Windows Task Scheduler.
- **Observable.** `status` tables and JSON status files show each peer's
  relay or direct path; exit codes are meaningful.
- **Verified self-update** against keyless Sigstore signatures.

## Install

```sh
# Linux and macOS (Apple silicon)
curl -fsSL https://raw.githubusercontent.com/naicoi92/iroh-tunnel/main/install.sh | sudo sh

# macOS with Homebrew
brew tap naicoi92/tap https://github.com/naicoi92/homebrew-tap
brew install --cask iroh-tunnel

# Docker
docker run --rm ghcr.io/naicoi92/iroh-tunnel:latest --version
```

Windows (elevated PowerShell):

```powershell
irm https://raw.githubusercontent.com/naicoi92/iroh-tunnel/main/install.ps1 -OutFile install.ps1
powershell -ExecutionPolicy Bypass -File install.ps1
```

The installers put three executables on your `PATH`: `iroh-tunnel` (both
roles), `iroh-tunnel-serve` and `iroh-tunnel-access`. On Linux and macOS
they also add the aliases `it`, `its` and `ita`. `.deb`/`.apk` packages,
per-role images, archives and building from source are covered in
[Install](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/install.md).

## Quick start

### 1. Try it without a config file

Start Serve next to the service. It prints its NodeId:

```sh
iroh-tunnel serve -n postgres -t 127.0.0.1:5432
```

On another machine (or another terminal), start Access with that NodeId:

```sh
iroh-tunnel access -n postgres -l 127.0.0.1:15432 -N <SERVE_NODE_ID>
```

Clients now connect to `127.0.0.1:15432`. The standalone executables take
the same flags without the role word: `its -n postgres -t 127.0.0.1:5432`.
Add `-p udp` for [UDP](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/udp.md). Quick mode uses a fresh NodeId on every
start. To keep it, save the `Secret key:` value printed by
`iroh-tunnel serve config keygen --print` to a private file and pass
`-k FILE`.

**NodeId is public identity, not target authorization.** Anyone who knows a
NodeId and service name can connect. Target services must enforce their own access protection;
the tunnel does not add an authorization scheme. See the
[security model](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/how-it-works.md#security-model).

### 2. Keep it running with a config file

On the Serve host:

```sh
iroh-tunnel serve config keygen               # create a stable identity
iroh-tunnel serve config set -n postgres -t 5432
iroh-tunnel serve run                         # prints NodeId: <hex>
```

On the Access host:

```sh
iroh-tunnel access config set -n postgres -l 15432 -N <SERVE_NODE_ID>
iroh-tunnel access run
```

To start a role at boot, register it as a service:

```sh
iroh-tunnel serve service install
```

More in [Configuration](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/configuration.md) and
[Running as a service](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/services.md).

### 3. Check the connection

```console
$ iroh-tunnel access status
node_id: acc01c0d4c694eb1e756c5e9260eb267208f7b677e6a74e53146bf417fb31f84
pid: 8460  started_at: 1755648010

SERVICE   LISTEN           PEER       TRANSPORTS
--------  ---------------  ---------  -----------------------------------------------
postgres  127.0.0.1:15432  1aa27080…  relay https://use1-1.relay.n0.iroh.link./ [active]
                                      direct 192.168.1.10:54177 [active]
```

`[active]` marks the paths in use: here a direct path, with the relay kept as
fallback. Logs show the same in `connected to serve peer ... via` lines; see
[Monitoring](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/monitoring.md).

## Use cases

- **Reach a development database** on a work machine or CI box from your
  laptop.
- **SSH into a homelab behind CGNAT:** serve port 22, access it on
  `127.0.0.1:2222`, then `ssh -p 2222 user@127.0.0.1`.
- **Expose an in-cluster Kubernetes service** without Ingress or a
  `kubectl port-forward` that dies; see
  [Docker and Kubernetes](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/deployment.md).
- **Embedded boards.** Static riscv64 builds and BusyBox init support let a
  buildroot device publish its web UI or SSH from wherever it boots.

## GitHub Actions

Reach a private service from a workflow, or expose one from a runner:

```yaml
- uses: naicoi92/iroh-tunnel@v0
  with:
    name: postgres
    node-id: ${{ vars.POSTGRES_NODE_ID }}
    listen: 127.0.0.1:15432
```

```yaml
- uses: naicoi92/iroh-tunnel/serve@v0
  with:
    name: web
    target: 127.0.0.1:8080
```

See [GitHub Actions](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/github-actions.md).

## When to use something else

iroh-tunnel forwards fixed ports between two machines. Look elsewhere if you
need HTTP edge features such as TLS certificates or custom domains
(cloudflared, ngrok), a full network between many machines (Tailscale or
another WireGuard mesh), or port mapping through a server you already run
(frp, bore).

## Documentation

| | |
|---|---|
| [Install](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/install.md) | Every install method, `update`, verifying releases |
| [Configuration](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/configuration.md) | Config files, keys, relays, validation |
| [Running as a service](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/services.md) | systemd, launchd, BusyBox init, Windows |
| [Docker and Kubernetes](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/deployment.md) | Images, compose demo, Kubernetes |
| [Monitoring and logs](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/monitoring.md) | `status`, status files, log events, exit codes |
| [GitHub Actions](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/github-actions.md) | Access and Serve steps for CI/CD |
| [UDP (experimental)](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/udp.md) | Behavior, limits and tuning |
| [Self-hosted relay](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/self-hosted-relay.md) | Running your own iroh-relay |
| [CLI reference](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/cli.md) | Every command and flag |
| [How it works](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/how-it-works.md) | Architecture, lifecycle, security, compatibility |
| [Troubleshooting](https://github.com/naicoi92/iroh-tunnel/blob/main/docs/troubleshooting.md) | Common problems |

## Contributing

Bug reports and pull requests are welcome; see
[CONTRIBUTING.md](https://github.com/naicoi92/iroh-tunnel/blob/main/CONTRIBUTING.md). Report security issues privately as
described in [SECURITY.md](https://github.com/naicoi92/iroh-tunnel/blob/main/SECURITY.md). Changes are listed in the
[changelog](https://github.com/naicoi92/iroh-tunnel/blob/main/CHANGELOG.md).

## Acknowledgments

Built on [iroh](https://github.com/n0-computer/iroh) and its QUIC stack by
the n0 team. Linux packages are built with
[nFPM](https://nfpm.goreleaser.com).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option.
