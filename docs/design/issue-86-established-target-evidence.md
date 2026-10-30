# Issue #86 — established target socket failure isolation

Pinned starting head: `af42d116bf7bf6d3a35ba4254ce62cc262b6fe17` (includes
`origin/main` at `c3390ac`). Bounded slice only; not issue closure or full
TCP-FAILURE-01 acceptance.

## Contract and acceptance plan (recorded before test implementation)

Live #86 and #76 bodies/comments and native blockers refreshed before coding.
#86 blockers #76 and #81 are closed; #76 has no native blockers. #86 handoff
confirms target failure closes only affected channel, without replay. Older
spec snapshots marking isolation proposed are superseded by live decisions.
No peer-loss, retry or shutdown policy implementation belongs to this slice.

User confirmed process E2E seam. Extend `tests/quick_run.rs` using actual
`iroh-tunnel serve` / `access` children, real TCP clients/target and isolated
loopback relay. No public relay or real secrets with `test-utils`.

| Requirement | Planned observable assertion |
|---|---|
| TCP-02, AC-TCP-04/05, #86 AC2, target-failure portion of TCP-FAILURE-01 | Two overlapping clients complete distinct bidirectional exchanges before target aborts one already-established socket with zero linger (TCP reset), not failed dial or orderly EOF. Affected client reaches EOF/error within fixture deadline. |
| TCP-01, #86 AC1, supporting TCP-SHARED-01 | Healthy client remains open across reset and transfers exact fresh request/reply; Serve connection log shows one shared peer connection. |
| No silent replay | Failed channel's unique final marker reaches target once before abort; healthy/new streams carry only their own exact markers; bounded observation rejects extra target accepts or unsolicited bytes on surviving sockets. |
| Continued service (supporting) | New client completes exact request/reply after reset on same service connection. |
| RS-08/09/15 fixture cleanup | Bound scenario and closure waits; kill/reap both children with bounded polling, explicitly shut down local relay. This tests fixture cleanup, not graceful role shutdown/drain policy. |

Applicable Rust standards: RS-01 (focused names and existing fixture reuse),
RS-05 (I/O context and propagated errors), RS-08/09 (owned processes, bounded
fixture waits), RS-13 (isolated local relay/TLS boundary), RS-14 (spec-derived
observable assertions), RS-15 (actual processes), RS-16 (locked/MSRV gates).

## Results

**Existing production behavior already passes. No production change made; no
behavioral RED claimed.** Test written before inspecting/changing production.
First valid fixture run passed on unchanged starting production code.

Harness-only failures before valid run (not application regressions):

1. First compile failed with E0282 (unconstrained success type for an always-error
   quiet-window future). Changed future to return descriptive unexpected-event
   strings and supplied separate one-byte read buffers.
2. First executable attempt timed out after 45 seconds before any target socket
   established: fixture awaited target accept without sending client bytes.
   QUIC opens lazily; added exact opening markers before accept. Both child
   kill/reap checks and relay shutdown still completed after that timeout.

Commands used Rust `1.91.1-aarch64-apple-darwin` on macOS ARM64 (`Darwin arm64`):

- `cargo +1.91.1 test --all-features --locked --test quick_run established_target_reset -- --nocapture`
  - First valid run: **1 passed**, 15 filtered out, 6.04 seconds.
  - Affected client observed `Ok(0)` within 15-second fixture deadline.
  - Both initial clients completed exact bidirectional exchanges while open
    together, before zero-linger target abort. Healthy post-reset exchange and
    new-client exchange matched exact distinct literals.
  - Two-second quiet window observed no extra target connection and no trailing
    bytes/closure on healthy/new client or target sockets.
  - Serve reported one `peer connected via relay=https://127.0.0.1:56387/` and
    streams 1, 2, 3; no peer replacement. Relay port is ephemeral.
  - Both actual role children killed/reaped within five seconds each; explicit
    local relay shutdown completed within five seconds.
- Same focused command repeated **5/5 passed**, runtimes 6.05, 6.05, 6.06, 6.26,
  6.04 seconds; same EOF, connection count, exact data and cleanup outcomes.
- `cargo +1.91.1 fmt --all -- --check`: passed after formatting.
- `cargo +1.91.1 clippy --all-targets --all-features --locked -- -D warnings`: passed.
- `cargo +1.91.1 test --all-features --locked`: **144 passed, 0 failed, 5 ignored**;
  includes 111 unit tests and all 16 quick-run process tests. Ignored tests:
  authentic released-0.4 interop (one), public-relay tests (four). No ignored
  test run, no insecure test artifact contacted public relay.
- `git diff --check`: passed.

Raw local diagnostic logs (not committed): `/tmp/issue86-established-first.log`
(compile error), `/tmp/issue86-established-first-behavior.log` (fixture timeout),
`/tmp/issue86-established-valid-first.log`,
`/tmp/issue86-established-repeat-{1,2,3,4,5}.log`,
`/tmp/issue86-clippy.log`, `/tmp/issue86-full-tests.log`.

## Limits and review

Only established-socket target reset isolation characterized here. Test asserts
client EOF/error, not exact error mapping. No in-flight-reset completeness,
peer-loss, retry, drain/shutdown, 0.4 interop, production-TLS/public-relay,
forced-relay-only transport, load/resource-exhaustion or Linux/Windows runtime
claim. No-replay evidence is bounded to exact markers and two-second observation,
not mathematical proof for arbitrary future time. Fixture deadlines are test
bounds, not new application policy defaults.

Existing `Process` drop guard remains fallback on early setup failure/panic;
normal path and returned scenario errors perform explicit bounded cleanup.
Kill/reap is fixture cleanup, not graceful application lifecycle acceptance.

Sole-writer self-check: no identified Standards/Spec blocker; changes limited to
one process E2E and evidence. RS-08/09 owned sockets, timeout/cancellation and
child cleanup; RS-13 local relay only; RS-14/15 real observable spec path;
RS-16 gates run on 1.91.1. Independent Standards and Spec reviews remain required
outside this non-delegating implementation task. Repository lacks
`docs/agents/issue-tracker.md`; live issue/native dependency reads used `gh`
directly. No push, PR, merge or delegation performed.
