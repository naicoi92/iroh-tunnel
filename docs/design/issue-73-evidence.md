# Issue #73 evidence — UDP payload size and tunnel transport policy

Baseline `3900e2b` (main). Iroh locked at 1.4.1 (`Cargo.lock`); issue #72 still says 1.1.0, which is stale. Blocker #72 is closed.

**Status: no benchmark has been run. No transport is chosen.** Results below are empty on purpose; do not fill them with estimates.

## Policy principles (proposed, pending owner approval on #73; see spec UDP-05)

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
- E4: datagram boundaries are not preserved for delivered datagrams. A truncated datagram delivered as complete is reported under E4 (length mismatch), not E1.

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

## API probe (not a benchmark)

`examples/udp_bench_probe.rs`, Iroh 1.4.1, in-process relay (`iroh::test_utils`), loopback, one run. It only confirms the harness assumptions; it measures no latency, loss or throughput and decides nothing.

| Observation | Forced relay (`clear_ip_transports`) | Direct (after holepunch) |
|---|---|---|
| Selected path | relay | ip (relay path still open, not selected) |
| `max_datagram_size` | 1162 | 1414 |
| Sizes 64, 512, limit−1, limit | all received intact | all received intact |
| limit+1 | `send_datagram` returns `TooLarge`, nothing sent | same |
| Server-side limit | equal to client limit | equal to client limit |

Findings:

- The datagram limit read 1162 on the relay path and 1414 on the direct path in the probe. The self-check below shows that is not a per-path constant: a relay connection later read 1414 as well, so the first reading reflects the moment of measurement. Either way the limit is dynamic, which supports UDP-05 principle 6; a fixed value would be wrong at some point.
- Oversize is refused synchronously by the sender, so drop-and-count (principle 3) needs no wire-level detection.
- The direct path is not selected immediately after connect; the harness must wait for `is_ip() && is_selected()` and record the selected path per run.
- Not yet observed: limit change on migration mid-session, loss behavior, queue drops (`datagram_send_buffer_space`).
- Loopback only: numbers do not represent real networks.

## Harness

`examples/udp_bench.rs` (not shipped). Two in-process endpoints and an in-process relay, Iroh 1.4.1. Prints one JSON object per run.

```sh
cargo run --release --example udp_bench --features test-utils --locked -- \
  --path relay|direct --transport datagram|stream \
  --payload <bytes>|limit-1|limit|limit+1 --flows N --count M --pps P --note "<condition>"
```

What it checks and reports per run:

- Path: `relay` removes IP transports; `direct` waits up to 10 s for a selected IP path and fails otherwise. Selected path and datagram limit are recorded before and after the run (path migration shows as a difference).
- E1/E2: each message carries flow id, sequence and a body derived from both; a mismatch counts under `E1_E2_corrupt_or_inconsistent`. Limitation: there is no tunnel here, so this proves the transport does not mix or damage datagrams, not that the future flow table routes correctly.
- E4: delivered length differs from sent length.
- Oversize: sends are counted as `refused_too_large`, not lost.
- Loss, reordering, duplication, latency p50/p95/p99/max, mean absolute latency delta (jitter), throughput of unique messages.
- Process RSS peak and CPU time from `ps`. Both endpoints share one process, so these are upper-level indicators, not per-role figures. E3 (bounded memory) needs a long run with many flows; one run does not prove it.

Not covered, by design: loss, delay and congestion are injected outside the process (for example macOS `dnctl`/`pfctl` or Linux `tc netem`), and the operator records the condition in `--note`. Path migration needs the operator to change the network mid-run. Neither has been run.

### Harness self-check (not benchmark results)

Loopback, macOS, one run each, head after review fixes. Raw JSON of the four runs: `issue-73-harness-selfcheck.jsonl` (one object per line, in the order below). Smoke runs only: they show the harness works and decide nothing.

| Run | Result |
|---|---|
| relay, datagram, payload = limit (1162), 4 flows x 100 | 400/400 received, E1/E2/E4 = 0 |
| direct, datagram, payload = limit+1 (1415 vs 1414), 2 x 20 | 40 refused `TooLarge`, 0 sent, 0 delivered |
| direct, datagram, payload = limit (1414), 10 x 100 | 1000/1000 received, E1/E2/E4 = 0 |
| relay, stream, 512 B, 4 x 100 | 400/400 received, E1/E2/E4 = 0 |

Observations:

- The datagram limit is not fixed within a connection. In the first run the path stayed on the relay, yet the limit read 1162 before and 1414 after the run (the stream run: 1288 to 1414). The limit grows while the connection warms up (path MTU discovery is the probable cause; not verified here). A payload chosen as `limit` at start can therefore sit below the limit later, and any fixed limit would be wrong in both directions. Benchmark runs must record the limit before and after, and should wait for it to settle or state that they did not.
- In the probe, the direct path was selected 110 ms after connect on loopback. Real networks will take longer; the harness waits up to 10 s.
- An earlier harness version hung when a sender sent nothing (all oversize); receivers now stop when senders are done.

### Known limits of the harness

- `lost` counts every datagram that did not arrive. `send_datagram` returning `Ok` can still drop locally when the send buffer is full, so local queue drops and network loss are not separated.
- E3 is only sampled through process RSS (`ps`, Unix only, both endpoints in one process). It does not prove bounded memory.
- Run end is decided by `--idle-end-ms` (default 2000). With injected delay near or above it, late datagrams are counted lost; raise it.
- Reordering is counted per flow by sequence; a duplicate is counted only as duplicated.
- Jitter is the mean absolute difference between consecutive arrivals of the same flow.
- Loss, delay, congestion and mid-run path migration are not exercised: the current environment cannot inject network impairment. Those rows of the matrix stay NOT RUN and block the decision on #73.
