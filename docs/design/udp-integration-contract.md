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
- The flow identifier is 32 bits, scoped to the session incarnation and never reused within it. Access assigns identifiers in increasing order. "Admitted" on Access means it passed the Access-local cap; Serve's decision is not signalled, so a flow Serve refuses still consumes an identifier and an Access slot until idle expiry.
- Serve keys its flow table by (peer, flow id), so equal identifiers from two peers are distinct flows.
- Serve admits an identifier only if it is higher than every identifier it has already seen from that peer in this incarnation. An identifier at or below that mark with no live flow (expired, quarantined or earlier refused) is dropped and counted as `unknown_flow`; it is never rebound. This holds even if Access and Serve idle timers drift apart.
- A datagram carrying a higher identifier than any seen asks Serve to admit a new flow: Serve applies the caps and, if admitted, binds a new connected upstream socket to the fixed target. Admission is decided once, on that first datagram.
- The datagram limit is the connection's current maximum datagram size minus 4. It is read per send and never cached as a constant. If the maximum is 4 or less, datagrams count as unusable.
- Replies from Serve to Access use the same header. An empty payload is valid in both directions and becomes a zero-length UDP datagram, not an ignored one.

## Failure contract

| Situation | Behavior | Observable |
|---|---|---|
| Client payload larger than the datagram limit | Access drops it; never delivered in part | Counter `oversize`, rate-limited log |
| Target reply larger than the datagram limit | Serve drops it; never delivered in part | Counter `oversize`, rate-limited log |
| Datagram shorter than the 4-byte header | Dropped by the receiver | Counter `malformed` |
| Identifier not admittable or not live (see above), on either side | Dropped | Counter `unknown_flow` |
| Upstream send error on Serve | Datagram dropped; the flow stays | Counter `upstream_error` |
| Datagrams unusable on the connection | Session fails at setup with a clear error | Error at setup |
| Limit shrinks mid-session | Affected datagrams dropped individually | Counter `limit_shrink`, rate-limited log |
| Service or per-peer cap reached | Serve refuses the flow and drops the datagram; no existing flow is evicted | Counter `cap` |
| Serve refused a flow | Access keeps the flow until idle expiry. Later datagrams of that identifier are dropped as `unknown_flow`, never admitted mid-flow. No refusal signal is sent on the wire | Counter `unknown_flow` |
| Per-flow queue full | Newest datagram dropped | Counter `queue_full` |
| Reply from a source other than the target | Dropped by the operating system, because the upstream socket is connected | Counter `foreign_source` exists but is not incremented: the drop happens below the tunnel and is not observable |
| Idle expiry | Flow ends; upstream socket stays open as a sink for the quarantine period, then closes | Late replies counted as `quarantine_reply` |
| Peer lost or reconnect | All flows end with the incarnation; quarantined sockets remain as sinks only | No flow survives |
| Flow identifiers exhausted | Access evicts the connection; the next datagram dials afresh with an empty flow table, so all flows end | Counter `cap`, warning log |
| Connection lost | Access evicts the dead connection on the first send error; the next datagram dials afresh | None; forwarding resumes |
| Bind or descriptor exhaustion | Flow refused, process keeps running | Counter `cap` (shared with the cap rows; the log line names the cause) |
| Access cap reached | Access drops the new client's datagram before it reaches Serve; known clients and running flows are untouched | Counter `cap` on the Access status |

Counter names here are the contract names; the spec's prose names (queue full, limit shrink) map to `queue_full` and `limit_shrink`. Quarantined sockets are reported as a count separate from active flows.

Residual risk: after quarantine, a late reply on a reused tuple may reach a new flow. The tunnel documents this and does not claim absolute isolation (UDP-04).

## Defaults (untested estimates)

Idle timeout 60 s (`idle_timeout`, seconds); quarantine 30 s (fixed constant); 256 flows per service (`max_flows`); 64 flows per peer (`max_flows_per_peer`, Serve only); 64 datagrams queued per flow (`queue_depth`). All four fields are optional on a UDP service table and rejected on a TCP service. A per-peer cap above an explicit service cap is rejected. Changing a value does not change the contract. Every value is an untested estimate.

## Left to implementation

- Nothing is left open from #89: the Access cap counts live flows of the service (default 256, same field as Serve's `max_flows`), and the queue depth default is 64 datagrams per flow.
- Keep-alive or refresh semantics beyond UDP-04's rule that a datagram in either direction refreshes the idle timer.

## Evidence and known risk

| Evidence | Status |
|---|---|
| `examples/udp_bench_probe.rs`, `examples/udp_bench.rs` and `issue-73-evidence.md`: loopback runs on forced relay and verified direct path, limit observed to change (1162 and 1414), oversize refused synchronously at the sender (not the tunnel's drop-and-count, which is NOT RUN) | Available |
| Loss, delay, congestion, path migration, long-run memory (E3), flow routing through a real tunnel | NOT RUN. Accepted risk of experimental status. A `tc netem` environment is unconfirmed (a hosted Linux runner may or may not allow it; not checked) |
| Direct and forced-relay measurements through the real tunnel, per platform | Stay with UDP-PATH-01, #89 and final gate #97 |

This document does not claim the #73 benchmark acceptance was met.

## Traceability to implementation

| Behavior | Owner | Acceptance case |
|---|---|---|
| Wire codec (header encode and decode, per-send limit read, empty payload, short datagram) | #88 | UDP-DATA-01 |
| Opaque payload, boundaries, interleaved replies, equal flow ids across peers | #88 | UDP-DATA-01 |
| Upstream socket per flow, shared connection, documented NAT and best effort | #88 | UDP-SOCKET-01 |
| Expiry, quarantine, reconnect, source filtering, tuple reuse | #89 | UDP-LIFECYCLE-01 |
| Caps, admission, queues, oversize, descriptor exhaustion, diagnostics | #89 | UDP-LIMIT-01 |
| Direct and relay paths, impairment, platform matrix | #89, #97 | UDP-PATH-01 |
