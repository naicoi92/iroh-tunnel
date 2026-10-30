# Issue #86 — bounded new-client retry slice

Baseline `166e4f1f764a8304117a0ff9ac8c0eb0c9e8acd6`. Live #86/#76 refreshed: #76 and #81 closed, both #86 native blockers closed, #76 has none. #86 handoff confirms bounded default retry with backoff; numeric defaults are technical work, not previously approved. Shutdown/drain and repeated stop are outside this slice.

## Acceptance and process test matrix (before test implementation)

Seam: actual configured Access/Serve processes, TCP clients and target, isolated loopback test relay. `test-utils` disables TLS verification; never contact public relay. TCP-02, TCP-FAILURE-01, #86 handoff and user task:

| Criterion | Process test and independent expected outcome |
|---|---|
| Unavailable peer retries with backoff and each new client fails at bounded deadline | `unavailable_peer_retries_then_closes_client_within_deadline`: configured absent peer; observe two completed timed-out dials at least 5.5 s apart, TCP EOF/error 28–40 s after connect, deadline-expiry log and live Access process. These bounds include test tolerance, not approved policy. |
| Concurrent waiting clients retain own deadlines including lock waiting | `staggered_unavailable_clients_have_independent_deadlines`: stagger real clients by 10 s; each closes within 35 s of own connect, second after first. |
| Peer recovery before deadline permits exchange | `peer_recovery_connects_waiting_client_before_deadline`: absent peer then start same configured identity, verify exact real TCP request/reply within 30 s. |
| Healthy established channels not timed out | `established_client_remains_usable_past_connect_deadline`: exchange after 32 s idle. |
| Cancellation | `shutdown_cancels_pending_connection`: real process SIGTERM while peer unavailable; client closes before 10 s observation limit. Unix only. |

Numeric default requires evidence and review, not a confirmed tracker decision. Standards: RS-01, RS-05, RS-08, RS-09, RS-13, RS-14, RS-15, RS-16. No full #86/release acceptance claimed.

## RED, implementation and measured GREEN

RED before production edits: `cargo +1.91.1 test --all-features --locked --test configured_multiplex unavailable_peer_retries_then_closes_client_within_deadline -- --nocapture` failed at 40.85 s with `unavailable peer left client waiting beyond 40s`. The original iroh dial itself remained pending; RED did not prove backoff happened. After total deadline alone, same test failed because no failed retry occurred in 30 s. Added per-dial timeout to make stalled attempts retry; repeated exponential backoff assertions now require attempts 1 and 2 and 1000/2000 ms log values.

Technical defaults for review, not prior user-approved numeric policy: 30 s total per accepted TCP client; 5 s maximum per dial; retain existing 1, 2, 4, 8, 16, 30 s capped exponential backoff, no jitter. The 30 s budget gives multiple attempts and recovery opportunity yet bounds client wait; 5 s dial allows multiple retries before that deadline, unlike observed >30 s stalled first dial. The local healthy handshake took 3.04 s and recovery took 9.05 s. These are only local observations, not proof 5 s suits real remote or mobile networks. Random jitter, configurability and wider empirical tuning remain review questions. No claim these values are optimal.

Timeout starts at accept, includes shared mutex wait, dial, retry sleeps, cached connection invalidation/redial and stream opening, without reset. Timeout/cancellation drops pending future and releases guard. Successful stream leaves timeout before piping; no replay of established data. Shutdown watch cancels pending establishment, not healthy streams; full shutdown/drain implementation remains out of scope. Dropping an iroh dial future is relied upon for cancellation; no claim about lower-layer traffic already sent. Existing client tasks remain detached, not general task/drain lifecycle work.

On macOS arm64, Rust 1.91.1, local relay only, test-feature binary SHA-256 `a1b0db6a706f7e09e2710cd69ffb09246c0090bd31b303bc628a9a1ffd85477d`:

- First valid GREEN focused test: 30.0027 s; log showed four timed-out dials and 1/2/4/8 s scheduled backoffs (last sleep truncated by overall deadline).
- `cargo +1.91.1 test --all-features --locked --test configured_multiplex -- --nocapture`: 9 passed. Two clients staggered by 10 s closed after 30.0020/30.0044 s from own connect; peer recovery exchange after 9.0476 s; established stream exchanged exact new data after 32 s idle; SIGTERM closed pending client at 5.0548 s from start. Scenario log `/tmp/issue86-retry-scenarios.log`. Subsequently strengthened retry-log and cancellation-log checks, both covered by full suite.
- `cargo +1.91.1 fmt --all -- --check`: passed.
- `cargo +1.91.1 clippy --all-targets --all-features --locked -- -D warnings`: passed, `/tmp/issue86-retry-clippy.log`.
- `cargo +1.91.1 test --all-features --locked`: 150 passed, 5 ignored, zero failures; `/tmp/issue86-retry-full.log`. Ignored: released 0.4 interoperability and four public relay tests. No test-feature executable contacted public relay.
- `git diff --check`: passed.

## Review rework and sensitivity

Listener readiness now waits for the specific bound `listen_addr` log before opening a client (RS-14/15). The unavailable-peer process test measures elapsed time from local connect, requires deadline-expiry log and checks Access still runs. It observes the first two completed 5 s dial timeouts by polling their log lines with monotonic `Instant` timestamps; spacing must be at least 5.5 s, allowing 0.5 s observation/scheduler slack against the expected 6 s (5 s dial plus 1 s backoff). The local relay fixture deliberately uses an absent peer and requires timeout logs; it does not cover immediate dial errors or heavily delayed polling.

Sensitivity trials temporarily modified production files in this worktree only and restored them before final gates: replacing retry sleep with zero produced failure at 4.9785 s spacing; changing the 30 s deadline to 15 s produced failure at 15.0085 s elapsed. These are deliberate failing mutation runs, not production changes. Logs: `/tmp/86-mutation-backoff.log`, `/tmp/86-mutation-deadline.log`.

On macOS arm64, Rust 1.91.1, isolated local relay: focused process suite passed 9/9; unavailable client closed at 30.0042 s with observed 6.0298 s spacing (`/tmp/86-review-process.log`). `cargo +1.91.1 fmt --all -- --check`, `cargo +1.91.1 clippy --all-targets --all-features --locked -- -D warnings`, and `cargo +1.91.1 test --all-features --locked` passed; full test count 150 passed, 5 ignored (`/tmp/86-review-full.log`). Independent review of rework remains required.

Coverage limits: initial deadlines are measured from local connect, a close approximation to accepted time, not exact synchronized clocks. No forced relay-only route, real Internet or production TLS, Linux/Windows runtime, late recovery near deadline, cancellation during every state, precise retry timestamps under load, high contention or long-haul performance claim. Connection/open failure error classification and complete lifecycle policy are not resolved. Independent Standards and Spec reviews required; #86 and release remain open. Standards RS-01, RS-05, RS-08/09, RS-13/14/15/16 apply.
