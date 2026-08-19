# Changelog

## [Unreleased]

### Added

- **Docs**: README rewritten as a full OSS landing page — badges, table of
  contents, comparison table (ngrok/cloudflared/Tailscale/frp/bore), use
  cases, FAQ/troubleshooting, security notes, acknowledgments. Claims
  corrected to TCP-only while UDP framing is not yet wired into the run
  path. Added CONTRIBUTING.md and SECURITY.md.
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

### Fixed

- Corrected a stale comment claiming `Endpoint::connect` reuses existing
  connections: in iroh 1.0 every `connect` is a fresh QUIC connection —
  which is precisely the per-channel handshake cost multiplexing removes.

## [0.1.4] — 2026-08

riscv64 (musl) release artifacts and build pipeline.

## [0.1.0] — 2026-07

First stable release: serve/access TCP tunneling, config CLI, systemd/
launchd service management, status file, GoReleaser packaging.
