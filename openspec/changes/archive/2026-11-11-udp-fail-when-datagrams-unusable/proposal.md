## Why

A UDP service whose connection cannot carry QUIC datagrams fails silently today: Access and Serve drop every datagram, count it as `limit_shrink` and log a warning, so the operator sees no error (issue #152). The integration contract already promises a clear setup failure, and the spec records the gap as a deviation. Closing it now keeps a future config change or a foreign peer on the same ALPN from draining traffic unnoticed, and restores `limit_shrink` to its mid-session meaning.

## What Changes

**Connection that cannot carry datagrams**
- From: every datagram on the connection is dropped and counted as `limit_shrink`, with a warning log and no setup error.
- To: right after the handshake of a UDP service connection, each side checks whether datagrams are usable. If not, it closes that one connection with a dedicated application close code and reason, and logs an error naming the service and peer. The side that receives this close logs the cause too. The role process and its other services keep running.
- Reason: make the failure visible at setup, as the integration contract specifies.
- Impact: non-breaking between current iroh-tunnel binaries, which always enable datagrams. Only peers that cannot carry datagrams see different behavior.

**Access after such a failure**
- From: the next local datagram redials immediately.
- To: Access waits a fixed 30 seconds before dialing that service again. Client datagrams that arrive while it waits are dropped with a rate-limited log and are not counted in any drop counter.
- Reason: no redial storm against a peer that cannot carry datagrams.
- Impact: non-breaking; only affects services that hit this failure.

**Meaning of `limit_shrink`**
- From: also counts every datagram of a connection whose datagrams are never usable.
- To: counts only datagrams dropped because the limit became unusable mid-session. A payload over a smaller but still usable limit stays `oversize`, as today.
- Reason: the never-usable case is now a setup failure.
- Impact: counter semantics narrow; the status schema is unchanged.

Docs: the "Datagrams unusable on the connection" row of `docs/udp-integration-contract.md` and the `limit_shrink` rustdoc in the engine and host crates describe the implemented behavior instead of the deviation.

Non-goals: no status file field and no host API change; no wire-format or protocol message; no change to TCP services or to the Access dialer shared by TCP and UDP; no detection of datagrams blocked on the network path while the peer advertises support; no configurable back-off.

## Capabilities

### New Capabilities

None.

### Modified Capabilities
- `udp-datagram-forwarding`: the requirement "Unusable datagrams are dropped, not failed at setup" is replaced by a requirement that a UDP connection which cannot carry datagrams is closed at setup with a clear error on both sides, that Access backs off before redialing, and that `limit_shrink` counts mid-session drops only.
- `workspace-boundaries`: the requirement "Drop counter docs expose the unusable-datagram deviation" changes to require that the `limit_shrink` rustdoc describes mid-session drops only and no longer refers to the #152 deviation; a requirement is added that the datagram-disable test seam exists only under `test-utils`.

## Impact

- Code: `crates/engine/src/udp/` (usability classification, close code), `crates/engine/src/serve/run_serve.rs` (`handle_udp_connection`), `crates/engine/src/access/run_access.rs` (`udp_listen_loop` back-off), `crates/engine/src/endpoint/create.rs` (test-utils-only seam to disable datagrams), `limit_shrink` rustdoc in `crates/engine/src/observation/types.rs` and `crates/host/src/status/schema.rs`.
- Tests: engine unit tests for the classification; a real-binary end-to-end test with datagrams disabled on one side.
- Docs: `docs/udp-integration-contract.md`.
- API: no public API, CLI, config or on-disk format change; status schema and goldens unchanged.
- Security: the test seam that disables datagrams compiles only with the `test-utils` feature, which must never ship in production builds. A foreign peer can at most get its own connection closed; it cannot stop the process or other services.
- Platforms: behavior is platform-independent; the end-to-end test runs on Linux, macOS and Windows CI.
