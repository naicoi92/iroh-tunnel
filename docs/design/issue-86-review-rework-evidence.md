# Issue #86 — independent-review rework

Baseline `d01ad2906b418e7e959f516a6cfdc3593e578ac1`. Live #86 body,
comments and native blockers refreshed before edits; #76/#81 closed. User
review-rework instruction supplies exact scope; prior numeric defaults remain
technical choices, not earlier tracker approval. Read tracked glossary, spec,
plan, decisions, acceptance, workflow, Rust standards and supplied TDD and
test-from-spec skills. CodeGraph tools absent from worker allowlist; inspected
named seams directly. No delegation, public-relay test, PR/push/merge.

## Acceptance matrix (before edits)

| Criterion / source | Test seam and independent expected result |
|---|---|
| RW-1: P1 Standards finding / user task: peer STOP_SENDING is failure, not acknowledged delivery | `access_reports_peer_stop_as_tunnel_error`: actual Access child, real TCP client and Iroh peer on local relay; peer reads literal prefix, sends STOP_SENDING code 42 and FIN; client ends request without more bytes; Access must log tunnel error with peer-stop code, not normal closure. Exercises real transport, no mocked stopped result. |
| RW-2: TCP-FAILURE-01 / P1 Spec finding: kill during marked transfer, bounded no replay on recovery | Strengthen `configured_peer_loss_closes_active_channels_without_replay`: target confirms unique prefix of unfinished logical request, remainder held by client until after Serve killed/reaped; client EOF/error; same identity restart; distinct new marker/reply, no old bytes or extra target accepts for 2 s. No exactly-once or arbitrary-future claim. |
| RW-3: P2 Standards finding / user task: configured process fixture handles bind races only, reaps failed children, checks actual address | Shared bounded collision retry for Access starts in `tests/configured_multiplex.rs`; `configured_listener_retries_confirmed_bind_collision` holds first candidate port, checks collision log and reaped child then real TCP exchange on replacement listener. Retry only bind failure plus platform address-in-use code; unrelated failures propagate. |
| RW-4: user task: publish retry and shutdown defaults | Unreleased changelog inspection: 30 s total client establishment, 5 s per dial, exponential backoff, 3 s shared shutdown grace; no healthy-channel timeout or replay; endpoint close may add transport teardown time. Documentation-only criterion. |

Applicable Rust rules: RS-01 minimal cohesive seams; RS-05 actionable error;
RS-08/09 owned cleanup and bounded retry/drain; RS-10 secret hygiene;
RS-13 local relay only and platform disclosure; RS-14/15 observable real
transport/process regressions; RS-16 locked MSRV gates. Independent exact-head
Standards/Spec reviews remain parent-owned and required.

## Results

Pending; each verified slice and final gates recorded below.

### RW-1 verified slice

Real STOP_SENDING RED: `/tmp/86-rework-stop-red.log`, focused test failed
because Access logged `DEBUG tunnel closed` after peer code 42. No fake
transport or result mapping. Minimal fix matches `None` vs `Some(code)`;
transport failure retains context. GREEN: `/tmp/86-rework-stop-green.log`,
1 passed (4.97 s), same command:
`cargo +1.91.1 test --all-features --locked --test configured_multiplex access_reports_peer_stop -- --nocapture`.
`cargo +1.91.1 fmt --all` and `git diff --check` passed. macOS ARM64,
Rust 1.91.1; local relay only. Full gates pending after all slices.

### RW-2 verified slice

Strengthened real-process peer-loss test passed on unchanged production,
`cargo +1.91.1 test --all-features --locked --test configured_multiplex configured_peer_loss -- --nocapture`,
1 passed in 44.02 s; `/tmp/86-rework-inflight-green.log`. No behavioral RED
claimed: this closes missing scenario coverage. Target receives literal
`old-first-marker`, then bounded read confirms request is still open with no
remaining part. Client holds `old-first-request-remainder` and FIN until
Serve kill/reap; other channel confirms shared connection remains active.
After reaping, client attempts remainder/FIN (local write success is not
remote delivery), both clients end and old target sockets close without
trailing bytes. Restart uses same configured identity, distinct
`new-client-marker` exchanges exact reply; 2 s sees no extra target accept
or unsolicited bytes/closure. No exactly-once claim or unbounded absence of
replay. fmt and diff check passed. This supersedes prior idle-channel-only
coverage limit, not platform/release limits.

### RW-3 / RW-4 verified slice

Deterministic collision RED `/tmp/86-rework-collision-red.log`: occupied first
port caused actual Access child `failed to bind ... Address already in use
(os error 48)`, no retry, test failed. GREEN
`/tmp/86-rework-collision-green.log`: failed child killed and `try_wait` reaped
before same-identity replacement, new listener exact overlapping TCP exchanges,
occupied socket saw no readiness probe; 1 passed, 4.09 s. Command:
`cargo +1.91.1 test --all-features --locked --test configured_multiplex configured_listener_retries -- --nocapture`.

All Access starts in configured fixture now use shared eight-attempt helper.
Only same-line bind operation/address plus known platform address-in-use code
triggers retry. Other readiness errors propagate. Failed child is killed/reaped
with 5 s bound before next candidate; replacement readiness requires exact
`listening for local clients listen_addr=IP:PORT`, then callers use returned
address. Port-only TOML edit preserves legacy field under test. Original
legacy-file expectation models only fixture port edits, never rereads app
output as expected value. No production CLI/config port-zero policy change.

Default-parallel configured suite passed 16/16 in 47.20 s,
`/tmp/86-rework-configured-green.log`. After strengthening legacy-source
expectation, `configured_` filter passed 5/5 in 44.11 s,
`/tmp/86-rework-collision-variants.log`. fmt/diff check passed. Unreleased
changelog now documents 30 s client budget, 5 s per dial, existing exponential
backoff, 3 s shared drain and transport teardown caveat. Broader full gates
and repeated targeted runs follow on committed candidate.
