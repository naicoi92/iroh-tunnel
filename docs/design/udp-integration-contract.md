# UDP integration contract (#78)

Single reference for implementation issues #88 and #89. It consolidates decisions already recorded in `redesign-spec.md` (UDP-01 to UDP-06); it adds only the wire framing and the admission-refusal behavior chosen on #78. UDP is **experimental** (#73 decision): behavior under loss, delay, congestion and path migration is not verified.

## Decision sources

| Topic | Rule | Issue |
|---|---|---|
| Boundaries, best effort, client routing | UDP-01, UDP-02, UDP-03 (Confirmed) | #68, #70 |
| Flow identity, filtering, lifecycle, quarantine | UDP-04 (Confirmed) | #71 |
| Transport, oversize, datagram limit, flow identifier | UDP-05 (Confirmed) | #72, #73 |
| Caps, admission, timeouts, queues, diagnostics | UDP-06 (Confirmed; defaults are untested estimates) | #74 |

## Transport and wire framing

- Transport is QUIC DATAGRAM on the existing per-service Iroh connection. There is no stream fallback and no fragmentation.
- Each tunnel datagram is `[flow id: u32, big-endian][client payload]`. The payload is opaque and may be empty.
- The flow identifier is 32 bits, assigned by Access only after a flow is admitted, scoped to the session incarnation and never reused within it.
- A datagram whose flow identifier Serve has not seen asks Serve to admit a new flow: Serve applies the caps and, if admitted, binds a new connected upstream socket to the fixed target.
- The datagram limit is the connection's current maximum datagram size minus 4. It is read per send and never cached as a constant.

## Failure contract

| Situation | Behavior | Observable |
|---|---|---|
| Payload larger than the datagram limit | Access drops it; never delivered in part | Counter `oversize`, rate-limited log |
| Datagrams unusable on the connection | Session fails at setup with a clear error | Error at setup |
| Limit shrinks mid-session | Affected datagrams dropped individually | Counter `limit_shrink`, rate-limited log |
| Service or per-peer cap reached | Serve refuses the flow and drops the datagram; no existing flow is evicted | Counter `cap` |
| Serve refused a flow | Access keeps the flow until idle expiry; later datagrams are each dropped and counted by Serve. No refusal signal is sent on the wire | Counter `cap` |
| Per-flow queue full | Newest datagram dropped | Counter `queue_full` |
| Reply from a source other than the target | Dropped (connected socket) | Counter `foreign_source` where the platform exposes it |
| Idle expiry | Flow ends; upstream socket stays open as a sink for the quarantine period, then closes | Late replies counted as `quarantine_reply` |
| Peer lost or reconnect | All flows end with the incarnation; quarantined sockets remain as sinks only | No flow survives |
| Flow identifiers exhausted | Session incarnation ends and reconnects | Log; all flows end |
| Bind or descriptor exhaustion | Flow refused, process keeps running | Counter `cap` |

Residual risk: after quarantine, a late reply on a reused tuple may reach a new flow. The tunnel documents this and does not claim absolute isolation (UDP-04).

## Defaults (untested estimates)

Idle timeout 60 s (configurable); quarantine 30 s (fixed); 256 flows per service; 64 flows per peer. Changing a value does not change the contract. New config fields need explicit owner approval before implementation.

## Left to implementation

- Access cap: default, what it counts, test case. Owner: #89.
- Per-flow queue depth default. Owner: #89.
- Keep-alive or refresh semantics beyond UDP-04's rule that a datagram in either direction refreshes the idle timer.

## Evidence and known risk

| Evidence | Status |
|---|---|
| `examples/udp_bench_probe.rs`, `examples/udp_bench.rs` and `issue-73-evidence.md`: loopback runs on forced relay and verified direct path, limit observed to change (1162 and 1414), oversize refused synchronously | Available |
| Loss, delay, congestion, path migration, long-run memory (E3), flow routing through a real tunnel | NOT RUN. Accepted risk of experimental status. A `tc netem` environment is unconfirmed (a hosted Linux runner may or may not allow it; not checked) |
| Direct and forced-relay measurements through the real tunnel, per platform | Stay with UDP-PATH-01, #89 and final gate #97 |

This document does not claim the #73 benchmark acceptance was met.

## Traceability to implementation

| Behavior | Owner | Acceptance case |
|---|---|---|
| Opaque payload, boundaries, interleaved replies, equal flow ids across peers | #88 | UDP-DATA-01 |
| Upstream socket per flow, shared connection, documented NAT and best effort | #88 | UDP-SOCKET-01 |
| Expiry, quarantine, reconnect, source filtering, tuple reuse | #89 | UDP-LIFECYCLE-01 |
| Caps, admission, queues, oversize, descriptor exhaustion, diagnostics | #89 | UDP-LIMIT-01 |
| Direct and relay paths, impairment, platform matrix | #89, #97 | UDP-PATH-01 |
