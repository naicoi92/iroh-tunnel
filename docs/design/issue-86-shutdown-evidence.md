# Issue #86 — owned shutdown and bounded drain

## Pre-implementation ownership plan

Baseline `3138842392960e1ab259d720effbad77821bfd13`; sole writer, branch
`redesign/issue-86-current`. Read tracked glossary, spec, plan, acceptance,
workflow, TCP decisions and prior #86 evidence; live #86/#76 bodies, comments
and native blockers refreshed. #76 and #81 closed; #86 open. Live handoff
permits numeric defaults, cancellation and repeated-stop as technical work;
no illustrative numeric policy is treated as approved.

One lifecycle invariant across four tightly coupled seams: **every admission
supervisor owns and joins its children; stop seals admission before waiting
for established pipes; all owners share one absolute drain deadline; endpoint
and auxiliary/status teardown follows pipe joins.** No detached channel tasks.

- `src/access.rs`: listeners own client JoinSets, observe normal completions,
  stop binding/accepting on watch notification, cancel pending lock/dial/open/
  backoff, then drain established pipes. Dialers retain poller/watcher handles,
  replace with abort+join and clean up only after channel joins.
- `src/serve.rs`: accept supervisor cancels pending incoming handshake; owns
  connection supervisors. Each connection seals `accept_bi` and target dial on
  stop, owns/observes stream JoinSet, drains pipes against same deadline. Root
  joins supervisors rather than aborting parents with unjoined descendants.
- `src/shutdown.rs`: shared absolute-deadline stop notification and leaf-task
  drain/abort/join behavior; remove placeholder sleep. Repeated signals do not
  restart grace or force premature closure.
- `src/role_run.rs`: remove post-endpoint placeholder/double grace; disconnect
  watcher returns future spawned into caller-owned JoinSet. Role loops finish lifecycle before returning.
- Tests: extend existing actual-process configured fixture, local loopback
  relay only. No public relay with `test-utils`. No changes to normal healthy
  established-stream deadlines, protocol, retry numbers, CLI or dependencies.

Proposed technical default for review: **3 seconds**, single monotonic budget
from stop receipt. Gives short in-flight request/reply exchanges time to finish
(the process test delays completion by 250 ms) while promptly releasing held
sockets and service resources. Not a latency guarantee or #76-approved number;
Internet/high-load tuning remains unmeasured. No extra grace after leaf drains.

## Acceptance matrix (recorded before tests/code)

Approved seam: actual Serve/Access processes + TCP clients/target + isolated
local relay (user task, RS-14/15). Each test maps to one criterion; assertions
within lifecycle test establish that single ordered lifecycle contract.

