# Issue #73 evidence — UDP payload size and tunnel transport policy

Baseline `3900e2b` (main). Iroh locked at 1.4.1 (`Cargo.lock`); issue #72 still says 1.1.0, which is stale. Blocker #72 is closed.

**Status: no benchmark has been run. No transport is chosen.** Results below are empty on purpose; do not fill them with estimates.

## Policy principles (proposed, owner-confirmed in discussion; see spec UDP-05)

1. No fragmentation or reassembly in the first release.
2. No fallback to reliable ordered streams, automatic or configured.
3. Oversized datagram: drop, count, report via diagnostics. Never deliver a partial datagram as complete.
4. Datagrams unusable on the connection: fail at session setup with a clear error.
5. Datagram limit shrinks mid-session: drop affected datagrams individually, count, rate-limit logs.
6. Datagram limit is dynamic: connection maximum datagram size minus tunnel framing. Documentation may cite a conservative reference value.
7. Flow multiplexing by a short Access-assigned flow identifier, scoped to the session incarnation, not reused within it.
8. Numeric resource bounds, timeout and quarantine values: deferred to #74.

## Candidates

| Candidate | Role |
|---|---|
| QUIC DATAGRAM (unreliable, unordered, no frame fragmentation) | Candidate |
| Framed reliable stream | Reference baseline only; one short run, not selectable |

## Exclusion criteria (correctness only)

A candidate is excluded if any of these occurs in any scenario:

- E1: a partial or corrupted datagram is delivered as complete;
- E2: a datagram is delivered to a different flow than its origin (cross-delivery);
- E3: memory, queue or handle use is not bounded under flow-count or payload stress;
- E4: datagram boundaries are not preserved for delivered datagrams.

Performance metrics are reported, not thresholded.

## Benchmark matrix

| Dimension | Values |
|---|---|
| Path | direct; forced relay. Selected path verified and recorded per run |
| Payload | 64 B, 512 B, 1200 B, datagram limit minus 1, equal to limit, limit plus 1 |
| Flows | 1, 10, 100, 1000 concurrent (upper values bounded by environment) |
| Network | clean; injected loss; injected delay and jitter; congestion |
| Path change | one direct-to-relay or relay-to-direct migration mid-run, observing limit change |
| Version | Iroh 1.4.1 (the locked version), exact commit recorded |

Metrics: end-to-end latency, jitter, loss, throughput, CPU, memory, and where each drop occurs (send queue, limit, receive side). Record methodology, environment, command lines and raw data location with every result.

## Acceptance mapping

| Check | Evidence | Status |
|---|---|---|
| E1–E4 hold for the candidate on direct and relay | Benchmark results | NOT RUN |
| Oversized datagram dropped, counted, reported | Test at limit plus 1 | NOT RUN |
| Setup failure without datagram support | Test against a peer or config without datagram support | NOT RUN |
| Mid-session limit shrink handled | Path-change run | NOT RUN |
| Owner approves transport | Comment on #73 | PENDING |

## Results

None yet.

## Follow-ups after approval

Promote UDP-05 to Confirmed in the spec, add an ADR for the transport choice (hard to reverse, surprising, real trade-off), update acceptance case UDP-LIMIT-01, and hand numeric bounds to #74.
