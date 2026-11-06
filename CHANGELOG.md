# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Before 1.0.0, a minor release may contain breaking changes; they are marked
**Breaking**.

## [Unreleased]

### BREAKING

The `config` command line is reworked. Config files are unchanged.

| Old | New |
|---|---|
| `config add --name N --protocol P --port N [--host H]` | `config set -n N -t [HOST:]PORT [-p P]` (Serve) |
| `config add ... --node-id ID --port N` | `config set -n N -l [HOST:]PORT -N ID [-p P]` (Access) |
| `config remove --name N` | `config rm -n N` |
| `config keygen random` | `config keygen --print` |

`config set` creates or updates a service and prints what changed.

### Added

- GitHub Actions: Access (naicoi92/iroh-tunnel) and Serve (naicoi92/iroh-tunnel/serve) for CI/CD.
- `config check` validates a config file without changing it.
- Usage errors print `error: ...` and the help of the failing subcommand, and
  exit `2`; bare `it`, `its`, `ita`, role, `config` and `service` print help.
- Targets and listeners accept `[HOST:]PORT`, including hostnames and bracketed
  IPv6; a bare port means `127.0.0.1`.
- `-r` is a short option for `--relay-url`; short options for `-c`, `-n`, `-t`,
  `-l`, `-N`, `-p`, `-k` and `-y` are consistent across commands.

## [0.5.0] - 2026-11-06

The largest release so far: quick mode, standalone role executables, UDP,
Windows, installers, verified self-update, and a reworked configuration,
service and shutdown lifecycle.

**Upgrade notes**

- **Breaking:** on Linux and macOS, `XDG_CONFIG_HOME` and `XDG_STATE_HOME`
  are ignored; config always lives in `~/.config/iroh-tunnel` and status files
  in `~/.local/state/iroh-tunnel`. Use `--config` and `IROH_TUNNEL_STATE_DIR`
  for other locations.
- **Breaking:** on macOS the config and status files move out of
  `~/Library/Application Support/iroh-tunnel`. The legacy config is copied on
  first use; monitoring that reads the old status path must move.
- **Breaking:** `config list`, `config show` and `run` without a config file
  now fail with exit code `2` instead of succeeding.
- **Breaking:** Linux packages no longer ship a systemd unit, `/etc/iroh-tunnel`
  or a postinstall script. Register services with `iroh-tunnel <role> service
  install`.
- **Breaking:** glibc builds need glibc 2.35 or newer (was 2.31). The static
  musl builds are unaffected.
- A second process of the same role is now refused with exit code `6`.
- Access ignores `multiplex = false` and always reuses a connection. Serve
  older than 0.2.0 is no longer usable; upgrade Serve first.

### Added

- **Quick mode:** `serve -n NAME -t IP:PORT` and
  `access -n NAME -l IP:PORT -N NODE_ID` run one service without a config
  file, with a fresh identity per start, `--relay-url` and `-p udp`. NodeId is
  public identity, not target authorization; protect exposed services.