| Criterion / source | Test IDs / planned expected outcomes |
|---|---|
| AC-S1: TCP-02, #86 shutdown handoff: role stop seals admission, permits established completion, then forces remainder closed within bounded grace, followed by resource cleanup | `access_shutdown_drains_established_channels_before_forcing_remainder`, `serve_shutdown_drains_established_channels_before_forcing_remainder`: two live literal-marker streams; stop; Access bind refused or Serve new stream on existing shared connection never reaches target; after 250 ms exact final response and orderly EOF completes first; second remains usable during grace then EOF/error; process success and own status removed; listener rebind/target closure; one shared connection before stop. Test tolerance bounds 2–4 s for held close, not tracker-approved numbers. SIGTERM repeated at 250 ms and 1.5 s; latter would expose grace reset. Serve also attempts fresh peer connection; target accept remains monitored through held closure. |
| AC-S2: user task / #86 technical cancellation: stop interrupts pending establishment without consuming grace; repeated stop remains bounded without resetting deadline | `access_shutdown_cancels_pending_retry_without_waiting_for_grace`: absent peer, two concurrent clients (dial + lock wait), timed-out attempt/backoff observed, SIGTERM twice, both EOF/error <1 s; successful exit <5 s including Iroh close retransmission (upstream documents up to about 3 s for failed connectivity). Additional dial-stop variant sends stop before first timeout. Initial <2 s process bound failed despite prompt client cancellation: measured separately rather than treating QUIC close retransmission as extra application grace. |
| AC-S3: user task / #86 technical repeated stop: a repeat inside the grace is ignored and never extends the deadline; a repeat after the grace forces exit 130; a stuck shutdown is abandoned by a watchdog (exit 1) | Inside grace: same AC-S1 role scenarios send repeated SIGTERM during grace and still finish gracefully (success exit). After grace: `repeated_stop_after_grace_exits_130_and_cleans_status` (frozen Serve peer plus held channel keep shutdown unfinished; SIGTERM at 0.5 s ignored, process alive past 2 s, SIGTERM after 3.3 s exits 130, own status file removed). Watchdog: `stuck_shutdown_is_abandoned_by_watchdog_with_exit_1` (same stuck setup, watchdog extra time shortened to 300 ms through a `test-utils`-only environment seam, exit 1, status removed). Unit tests in `src/shutdown.rs` cover the same decisions with injected signals. The 130/1 decision is made at the CLI boundary (`src/main.rs` races the run future against `shutdown::forced_exit()`), which drops the run future and attempts status cleanup before exiting. |
| AC-S4: RS-08/09 ownership: cancel handshakes/new streams; observe/join all child tasks without spawn/drain race; close endpoint/status/pollers/watchers only afterward | Code ownership review plus AC-S1 external no-new-target/closure/resource observations. Pending incoming-handshake state not reliably inducible through TCP process fixture; report coverage limit, do not claim direct handshake-state test. |
| AC-S6: TCP-02 graceful drain includes last channel's final queued bytes before endpoint close | `roles_deliver_last_channel_response_before_early_shutdown_exit`: actual roles, one half-closed stream, stop, final 256 KiB literal-pattern response, target FIN, delayed client read; exact bytes/EOF and process success before full grace. No held sibling to hide premature endpoint close. |
| AC-S5: #86 bounded-drain technical work / no placeholder sleep: when no channels need draining stop completes without consuming full grace | `idle_roles_exit_without_waiting_for_drain_grace`: both actual idle roles SIGTERM, successful exit <2 s, status removed, listener rebind, relay shutdown. |
| Existing TCP-02 healthy pipes remain untimed in normal operation | Existing `established_client_remains_usable_past_connect_deadline` regression; full suite. |

Rust rules: RS-01 cohesive ownership; RS-04 deliberate task lifetimes; RS-05
observed errors; RS-08 cancellation/drop/join; RS-09 bounded shutdown without
healthy pipe timeout; RS-10 no key logs; RS-13 isolated TLS fixture/platform
limits; RS-14/15 spec-derived process tests; RS-16 Rust 1.91 locked gates.
CodeGraph tools unavailable in worker allowlist; named source seams inspected
directly. Independent Standards/Spec review remains parent-owned; no delegation,
PR, push or merge in this task.

## Implementation checkpoint — not final acceptance

Partial candidate remains **uncommitted** at baseline HEAD
`3138842392960e1ab259d720effbad77821bfd13`. No reset/stash, push, PR, merge or
subagent delegation. Original worker timed out; same run resumed with explicit
bounded-checkpoint instruction on 2026-10-30 UTC. Tracked source/test diff before
and after resume has SHA-256
`28238b4f0e3a60985b1755afd34a2400e8571174947fd4e9c411c93c075f8fda`, matching
`/tmp/issue86-timeout-partial.diff`. This digest excludes this untracked evidence
file and preexisting `.codegraph/`. No runtime/test source edited during resume.

Implemented ownership shape: listeners own client JoinSets; Serve accept owner
owns connection supervisors, each owning stream JoinSet. Stop propagates one
absolute deadline via watch; biased cancellation wins over ready admission,
handshake, local target dial or new-client establishment. Leaf tasks drain then
abort/join on expiry; supervisors join descendants rather than aborting them.
Active-stream counters use drop guards. Auxiliary pollers/watchers retain owned
handles; role loops stop/join them and status flush only after channel joins.
Skeleton no longer performs placeholder/post-endpoint drain sleep. Repeated
stop does not reset deadline. Normal healthy pipes receive no added timeout.

Important drain detail: QUIC `AsyncWrite::shutdown` queues FIN, not delivery.
Each channel now retains `SendStream::stopped()` future and waits for final-byte
acknowledgement after pipe completion. During shutdown this remains inside same
3 s leaf-task budget; no second grace or unconditional sleep. Upstream endpoint
close subsequently waits for QUIC close retransmission (documented about 3 s
for failed connectivity), distinct from application drain.

