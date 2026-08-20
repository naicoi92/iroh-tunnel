# Changelog

## [Unreleased]

### Added

- **Access status file**: `access run` writes `access-status.json` beside
  the serve file (same atomic write, same 5 s change-detect flush) —
  `node_id`, `pid`, `started_at`, and one row per configured service with
  its `listen_addr`, the configured serve `peer`, the live `transports` of
  its multiplexed connection (empty while not connected, always empty for
  `multiplex = false` services), and the access endpoint's
  `local_bound_addrs` candidates.

- **`iroh-tunnel <role> status`**: new subcommand for both roles — prints a
  human-readable table (node header, per-connection rows on serve /
  per-service rows on access, transports with `[active]` markers) from the
  role's status file; `--json` prints the file verbatim. A missing file
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

## [0.3.0] — 2026-08-20

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

## [0.2.0] — 2026-08-19

**Multiplexing: one connection per service, one stream per channel.**

Minor — non-breaking **with the correct rollout order (serve first, then
access)**.

### Rollout contract (important)

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

## [0.1.4] — 2026-08

riscv64 (musl) release artifacts and build pipeline.

## [0.1.0] — 2026-07

First stable release: serve/access TCP tunneling, config CLI, systemd/
launchd service management, status file, GoReleaser packaging.
