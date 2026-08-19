# Changelog

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

- **Transport tuning, explicit**: both roles set
  `max_concurrent_bidi_streams = 256` and a 5 s QUIC keep-alive (iroh's
  own default, now pinned — long-lived connections do not depend on it
  silently). 256 is headroom, not a requirement — tune to your real
  concurrent-channel count. Worst-case buffer memory scales with
  `max_concurrent_bidi_streams × stream_receive_window`, and when all
  slots are busy a new channel's `open_bi` is flow-control blocked until
  another stream closes.

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