### Actual RED / GREEN and fixture failures

- `/tmp/86-shutdown-red-access.log`: valid process RED before Access lifecycle
  edit, `graceful response lost: []`; immediate endpoint close discarded response.
- `/tmp/86-shutdown-red-serve.log`: same valid process RED for Serve before its
  lifecycle edit. First respective GREENs:
  `/tmp/86-shutdown-green-access.log` and `/tmp/86-shutdown-green-serve.log`, held
  closure 3.0094 / 3.0087 s with exact completing-stream response.
- `/tmp/86-shutdown-last-first.log`: stronger last-channel process RED after
  initial drain implementation, Serve delivered only 51,146 of 262,144 bytes.
  Added final-byte acknowledgement ownership; `/tmp/86-shutdown-last-green.log`
  passed both roles with all 256 KiB, EOF and early exit around 1.29 s.
- `/tmp/86-shutdown-cancel.log`: pending clients cancelled promptly but <2 s
  process-exit assertion failed. Upstream Iroh close retransmission explained
  ~3 s exit; client closure remains <1 s, process bound changed to <5 s.
  `/tmp/86-shutdown-cancel-green.log` passed both dial/lock and backoff variants.
- Harness-only failures, not behavioral RED: initial test used nonexistent
  `Result::is_err_or`; temporary watcher rename compile failure; idle test wrote
  debug-form EndpointId instead of string. Corrected before valid runs.
- `/tmp/86-shutdown-process-final.log`: extra parallel configured suite failed
  12 passed / 2 failed. Existing recovery fixture hit listener bind race
  `Address already in use (os error 48)`; new idle Serve fixture unnecessarily
  released/rebound unused target port and collided. Fixed idle fixture to retain
  its real target reservation; did not change unrelated recovery fixture.
  `/tmp/86-shutdown-process-serial.log`: configured suite then passed 14/14,
  219.08 s. Port reservation race in older parallel fixtures remains disclosed.

### Passing historical gates, interrupted final gate

Rust 1.91.1, macOS ARM64, locked dependencies, isolated local relays only:

- `/tmp/86-shutdown-full-final.log`: complete earlier `cargo +1.91.1 test
  --all-features --locked` passed **155 tests, 5 ignored**, before final idle
  fixture reservation edit. Covers healthy channel after 32 s idle, peer loss,
  target reset, retries, status and quick CLI regressions. Not final-partial gate.
- `/tmp/86-shutdown-clippy-final.log`: earlier locked all-target/all-feature
  Clippy passed; `/tmp/86-shutdown-clippy-final2.log` also completed successfully.
- `/tmp/86-shutdown-repeat-{1,2,3}.log`: targeted `shutdown_` filter passed
  5/5 three times, held closures 3.006–3.010 s; exact last-channel responses and
  repeated-stop/cancellation assertions passed.
- `/tmp/86-shutdown-full-final2.log`: final full-suite rerun **interrupted by
  worker timeout** after 111 unit tests passed, while `access_path_logging` was
  running. No full result; must not present as final full gate.

### Bounded resume validation

Same run resumed; verified branch/HEAD, unchanged diff digest and empty index.
No matching current-worktree Cargo/test/role process found before or after
validation. Unrelated #82 build and installed role processes left untouched.
Commands on Rust `1.91.1 (ed61e7d7e 2025-11-07)`, macOS ARM64:

| Command | Actual outcome / log |
|---|---|
| `cargo +1.91.1 fmt --all -- --check` | PASS; `/tmp/86-shutdown-resume-fmt.log` (empty output) |
| `cargo +1.91.1 clippy --all-targets --all-features --locked -- -D warnings` | PASS, 0.31 s; `/tmp/86-shutdown-resume-clippy.log` |
| `cargo +1.91.1 test --all-features --locked --test configured_multiplex shutdown_ -- --nocapture` | PASS, 5/5, 12.11 s; `/tmp/86-shutdown-resume-targeted.log` |
| `cargo +1.91.1 test --all-features --locked --test configured_multiplex idle_roles_exit_without_waiting_for_drain_grace -- --nocapture` | PASS, 1/1, 2.91 s; `/tmp/86-shutdown-resume-idle.log` |
| `git diff --check` | PASS |
| `git diff --cached --name-only` | Empty; no staged files |
| Final full `cargo +1.91.1 test --all-features --locked` during bounded resume | NOT RUN; earlier interrupted attempt remains pending |

