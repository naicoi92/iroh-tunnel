# Changelog

## [0.2.0] — 2026-08-19

**Multiplexing: one connection per service, one stream per channel.**
Minor, non-breaking, self-backward-compatible — mixed versions interoperate
on the first connect.

### Added

- **Multi-stream serve**: the serve role now serves every bidirectional
  stream on each connection (an `accept_bi` loop per connection), each piped
  independently to the local service. Previously each connection carried
  exactly one stream. Stream failures — including a refused local dial —
  reset only that stream; the connection and its other streams are
  unaffected.
- **Multiplexing on the access side** with per-service config:

  ```toml
  [[services]]
  # …
  multiplex = "auto"   # auto (default) | on | off
  ```

  - `auto` (default): the service keeps ONE long-lived Iroh connection and
    opens one QUIC bidirectional stream per local TCP connection. Handshakes
    are paid once. A dead connection surfaces as EOF on its channels; the
    next channel dials a fresh one.
  - `on`: multiplex only — a pre-0.2.0 serve refuses the handshake
    immediately (QUIC strict-ALPN, `NoApplicationProtocol`, ~100 ms) with a
    loud error suggesting `auto`/`off`.
  - `off`: one connection per channel, the pre-0.2.0 behavior verbatim.

- **Protocol negotiation (dual-ALPN, deliberate versioning)**: serve now
  registers two ALPNs per service — the legacy `iroh-tunnel/{name}` (fully
  compatible) and the multiplex variant `iroh-tunnel/{name}/multi`
  (registered first: rustls's server-side selection follows the server's
  registration order). Access `auto` makes ONE dial offering both ALPNs
  (RFC 7301); the negotiated ALPN decides the mode. A pre-0.2.0 serve
  negotiates legacy in the same handshake — no hang, no double dial, no
  timeout. A serve that later upgrades starts multiplexing on the next
  dial, with no fallback state on the access side. Adding the ALPN variant
  is the protocol versioning mechanism: the two modes have identical
  stream semantics, only the connection-sharing differs.

- **Transport tuning, explicit**: both roles set
  `max_concurrent_bidi_streams = 256` and a 5 s QUIC keep-alive (iroh's
  own default, now pinned). 256 is headroom, not a requirement — tune to
  your real concurrent-channel count. Worst-case buffer memory scales with
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

### Compatibility matrix

|                | Serve < 0.2.0 | Serve ≥ 0.2.0 |
|----------------|---------------|----------------|
| Access < 0.2.0 | as before     | as before      |
| Access ≥ 0.2.0 (`auto`/`on`/`off`) | `auto`: legacy per-channel | `auto`/`on`: multiplexed; `off`: per-channel |

`access on` against a serve < 0.2.0 is the only failing combination, by
design, and it fails fast with an actionable error.

## [0.1.4] — 2026-08

riscv64 (musl) release artifacts and build pipeline.

## [0.1.0] — 2026-07

First stable release: serve/access TCP tunneling, config CLI, systemd/
launchd service management, status file, GoReleaser packaging.