- **Persistent key file** for quick mode: `-k` / `--key-file PATH` (#82).
- **Standalone role executables** `iroh-tunnel-serve` (`its`) and
  `iroh-tunnel-access` (`ita`), each with its own archive, `.deb`/`.apk`
  package and container image (#82, #94). The all-in-one `iroh-tunnel` gains
  the `it` alias.
- **Experimental UDP forwarding** over QUIC datagrams with per-client flows,
  idle expiry, flow caps per service and per peer, bounded per-flow queues,
  quarantine of ended flows, and per-reason drop counters in the status files.
  New optional UDP service keys: `idle_timeout`, `max_flows`,
  `max_flows_per_peer`, `queue_depth` (#88, #89).
- **Windows:** release archives for x64 and ARM64 (`.zip`), CI coverage, user
  services through Task Scheduler, and config/state under `%LOCALAPPDATA%`
  (#84, #92, #124).
- **macOS arm64 role archives** for all three executables (#124).
- **Installers:** `install.sh` for Linux and macOS and `install.ps1` for
  Windows install all three executables, ask before replacing files or changing
  `PATH`, verify checksums and never touch services (#93).
- **`update`** replaces the invoked executable with the latest stable release
  or `--to VERSION`. It verifies the keyless Sigstore signature of the release
  checksums and the archive checksum, stages and renames the new file, never
  stops a running tunnel, and hands Homebrew, package and container installs
  back to their owner unless `--yes` (#95, #96).
- **Signed releases:** `checksums.txt` and `checksums-darwin.txt` are signed
  keyless with cosign and verified in the release run before publication
  (#100).
- **Role ownership:** one process per role and user, plus a machine-level lock
  between a system service and user processes on Linux and macOS. Conflicts
  exit with code `6`; `status` judges liveness from the locks and reports a
  live owner in another state directory (#87).
- **Service scopes:** `service` verbs take `--user` or `--system`; `install`
  asks an interactive non-root user, defaults to system for root and pins the
  absolute config and state paths (#90).
- **System services across the privilege boundary:** a root service trusts
  exactly one user's config and state, verifies ownership, modes and links at
  install and at every start, never writes a generated key, and asks before
  retargeting another user's service (#91).
- **`config keygen random`** prints a key and its NodeId without writing
  anything; `config show --reveal`; `--config` on every `config` subcommand
  (#99).

### Changed

- **Configuration lifecycle (#99):** `config keygen` asks before replacing a
  key (`--yes` skips; no terminal means no). `config show` hides `secret_key`
  and `relay_token`. Every write is validated, atomic, owner-only and
  serialized by a lock on `<config>.lock`; `config edit` validates on exit and
  keeps a rejected edit in `<config>.rejected`. A key generated by `run` is
  stored with mode `0600` (was `0644`).
- **TCP connection lifecycle:** each new Access client has a 30-second
  establishment deadline; dial attempts get 5 to 20 seconds; failed dials back
  off exponentially up to 30 seconds; Serve gives each target connect 10
  seconds; each side waits up to 2 seconds for the peer to acknowledge the
  last bytes, and Access reports a missing acknowledgement as peer loss.
- **Shutdown:** a stop signal starts a shared 3-second drain. A second signal
  after the grace exits with status `130`; a watchdog exits with `1` when
  shutdown has not finished 10 seconds after the grace.
- **Config and state locations:** macOS and Linux always use
  `~/.config/iroh-tunnel` and `~/.local/state/iroh-tunnel`, ignoring the XDG
  variables.
- **Packages and Homebrew:** the `it` alias is owned by the package or the
  cask, so upgrades replace it and uninstall removes it.
- **Release pipeline:** native parallel GitHub Actions builds replace
  GoReleaser; a version tag publishes the GitHub release, container images
  and Homebrew cask without a manual step; pre-release tags do not move
  `latest` or the cask.
- **CI:** PRs run lint and unit tests on Linux, macOS and Windows;
  end-to-end tests, including a self-hosted relay suite, run on every push to
  `main` and gate releases.
- **Dependencies:** iroh and iroh-relay 1.1.0 → 1.4.1 (#101).
- **Code layout:** the crate is split into the `iroh-tunnel-engine` and
  `iroh-tunnel-host` workspace packages. Executable names and the CLI are
  unchanged.
- **Documentation:** the README is a short landing page; detailed guides move
  to [`docs/`](docs/README.md).

### Removed

- The `multiplex` Access setting. Legacy values are ignored and dropped on the
  next application save.

### Fixed

- Integration tests compile and run on Windows; they were gated to Unix.
- A refused or timed-out target connect closes only its channel.
- Access invalidates only the failed connection and keeps dialing off the
  shared state lock.
- A UDP flow keeps its reply path after an ICMP error from the target,
  including on Windows (#88).
- Systemd units keep `$` literal in `Environment=` and quote paths with
  spaces (#90).

### Security

- Debug output of configuration types redacts `secret_key` and
  `relay_token`.
- Config parse errors report the line number without quoting the line.
- A root service started by systemd or launchd without a trust pin is
  refused (#91).

## [0.4.2] - 2026-08-26

### Changed

- **Dependencies**: iroh 1.0.0 → **1.1.0** (pulling iroh-relay/iroh-base
  1.1.0, noq 1.2.0; MSRV unchanged at 1.91). The tunnel protocol and
  config surface are unchanged — a 0.4.2 node interoperates with older
  iroh-tunnel peers (mixed 0.4.2 ↔ 0.4.1 verified in both role pairings;
  older versions share the same wire protocol) and with self-hosted
  iroh-relay 1.0.x/1.1.x. See the
  ["Version compatibility" section of the 0.4.2 README](https://github.com/naicoi92/iroh-tunnel/blob/v0.4.2/README.md#version-compatibility)
  for the full matrix and wire-level analysis (SNI removal, `CustomAddr`
  serialization, pkarr discovery).
- **Docs, self-hosted relay**: installer default pin bumped to **v1.1.0** and the
  guide's fact table re-verified against the v1.1.0 crate source (ports,
  bind defaults, 15 s WS-ping, `/healthz`, access control, asset names —
  all unchanged from v1.0.3).

### Added

- **README "Version compatibility" section**: peer/relay compatibility
  matrix for the iroh 1.1.0 upgrade, plus the operational improvements it
  brings (relay rate-limit warnings, priority messages during relay
  reconnect backoff, net-report probes via proxy).

### Fixed

- **Docker demo image builds again**: the source-build `Dockerfile` had
  silently broken when the crate gained its library target and `build.rs` —
  the dummy dependency-cache layer no longer matched the crate's targets
  (it failed on the missing `src/lib.rs`, and after compensation the
  post-swap rebuild failed with `APP_VERSION` undefined at compile time),
  and the builder/runtime glibc pairing no longer matched
  (`rust:1.91-slim` is now Debian 13). Replaced the fragile cache trick
  with a straight copy-and-build and pinned the builder to
  `rust:1.91-slim-bookworm` to match the `distroless/cc-debian12` runtime.
  Release images (`Dockerfile.goreleaser`, prebuilt binaries) were never
  affected.

## [0.4.1] - 2026-08-21

### Added

- **Serve connect lines carry the peer's path**: `peer connected via
  relay=<url>` / `direct=<addr>` (symmetric with the access role's
  established lines); `via paths pending` right after a handshake.

## [0.4.0] - 2026-08-21

### Added

- **Access status file**: `access run` writes `access-status.json` beside
  the serve file (same atomic write, same 5 s change-detect flush) —
  `node_id`, `pid`, `started_at`, and one row per configured service with
  its `listen_addr`, the configured serve `peer`, the live `transports` of
  its multiplexed connection (empty while not connected, always empty for
  `multiplex = false` services), and the access endpoint's
  `local_bound_addrs` candidates.

- **`iroh-tunnel <role> status`**: new subcommand for both roles — prints a
  human-readable table (node header; on serve an exposed-services table
  plus per-connection rows, on access per-service rows; transports with
  `[active]` markers) from the role's status file; `--json` prints the file
  verbatim (validated first — a corrupt file fails loudly). A missing file
  exits 1 with `serve is not running (no serve-status.json found at
  <path>)`-shaped guidance.

- **Connection path observability in the serve status file**: the status
  file now carries a `connections` array — one entry per connected peer
  with its full node id, the services (ALPN-demuxed) it is tunneling, the
  transports iroh currently uses for it (`kind` `relay`/`direct`, `addr`,
  `active` — iroh negotiates relay and direct paths concurrently, so a peer
  can be direct-only, relay-only, or have both active at once), and the
  serve endpoint's local UDP socket candidates (`local_bound_addrs`,
  endpoint-wide — not a per-transport local address). Refreshed by the 5 s
  flush task, rewritten only when the rendered snapshot changes; a peer
  disappears once its last connection closes.

- **Access connection-path logging + path-change events** (access role):
  connection-established lines (multiplexed and per-channel) now render the
  serve peer's active transports (`relay=<url>` / `direct=<addr>`,
  comma-separated) — full peer id in the `peer=` field for cross-host
  correlation, short id in the message. Each live multiplexed connection
  gets a background poller (5 s cadence, bounded by the connection's
  lifetime — it never keeps the connection alive) that logs a single
  `path changed` line when iroh migrates paths, e.g. `relay→direct` after
  a successful hole punch or `direct→relay` when the direct path dies. A
  new "Log events" glossary in the README documents every line and the
  relay/direct semantics.

- **`IROH_TUNNEL_STATE_DIR`**: environment override for the directory the
  status file is written to (advanced/testing seam — the file lands
  directly in it, no `iroh-tunnel` subpath appended).

### Changed

- **Breaking**: the serve status file was renamed `status.json` →
  `serve-status.json` (role-scoped, preparing for an access-side status
  file). Monitoring that reads the old path must update. On the first
  successful write of the new file, a pre-rename `status.json` in the same
  directory is removed best-effort, so upgraded nodes never leave a stale
  snapshot behind for old-path tooling to read silently.

## [0.3.0] - 2026-08-20

Self-hosted relay support: deploy guide + installer, relay authentication,
per-service relay overrides.

### Added

- **`relay_token` (both roles)**: `[node] relay_token` authenticates to
  relays that enforce `access.shared_token` — sent as
  `Authorization: Bearer` on the relay connection, applied to every relay
  in `relay_urls` (home + failover). Single-token semantics: mixing relays
  with different tokens is not supported; relays without access control
  ignore the extra header. Validated non-empty/no-whitespace; never logged
  or shown in status output.

- **Per-service relay override (access)**: `[[services]] relay_urls` dials
  that service's serve peer through its own relay set instead of
  `[node] relay_urls` (absent/empty keeps the node fallback, then the n0
  defaults). The access endpoint joins the union of node + service relays
  so every dial has a live relay transport; each service's dialer attaches
  only its own URLs. The serve peer must itself be registered on those
  relays. Serve deliberately has no per-service relays — one endpoint, one
  relay mode.

- **Self-hosted relay deployment guide + installer**
  (`docs/self-hosted-relay.md`, `docs/install-relay-debian.sh`): run your
  own iroh-relay on a Debian 12/13 LXC with a public IP — Caddy ACME TLS
  terminator, loopback binds, systemd + hang-guard timer, optional
  `--enable-token` / `--enable-quic` (QUIC address discovery reusing
  Caddy's cert with automatic reload), sha256-verified release download.
  Facts verified against iroh-relay v1.0.3 source.

### Changed

- **Docs**: README rewritten as a full OSS landing page — badges, table of
  contents, comparison table (ngrok/cloudflared/Tailscale/frp/bore), use
  cases, FAQ/troubleshooting, security notes, acknowledgments. Claims
  corrected to TCP-only while UDP framing is not yet wired into the run
  path. Added CONTRIBUTING.md and SECURITY.md.

## [0.2.0] - 2026-08-19

**Multiplexing: one connection per service, one stream per channel.**

Minor — non-breaking **with the correct rollout order (serve first, then
access)**.

**Rollout contract (important)**

There is deliberately **no protocol negotiation** — the ALPN is unchanged
(`iroh-tunnel/{name}`). Compatibility is managed by upgrade order:

|                | Serve < 0.2.0 (1 stream/conn) | Serve ≥ 0.2.0 (accept loop) |
|----------------|-------------------------------|------------------------------|
| Access < 0.2.0 | as before                     | as before                    |
| Access ≥ 0.2.0, `multiplex = true` (default) | **not supported** — a second stream would hang until the connection closes | multiplexed |
| Access ≥ 0.2.0, `multiplex = false` | as before (per-channel) | as before (per-channel) |

**Upgrade serve nodes first** — a 0.2.0 serve is fully backward-compatible
with every access version, so the serve side has no caveats at all. If an
access node must talk to a serve peer that is not yet upgraded, set
`multiplex = false` on that service.

### Added

- **Multi-stream serve**: the serve role now serves every bidirectional
  stream on each connection (an `accept_bi` loop per connection), each piped
  independently to the local service. Previously each connection carried
  exactly one stream. Stream failures — including a refused local dial —
  reset only that stream; the connection and its other streams are
  unaffected. Per-connection debug logs number the accepted streams.

- **Multiplexing on the access side** with per-service config
  (`multiplex`, default `true`): the service keeps ONE long-lived Iroh
  connection (dialed via the usual retry/backoff loop) and opens one QUIC
  bidirectional stream per local TCP connection. Handshakes are paid once.
  A dead connection surfaces as EOF on its channels; the next channel dials
  a fresh one. `multiplex = false` is the pre-0.2.0 behavior verbatim.

- **Transport tuning**: both roles pin a 5 s QUIC keep-alive (iroh's own
  default, now explicit — long-lived connections do not depend on it
  silently) and otherwise keep noq's own defaults, including the
  concurrent-bidi-stream budget (100 per connection). Nodes that need more
  can override it after measuring their real concurrent-channel count:
  `[node] max_concurrent_streams = N` (both roles; validated ≥ 1). Worst-case
  buffer memory scales with `max_concurrent_streams ×
  stream_receive_window`, and when all slots are busy a new channel's
  `open_bi` is flow-control blocked until another stream closes.

- **Status file**: `active_connections` in `status.json` now counts active
  **streams** (in-flight pipes), refreshed by a 5 s flush task that only
  rewrites on change. With multiplexing one connection carries many
  channels, so streams are the operator-meaningful number.

- **BusyBox / SysV init service backend** (non-systemd Linux): `service`
  subcommands now detect at runtime whether the host runs systemd
  (`/run/systemd/system`, fallback `/proc/1/exe`) and, when it does not,
  install an `/etc/init.d/S96iroh-tunnel-<role>` script instead of a systemd
  unit — covering buildroot/BusyBox embedded devices such as the Sipeed
  NanoKVM. The script is dependency-lean (plain `sh`, PID file, no
  `start-stop-daemon`), starts after networking (boot order S96), stops with
  SIGTERM + a 5 s grace window before SIGKILL, and both scopes map to the
  system-wide `/etc/init.d` (BusyBox has no per-user services). Requires
  root.

### Changed

- **CI**: sccache (`RUSTC_WRAPPER`) enabled on lint/test/build jobs to cache
  rustc output across runs on top of `Swatinem/rust-cache`.
- **Releases**: GoReleaser `draft: false` on both configs — a pushed tag now
  publishes immediately instead of waiting for a manual publish step.
- Dependency bumps: clap 4.6, tokio 1.53, toml 1.1, toml_edit 0.25, dirs 6,
  which 8, data-encoding 2.11, regex 1.13.

### Fixed

- Corrected a stale comment claiming `Endpoint::connect` reuses existing
  connections: in iroh 1.0 every `connect` is a fresh QUIC connection —
  which is precisely the per-channel handshake cost multiplexing removes.

## [0.1.4] - 2026-08

riscv64 (musl) release artifacts and build pipeline.

## [0.1.0] - 2026-07

First stable release: serve/access TCP tunneling, config CLI, systemd/
launchd service management, status file, GoReleaser packaging.

[0.5.0]: https://github.com/naicoi92/iroh-tunnel/compare/v0.4.2...v0.5.0
[0.4.2]: https://github.com/naicoi92/iroh-tunnel/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/naicoi92/iroh-tunnel/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/naicoi92/iroh-tunnel/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/naicoi92/iroh-tunnel/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/naicoi92/iroh-tunnel/compare/v0.1.4...v0.2.0
[0.1.4]: https://github.com/naicoi92/iroh-tunnel/compare/v0.1.0...v0.1.4
[0.1.0]: https://github.com/naicoi92/iroh-tunnel/releases/tag/v0.1.0