Resume observations: Access/Serve held channels closed 3.0119 / 3.0134 s;
exact completing response and no post-stop target admission observed. Last
256 KiB channel delivered exactly in both roles; process exits 1.3582 / 1.3480 s.
Pending dial/lock and backoff/lock clients closed <1 s; processes exited
3.0239 / 2.9745 s after stop. Idle Serve/Access exited 1.0281 / 1.0289 s. All
successful scenarios checked own status removal and bounded child reaping;
Access listener rebind and target socket closure checked where applicable;
relay explicitly shut down. Test-feature executable SHA-256
`740d29e08968e33d9449442e1394d43c1f114129f88dca6fd2fed98190db7462`.

### Parent verification after resume

The parent ran `cargo +1.91.1 fmt --all -- --check`,
`cargo +1.91.1 clippy --all-targets --all-features --locked -- -D warnings`,
and `cargo +1.91.1 test --all-features --locked` with default test parallelism
on macOS arm64. All passed: 155 tests, 5 ignored. Both previously failing idle
and peer-recovery tests passed. The tracked diff SHA-256 before and after was
`28238b4f0e3a60985b1755afd34a2400e8571174947fd4e9c411c93c075f8fda`;
log: `/tmp/issue86-parent-shutdown-full.log`. Existing address-reservation races
remain a test reliability risk; a passing rerun does not prove they are absent.

The locked `noq` 1.2.0 `SendStream::stopped()` documentation states that
`Ok(None)` means the peer acknowledged receipt of all stream data after FIN,
not that the application processed it. `Ok(Some(code))` indicates peer stop,
not successful delivery. Independent review must check both paths.

### Pending / limitations

- Independent Standards/Spec review is not complete.
  RS-08/09 ownership, ACK lifetime and cancellation races need independent
  review; self-check is not review approval. Keep #86 open.
- AC-S1/S2/S3/S5/S6 process evidence passed in bounded resume; AC-S4 partially
  externally covered, ownership ordering inspected only. Pending incoming
  handshake, target-dial and stream-open cancellation states not directly
  fault-injected. Fresh-peer no-admission scenario does not prove exact pending
  handshake state. No high-contention spawn/drain race or task-panic injection.
- Caller dropping entire public run future is not a completed graceful-stop
  operation; injected future resolution and OS signals are tested stop inputs.
- Three-second grace is technical candidate with local evidence, not #76
  numeric approval, wide-network benchmark or optimal setting. Upstream QUIC
  close and existing status fsync cleanup can add time after application drain;
  pathological blocking filesystem behavior not validated.
- New process signal tests Unix-only; no Windows/Linux runtime claim. No
  production-TLS/public-relay, forced-relay-only route, 0.4 interoperability,
  UDP, load/resource exhaustion or full release acceptance claimed. Five
  ignored tests remain unrun: authentic released-0.4 test and four public-relay
  tests. Test-feature binary never used against public relay.

## Post-review rework (second review of 38ca52e..cc16455)

- Forced exits moved out of the library: `forced_exit()` shares the single drain
  deadline published by `begin()`; there is no detached watchdog task any more.
  Status 130 (repeated stop after grace) and 1 (watchdog, 10 s after the grace)
  are documented in the CHANGELOG. The 10 s extra is a technical default.
- Delivery acknowledgement grace is 2 s, below the 3 s drain grace, so an
  acknowledgement wait can complete inside the drain; a pipe completing in the
  last second of a drain is still force-closed at the drain deadline. A missing
  acknowledgement is reported as peer loss (warning, Access returns an error),
  never as success, and never replayed. Serve bounds the same wait, so a
  vanished peer cannot pin the stream task or `active_streams`.
- Coverage limit: the missing-acknowledgement mapping is unit-tested with an
  injected pending future (`pipe::tests`); a real peer that withholds QUIC
  acknowledgements cannot be produced deterministically from a process test.
- Tests are Unix-only (signals; the stuck shutdown is simulated by the test-utils-only `IROH_TUNNEL_TEST_HANG_SHUTDOWN` seam); no Windows runtime claim.
