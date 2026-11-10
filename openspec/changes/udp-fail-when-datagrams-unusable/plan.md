# UDP Datagram Setup Failure Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** Close a UDP service connection at setup when it cannot carry QUIC datagrams, log the cause on both sides, back Access off for 30 s, and keep `limit_shrink` for mid-session loss only (#152).

**Architecture:** Crate-private pure policy in `engine::udp` (usability rule, dedicated close code and predicate, per-datagram send decision, redial back-off keyed by connection) is called from Serve's `handle_udp_connection` and Access's `udp_listen_loop` / `relay_peer_replies`. `ServiceDialer` policy and TCP paths do not change; Access only uses the existing `invalidate`. A `test-utils`-only env seam disables datagrams in one role so real binaries reproduce the failure.

**Tech Stack:** Rust 2021 (MSRV 1.91), Tokio, iroh 1.4.1 (noq 1.4), tracing, cargo test.

## Global Constraints

- Rust standards RS-01–RS-16 (`docs/agents/rust-coding-standards.md`); MSRV 1.91.
- Glossary terms (`docs/GLOSSARY.md`): Serve, Access, Flow, Session incarnation, Datagram limit, Oversized datagram.
- Unusable means maximum QUIC datagram size absent or at most 4 bytes (`HEADER_LEN`); one implementation, through `payload_limit`.
- Close code `0x1d47`, reason bytes exactly `udp datagrams unsupported`.
- Back-off is a fixed 30 s, not configurable; drops during it record no drop counter.
- No new `pub` item in `iroh-tunnel-engine` (workspace-boundaries "Public internal APIs are justified and documented"); every new item is `pub(crate)`.
- Each engine file exports at most one `pub fn` (existing `tests/workspace_layout.rs` gate); the new policy uses `pub(crate)`, which the gate does not count, and lives in one file per responsibility.
- No change to status schema, host API, CLI, config, wire framing, TCP forwarding, or production `ServiceDialer` methods or policy; private read-only `#[cfg(test)]` observers are permitted.
- `IROH_TUNNEL_TEST_DISABLE_DATAGRAMS` is read only under `#[cfg(feature = "test-utils")]`.
- Role logs go to stdout at `iroh_tunnel=info` by default (`src/cli.rs:521-546`), so tests read info, warn and error lines without `RUST_LOG`.
- Full gate: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, `cargo test --all-features --locked`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`, `cargo +1.91 test --workspace --all-features --locked`.

## Review Focus

- **Healthy pairs are unaffected.** Two current binaries always advertise datagrams; the check never fires for them. Pinned by unchanged `tests/udp_tunnel.rs` and `tests/quick_run.rs` (Task 6 Step 6).
- **Check once per connection, not per datagram.** A per-datagram check would close a connection whose limit only shrank. Task 4 places it where a newly dialed connection is first seen; the mid-session decision stays per datagram (Task 1 `send_decision_*`).
- **Reader-first close must not cost a second window.** If Access learns of Serve's close in the reader, the dead connection is evicted there; the first datagram after 30 s dials afresh. Pinned by Task 4 `reader_close_evicts_and_resumes_after_one_window`.
- **One failure, one window.** Reader and send path may both observe the same close; a stale reader of an older connection may finish late. Neither restarts or alters the window. Failure identity is a loop-owned `Incarnation`, not `stable_id()` (an allocation address that can be reused). Pinned by Task 1 `backoff_ignores_a_repeat_of_the_same_failure`, `backoff_ignores_a_stale_incarnation` and Task 4 `stale_incarnation_close_does_not_start_a_window`.
- **Ordinary peer loss keeps today's behavior.** Reset, timeout or another close code never start a back-off. Pinned by Task 1 `close_predicate_ignores_*` and unchanged `forwarding_resumes_after_serve_restarts`.

---

## Contracts

All engine items are `pub(crate)`; none is public API.

| Name | new / modified | Shape | public |
|---|---|---|---|
| `udp::DATAGRAMS_UNSUPPORTED_CODE` | new | `pub(crate) const DATAGRAMS_UNSUPPORTED_CODE: u32 = 0x1d47;` wire-visible application close code | no (wire contract, documented in `docs/udp-integration-contract.md`) |
| `udp::DATAGRAMS_UNSUPPORTED_REASON` | new | `pub(crate) const DATAGRAMS_UNSUPPORTED_REASON: &[u8] = b"udp datagrams unsupported";` | no (wire text) |
| `udp::carries_datagrams` | new, `udp/carries_datagrams.rs` | `pub(crate) fn carries_datagrams(max_datagram_size: Option<usize>) -> bool` = `payload_limit(max).is_some()` | no |
| `udp::closed_for_unsupported_datagrams` | new, `udp/closed_for_unsupported_datagrams.rs` | `pub(crate) fn closed_for_unsupported_datagrams(error: &iroh::endpoint::ConnectionError) -> bool`; true only for `ApplicationClosed(c)` with `c.error_code == VarInt::from_u32(DATAGRAMS_UNSUPPORTED_CODE)` | no |
| `udp::SendDecision`, `udp::send_decision` | new, `udp/send_decision.rs` | `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub(crate) enum SendDecision { Send, Drop(DropReason) }`; `pub(crate) fn send_decision(max_datagram_size: Option<usize>, payload_len: usize) -> SendDecision`: limit `None` → `Drop(LimitShrink)`, `payload_len > limit` → `Drop(Oversize)`, else `Send` | no |
| `udp::RedialBackoff`, `udp::BackoffCheck`, `udp::Incarnation` | new, `udp/redial_backoff.rs` | Module-local `const REDIAL_BACKOFF: Duration = Duration::from_secs(30);` (not re-exported). `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub(crate) struct Incarnation(u64);` — a token owned by the UDP loop, never derived from `stable_id()` (a freed allocation address may be reused). `#[derive(Debug)] pub(crate) struct RedialBackoff { window: Duration, latest: u64, failed: Option<u64>, until: Option<std::time::Instant>, warned: bool }`; `impl Default` uses `REDIAL_BACKOFF`; `#[cfg(test)] pub(crate) fn with_window(window: Duration) -> Self`; `#[cfg(test)] pub(crate) fn waiting_until(&self) -> Option<std::time::Instant>` (read-only, never consumes the warning flag or clears the window); `pub(crate) fn begin(&mut self) -> Incarnation` — increments `latest` and returns it; `pub(crate) fn record(&mut self, incarnation: Incarnation, now: std::time::Instant)` — ignored when `incarnation` is not the latest begun or equals `failed`, otherwise `failed = Some(..)`, `until = now + window`, `warned = false`; `pub(crate) fn check(&mut self, now: std::time::Instant) -> BackoffCheck` — `Clear` when no window or `now >= until` (clears `until`, `warned`), else `Waiting { first, resumes_in }`, `first` true once per window. `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub(crate) enum BackoffCheck { Clear, Waiting { first: bool, resumes_in: Duration } }` | no |
| `UdpForwardCtx` (`access/run_access.rs`) | new | `#[derive(Clone)] struct UdpForwardCtx { dialer: Arc<ServiceDialer>, backoff: Arc<std::sync::Mutex<RedialBackoff>>, gate: UdpDialGate, #[cfg(test)] hooks: Option<Arc<UdpTestHooks>> }` | no |
| `UdpTestHooks` (`access/run_access.rs`) | new, `#[cfg(test)]` only | `#[derive(Default)] struct UdpTestHooks { gate_waiting: tokio::sync::watch::Sender<u64>, admitted: tokio::sync::watch::Sender<u64>, reader_failed: tokio::sync::watch::Sender<u64> }`, carried as the `#[cfg(test)]` field `hooks` of `UdpForwardCtx`. Each is a sequence counter bumped with `send_modify(|n| *n += 1)` (never coalesced, never pre-armed): the loop bumps `gate_waiting` right before awaiting the gate, `admitted` right after a `Clear` back-off check and before `get_or_dial`; the reader bumps `reader_failed` right after it matched the dedicated close and before acquiring the gate. Tests read the current value as a baseline and wait with `wait_for(|n| *n > baseline)`. Production builds compile neither the type nor the calls | no |
| `UdpDialGate` (`access/run_access.rs`) | new | `type UdpDialGate = Arc<tokio::sync::Mutex<()>>;` one per UDP service, owned by `udp_forward_loop`. Held by the forwarding loop from the back-off `check` through `get_or_dial` and the adoption check; held by the reader from `record` through `invalidate`. It serializes dial admission with failure recording and eviction; `ServiceDialer` is unchanged. The `std::sync::Mutex` of `RedialBackoff` is still never held across `.await` | no |
| `relay_peer_replies` (`access/run_access.rs`) | modified | `async fn relay_peer_replies(conn: Connection, incarnation: Incarnation, socket: Arc<UdpSocket>, flows: Arc<std::sync::Mutex<ClientFlows>>, ctx: UdpForwardCtx)` — `stats` is read from `ctx.dialer.udp_stats`, back-off and gate from `ctx` | no |
| `udp_listen_loop` / `udp_forward_loop` (`access/run_access.rs`) | modified / new | `udp_listen_loop(dialer, stop)` keeps its signature: it binds the socket (as today) and calls the new private `async fn udp_forward_loop(ctx: UdpForwardCtx, socket: Arc<UdpSocket>, stop: shutdown::Stop)` with a `UdpForwardCtx` holding `RedialBackoff::default()` (see Task 4 hook plumbing). Tests call `udp_forward_loop` with their own bound socket (known address) and `RedialBackoff::with_window(..)` | no |
| `IROH_TUNNEL_TEST_DISABLE_DATAGRAMS`, `testing::datagrams_disabled` | new | env var `1` → `datagram_receive_buffer_size(None)`; `#[cfg(feature = "test-utils")] pub(crate) fn datagrams_disabled() -> bool` in `crates/engine/src/testing.rs` | no (test seam) |
| `limit_shrink` rustdoc (`observation::types` field, `host::status::schema::UdpDrops` field, `udp::DropReason::LimitShrink`) | modified | text only: "Datagrams dropped because the datagram limit became unusable during the session incarnation." | docs only |

The UDP loop's adoption state becomes `current: Option<(Connection, Incarnation)>` instead of `Option<usize>`: holding the adopted `Connection` keeps its allocation alive, so comparing `stable_id()` against it cannot be fooled by address reuse; a rejected connection leaves `current` as `None`. Failure identity is the `Incarnation` begun for each newly seen connection.

---

## Task 1: Pure UDP setup and send policy — implements 1

**Covers:** 1.1, 1.2, 1.3, 1.4, 1.5, 1.6
**Difficulty:** Normal
**Blocked by:** none

**Files:**
- Create: `crates/engine/src/udp/carries_datagrams.rs` (also holds the two close constants)
- Create: `crates/engine/src/udp/closed_for_unsupported_datagrams.rs`
- Create: `crates/engine/src/udp/send_decision.rs`
- Create: `crates/engine/src/udp/redial_backoff.rs`
- Modify: `crates/engine/src/udp/mod.rs` (modules and `pub(crate) use` re-exports)

**Interfaces:**
- Consumes: `payload_limit`, `DropReason`, `HEADER_LEN`.
- Produces: the `udp::*` contracts above.

Each new file has its own `#[cfg(test)] mod tests` (layout gate).

- [ ] **Step 1: Register modules and write failing tests.** Add the four `mod` lines to `udp/mod.rs` now, with each file holding only its tests, so the tests are compiled. Tests:
  ```rust
  // carries_datagrams.rs
  #[test] fn carries_datagrams_at_the_boundary() {
      assert!(!carries_datagrams(None));
      assert!(!carries_datagrams(Some(HEADER_LEN)));
      assert!(carries_datagrams(Some(HEADER_LEN + 1)));
  }
  // closed_for_unsupported_datagrams.rs
  fn app_close(code: u32) -> iroh::endpoint::ConnectionError {
      iroh::endpoint::ConnectionError::ApplicationClosed(iroh::endpoint::ApplicationClose {
          error_code: iroh::endpoint::VarInt::from_u32(code),
          reason: bytes::Bytes::from_static(DATAGRAMS_UNSUPPORTED_REASON),
      })
  }
  #[test] fn close_predicate_matches_the_dedicated_code() {
      assert!(closed_for_unsupported_datagrams(&app_close(DATAGRAMS_UNSUPPORTED_CODE)));
  }
  #[test] fn close_predicate_ignores_other_application_codes() {
      assert!(!closed_for_unsupported_datagrams(&app_close(0)));
  }
  #[test] fn close_predicate_ignores_local_close_reset_and_timeout() {
      use iroh::endpoint::ConnectionError::*;
      for e in [LocallyClosed, Reset, TimedOut] { assert!(!closed_for_unsupported_datagrams(&e)); }
  }
  // send_decision.rs
  #[test] fn send_decision_drops_as_limit_shrink_when_unusable() {
      assert_eq!(send_decision(None, 10), SendDecision::Drop(DropReason::LimitShrink));
      assert_eq!(send_decision(Some(HEADER_LEN), 0), SendDecision::Drop(DropReason::LimitShrink));
  }
  #[test] fn send_decision_drops_as_oversize_over_a_usable_limit() {
      assert_eq!(send_decision(Some(104), 101), SendDecision::Drop(DropReason::Oversize));
  }
  #[test] fn send_decision_sends_within_the_limit() {
      assert_eq!(send_decision(Some(104), 100), SendDecision::Send);
      assert_eq!(send_decision(Some(104), 0), SendDecision::Send);
  }
  // redial_backoff.rs — every test starts with `let t0 = std::time::Instant::now();`
  #[test] fn backoff_is_clear_before_any_failure() {
      let t0 = std::time::Instant::now();
      assert_eq!(RedialBackoff::default().check(t0), BackoffCheck::Clear);
  }
  #[test] fn backoff_waits_30_seconds_and_warns_once() {
      let t0 = std::time::Instant::now();
      let mut b = RedialBackoff::default();
      let a = b.begin();
      b.record(a, t0);
      assert_eq!(b.check(t0), BackoffCheck::Waiting { first: true, resumes_in: REDIAL_BACKOFF });
      assert_eq!(b.check(t0 + Duration::from_secs(29)), BackoffCheck::Waiting { first: false, resumes_in: Duration::from_secs(1) });
      assert_eq!(b.check(t0 + REDIAL_BACKOFF), BackoffCheck::Clear);
  }
  #[test] fn backoff_ignores_a_repeat_of_the_same_failure() {
      let t0 = std::time::Instant::now();
      let mut b = RedialBackoff::default();
      let a = b.begin();
      b.record(a, t0);
      let _ = b.check(t0);
      b.record(a, t0 + Duration::from_secs(10));
      assert_eq!(b.check(t0 + Duration::from_secs(10)), BackoffCheck::Waiting { first: false, resumes_in: Duration::from_secs(20) });
  }
  #[test] fn backoff_new_incarnation_failure_starts_a_new_window() {
      let t0 = std::time::Instant::now();
      let mut b = RedialBackoff::default();
      let a = b.begin();
      b.record(a, t0);
      let _ = b.check(t0 + REDIAL_BACKOFF);
      let c = b.begin();
      b.record(c, t0 + REDIAL_BACKOFF);
      assert_eq!(b.check(t0 + REDIAL_BACKOFF), BackoffCheck::Waiting { first: true, resumes_in: REDIAL_BACKOFF });
  }
  #[test] fn backoff_ignores_a_stale_incarnation() {
      let t0 = std::time::Instant::now();
      let mut b = RedialBackoff::default();
      let a = b.begin();
      let _c = b.begin();
      b.record(a, t0);
      assert_eq!(b.check(t0), BackoffCheck::Clear);
  }
  ```
  Incarnation tokens are counters, independent of `stable_id()`; `backoff_new_incarnation_failure_starts_a_new_window` pins that every begun incarnation gets its own window.

- [ ] **Step 2: Confirm red with the right tests selected**
  Run: `cargo test -p iroh-tunnel-engine --all-features --locked udp::`
  Expected: compile errors naming the missing items in all four files (not "0 tests").

- [ ] **Step 3: Implement the contracts** and add the `pub(crate) use` re-exports. `carries_datagrams` and `send_decision` call `payload_limit`. Rustdoc of `carries_datagrams.rs` states the invariant "a session incarnation exists only on a connection that can carry datagrams" and cites `docs/udp-integration-contract.md`.

- [ ] **Step 4: Confirm green**
  Run: `cargo test -p iroh-tunnel-engine --all-features --locked udp::`
  Expected: the 12 new tests plus the existing `udp::` tests pass (28 before this change), including the absent-limit snapshot test in `udp/types.rs`.

- [ ] **Step 5: Layout and format**
  Run: `cargo fmt --all -- --check` and `cargo test --test workspace_layout --all-features --locked`
  Expected: clean; layout gate passes. Clippy `-D warnings` first runs in Task 4 Step 7: until Task 4 consumes the back-off items (Task 3 consumes only the classifier, predicate and send decision), the library target reports `dead_code`/`unused_imports`, and silencing that with `pub`, `allow` or dummy uses is not allowed (tasks.md 1.6, 3.4, 4.5).

- [ ] **Step 6: Commit** `feat(udp): add datagram setup policy and redial backoff (#152)` with the five files.

---

## Task 2: Datagram-disable test seam — implements 2

**Covers:** 2.1, 2.2, 2.3
**Difficulty:** Easy
**Blocked by:** none

**Files:**
- Modify: `crates/engine/src/testing.rs`
- Modify: `crates/engine/src/endpoint/create.rs` (`transport_config`, ~lines 73-79)
- Modify: `tests/workspace_layout.rs`

**Interfaces:**
- Produces: `IROH_TUNNEL_TEST_DISABLE_DATAGRAMS`, `testing::datagrams_disabled`.

- [ ] **Step 1: Failing source check** `datagram_disable_seam_is_test_utils_only` in `tests/workspace_layout.rs`, over `engine_files()`: the literal `IROH_TUNNEL_TEST_DISABLE_DATAGRAMS` appears in exactly one file, `testing.rs`; the line before `fn datagrams_disabled` is `#[cfg(feature = "test-utils")]`; in `endpoint/create.rs` every line calling `datagrams_disabled` sits inside a block opened right after `#[cfg(feature = "test-utils")]`.
- [ ] **Step 2: Confirm red:** `cargo test --test workspace_layout --all-features --locked datagram_disable_seam` → FAIL (literal not found).
- [ ] **Step 3: Implement** `datagrams_disabled()` (true when the variable equals `"1"`) and, in `transport_config`, `#[cfg(feature = "test-utils")] if crate::testing::datagrams_disabled() { builder = builder.datagram_receive_buffer_size(None); }` (iroh `QuicTransportConfigBuilder::datagram_receive_buffer_size(self, Option<usize>) -> Self`). Rustdoc: "Test seam: never in production builds."
- [ ] **Step 4: Verify:** `cargo test --test workspace_layout --all-features --locked` and `cargo build -p iroh-tunnel-engine --locked` → PASS and a clean build without `test-utils`.
- [ ] **Step 5: Commit** `test(engine): add test-utils seam to disable datagrams (#152)`.

---

## Task 3: Serve setup check and close reporting — implements 3

**Covers:** 3.1, 3.2, 3.3, 3.4
**Difficulty:** Normal
**Blocked by:** Task 1

**Files:**
- Modify: `crates/engine/src/serve/run_serve.rs` (`handle_udp_connection` ~line 696; `relay_upstream_replies` ~lines 946-990)

**Interfaces:**
- Consumes: `carries_datagrams`, `closed_for_unsupported_datagrams`, `send_decision`, `SendDecision`, `DATAGRAMS_UNSUPPORTED_CODE`, `DATAGRAMS_UNSUPPORTED_REASON`.

- [ ] **Step 1: Setup check.** In `handle_udp_connection`, derive `service` from `proto::name_from_alpn(conn.alpn())` and `peer` from `conn.remote_id()`, as `accept_loop` does. Before the flow table: if `!carries_datagrams(conn.max_datagram_size())`, `tracing::error!(%peer, service, "udp datagrams unsupported by access peer, closing the connection")`, `conn.close(VarInt::from_u32(DATAGRAMS_UNSUPPORTED_CODE), DATAGRAMS_UNSUPPORTED_REASON)`, `return`.
- [ ] **Step 2: Received close.** In the `read_datagram` error arm: if `closed_for_unsupported_datagrams(&error)`, `tracing::error!(%peer, service, "access peer closed the udp connection: datagrams unsupported")`; otherwise keep the debug line.
- [ ] **Step 3: Send decision.** In `relay_upstream_replies`, replace the inline limit branches with `match send_decision(conn.max_datagram_size(), n)`; keep `set_datagram_limit(payload_limit(..))`, the counters and both warn texts.
- [ ] **Step 4: Verify:** `cargo test -p iroh-tunnel-engine --all-features --locked` and `cargo test --test udp_tunnel --all-features --locked` → pass. No clippy gate here (see Task 1 Step 5).
- [ ] **Step 5: Commit** `feat(serve): close udp connections that cannot carry datagrams (#152)`.

---

## Task 4: Access setup check, close reporting and back-off — implements 4

**Covers:** 4.1, 4.2, 4.3, 4.4, 4.5
**Difficulty:** Hard
**Blocked by:** Task 1

**Files:**
- Modify: `crates/engine/src/access/run_access.rs` (`udp_listen_loop` ~lines 604-740, `relay_peer_replies` ~lines 744-773, `mod tests`)

**Interfaces:**
- Consumes: all Task 1 contracts; existing `ServiceDialer::{get_or_dial, invalidate}` unchanged.
- Produces: modified `relay_peer_replies`, `udp_listen_loop` and new `udp_forward_loop` (Contracts).

Locking rule: the `RedialBackoff` mutex is a `std::sync::Mutex`, taken only for `begin`/`record`/`check` and never held across `.await` (RS-08).

Test harness (private, in `mod tests`; all async, all owned tasks aborted **and joined** in `shutdown()`):
- `struct TestPeer { ep: iroh::Endpoint, accepted: tokio::sync::mpsc::UnboundedReceiver<Connection>, task: JoinHandle<()> }` built by `async fn test_peer(datagrams: bool) -> TestPeer`: binds a loopback Serve-like endpoint with ALPN `t`, relay disabled; when `datagrams` is false its `QuicTransportConfig` sets `datagram_receive_buffer_size(None)`. The accept task sends every accepted `Connection` on the channel and, for each, spawns (into a `JoinSet` it owns) a datagram echo: `read_datagram` → `send_datagram` the same bytes. The test holds the received `Connection`s, so it decides when each one closes (`conn.close(VarInt::from_u32(DATAGRAMS_UNSUPPORTED_CODE), DATAGRAMS_UNSUPPORTED_REASON)`) and can await `conn.closed()`. The accept task also selects on a `oneshot` stop receiver held by `TestPeer`; `async fn shutdown(self)` sends stop, after which the supervisor stops accepting, calls `abort_all()` on its echo `JoinSet` and drains it with `join_next()` before returning; `shutdown` then awaits the supervisor handle (bounded 5 s) and closes the endpoint (RS-08: descendants are joined before the supervisor ends).
- `async fn udp_dialer(peer: &TestPeer) -> Arc<ServiceDialer>`: as `local_dialer` but `Protocol::Udp`, no accept task of its own.
- `struct ForwardHarness { client: UdpSocket, access_addr: SocketAddr, backoff: Arc<std::sync::Mutex<RedialBackoff>>, gate: UdpDialGate, stop: shutdown handle, task: JoinHandle<()> }` built by `async fn forward(dialer, window)`: binds the Access-side `UdpSocket` on `127.0.0.1:0`, spawns `udp_forward_loop` with `RedialBackoff::with_window(window)`; the harness keeps the `Arc<ServiceDialer>`; `shutdown()` signals stop, joins the loop, then calls `dialer.stop_auxiliary_tasks().await` (disconnect watchers and path pollers, as `run_access` does) and closes the client endpoint, each bounded 5 s.
- Observation is read-only: tests read `backoff.lock().waiting_until()` and `dialer`'s cached connection via a `#[cfg(test)] async fn cached_conn_id(&self) -> Option<usize>` on `ServiceDialer` (reads `state.conn`; no policy change). Never call `check` from tests.
- Window `Duration::from_millis(500)`; all waits are bounded `tokio::time::timeout` loops polling every 10 ms, up to 5 s. When a test captures `t = waiting_until()`, it asserts `t - Instant::now() >= 200 ms` (failing with a timing diagnostic otherwise) and sends at least one datagram inside the negative-observation interval, so an empty interval never counts as suppression coverage.

- [ ] **Step 1: Failing orchestration tests** in `mod tests`:
  - `adoption_check_closes_and_backs_off` (peer `datagrams: false`): send one datagram; await the first accepted connection `a` and `a.closed()`; assert the close is `ApplicationClosed` with `DATAGRAMS_UNSUPPORTED_CODE`. Let `t = waiting_until().unwrap()`. Send a datagram every 50 ms until `t - 50 ms`; assert no second connection arrived on the channel. Then send once after `t`; assert exactly one new connection arrives within 1 s.
  - `reader_close_evicts_and_resumes_after_one_window` (peer `datagrams: true`): send one datagram and receive its echo (connection `a` adopted). Stop sending. Close `a` from the peer with the dedicated code. Await `waiting_until().is_some()` and `cached_conn_id().await.is_none()` (no client traffic since the close, so only the reader can have recorded and evicted). Let `t = waiting_until().unwrap()`. Send every 50 ms until `t - 50 ms`; assert no new connection. After `t`, send once; assert exactly one new connection `b` arrives and the datagram is echoed back through it. Keep `b` open until the end.
  - `stale_incarnation_close_does_not_start_a_window` (peer `datagrams: true`): adopt `a` (one echo). From the test, `dialer.invalidate(&cached a)`; send again so the loop adopts `b` (echo received, second connection on the channel). Close `a` from the peer with the dedicated code. For 300 ms keep round-tripping through `b`; assert every echo arrives and `waiting_until()` stays `None`.
  - `reader_close_racing_a_send_cannot_redial` (peer `datagrams: true`, hooks on): adopt `a` (one echo). Test locks the dialer's private `dial` mutex (blocks `get_or_dial` after its check). Take `base = *hooks.admitted.subscribe().borrow()` (A's admission is already counted). Send one client datagram; wait for `admitted > base` (this datagram passed `Clear` and holds the gate, now blocked on `dial`). Close `a` from the peer; wait for `reader_failed` to exceed its baseline (the reader matched the close and is waiting for the gate). Release `dial`. The loop gets the dead cached `a`, its send fails and records the incarnation; then the reader acquires the gate and records the same incarnation (no-op) and invalidates. Capture `t` and assert, as above, no second connection before `t` and exactly one after. Run a mirror variant `reader_close_before_admission_cannot_redial`: close `a` and wait for `reader_failed` to exceed its baseline and for `waiting_until().is_some()` before sending; assert the same. Mutation check (manual, recorded in apply.md): temporarily removing the loop's gate acquisition makes the first test fail (a second connection appears before `t`).
  - `shutdown_while_gate_held_is_prompt`: test locks the `UdpDialGate` via the harness and keeps the guard to the end; takes the `gate_waiting` baseline; sends a datagram; waits for `gate_waiting > base` (the loop is now pending on the gate); then calls `ForwardHarness::shutdown()` and asserts the loop finishes within 1 s. Mutation check (manual, recorded in apply.md): taking the gate outside the shutdown `select!` makes this test time out.
  - `address_reuse_cannot_skip_the_setup_check` is covered structurally: `current` holds the adopted `Connection` (Contracts), so its address cannot be reused while compared; a rejected connection sets `current = None`. No runtime test can force allocator reuse; the reviewer checks the field type.
- Hook plumbing: a private `#[derive(Clone)] struct UdpForwardCtx { dialer: Arc<ServiceDialer>, backoff: Arc<std::sync::Mutex<RedialBackoff>>, gate: UdpDialGate, #[cfg(test)] hooks: Option<Arc<UdpTestHooks>> }` is built by `udp_listen_loop` (hooks `None`) or the test harness and passed to `udp_forward_loop(ctx, socket, stop)` and `relay_peer_replies`; every function stays at seven parameters or fewer.

- [ ] **Step 2: Confirm red:** `cargo test -p iroh-tunnel-engine --all-features --locked access::run_access::tests::` → compile failure naming `udp_forward_loop`, `cached_conn_id` and `RedialBackoff::waiting_until` usage in the six new tests (the existing tests must still be selected by the filter).
- [ ] **Step 3: Split and back-off gate (4.1).** Extract `udp_forward_loop` from `udp_listen_loop` (Contracts) with no behavior change. After `recv_from`, acquire the `UdpDialGate` (inside the existing shutdown `select!`, so stop still wins), and while holding it: check the shared back-off, then `get_or_dial` and the adoption check of Step 4; release it before encoding and sending the datagram. Back-off results: `Waiting { first: true, resumes_in }` → `tracing::warn!(svc, "udp datagram dropped: serve peer cannot carry datagrams, dialing resumes in {}s", resumes_in.as_secs())`, `continue`; `Waiting { first: false, .. }` → debug, `continue`. No `record_drop`.
- [ ] **Step 4: Adoption check (4.2).** Change `current` to `Option<(Connection, Incarnation)>` (Contracts). In the branch where `current` is `None` or holds a connection whose `stable_id()` differs from the dialed one, first retire old readers (`readers.abort_all()` and join, as today), then `let incarnation = backoff.begin();` (so any stale token is now ignored). If `!carries_datagrams(conn.max_datagram_size())`: `tracing::error!(peer = %conn.remote_id(), svc, "udp datagrams unsupported by serve peer, closing the connection")`, `conn.close(..)`, `backoff.record(incarnation, now)`, `dialer.invalidate(&conn).await`, `current = None`, `continue`. Otherwise set `current = Some((conn.clone(), incarnation))` and proceed as today.
- [ ] **Step 5: Received close (4.3).** `relay_peer_replies` gets `incarnation` and `ctx` (Contracts). Loop on `match conn.read_datagram().await`; on `Err(error)`: if `closed_for_unsupported_datagrams(&error)`, error log naming `dialer.svc_name` and `conn.remote_id()` ("serve peer closed the udp connection: datagrams unsupported"), then acquire the `UdpDialGate`, `backoff.record(incarnation, now)`, `dialer.invalidate(&conn).await` (evicts only if still current), release. A reader aborted while waiting for the gate (its incarnation was replaced) records nothing, which is correct. Either order on the gate is safe: if the loop wins, it gets the dead cached connection, its send fails and the send path records the same incarnation; if the reader wins, the loop then sees `Waiting`. On the send-error path: if `conn.close_reason()` matches the predicate, `backoff.record(current incarnation, now)` before the existing invalidate; no second error log. Because `record` is idempotent per incarnation, reader-first and send-first orders yield one window.
- [ ] **Step 6: Send decision (4.4).** Replace the inline limit branches with `send_decision(conn.max_datagram_size(), n)`; keep counters and log texts.
- [ ] **Step 7: Verify (4.5):** `cargo test -p iroh-tunnel-engine --all-features --locked`, `cargo test --test udp_tunnel --all-features --locked` (incl. `forwarding_resumes_after_serve_restarts`), `cargo clippy -p iroh-tunnel-engine --all-targets --all-features --locked -- -D warnings` → all pass.
- [ ] **Step 8: Commit** `feat(access): back off after a udp datagram setup failure (#152)`.

---

## Task 5: Contract doc and drop counter rustdoc — implements 5

**Covers:** 5.1, 5.2, 5.3
**Difficulty:** Easy
**Blocked by:** none

**Files:**
- Modify: `docs/udp-integration-contract.md`
- Modify: `crates/engine/src/observation/types.rs` (`limit_shrink` rustdoc, ~line 78)
- Modify: `crates/host/src/status/schema.rs` (`limit_shrink` rustdoc, ~line 74)
- Modify: `crates/engine/src/udp/types.rs` (`DropReason::LimitShrink` rustdoc, ~line 42; listed in tasks.md group 5)

- [ ] **Step 1: Contract rows.** "Datagrams unusable on the connection": "Detected right after the handshake on either side; that connection is closed with application code `0x1d47` and reason `udp datagrams unsupported`; other connections and services continue. Access does not redial the service for 30 s and drops client datagrams meanwhile" | "Error log on both sides; one warning per wait on Access; no counter". Rename "Limit shrinks mid-session" to "Limit becomes unusable mid-session" (counter `limit_shrink`) and add: "A smaller but usable limit makes larger payloads oversized (`oversize`)". Remove the "Open work" text.
- [ ] **Step 2: Rustdoc.** All three items: "Datagrams dropped because the datagram limit became unusable during the session incarnation."
- [ ] **Step 3: Verify:** `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` clean; `grep -n "became unusable during the session incarnation" crates/engine/src/observation/types.rs crates/host/src/status/schema.rs` → one hit each; `grep -rn "#152" crates docs/udp-integration-contract.md` → no hit.
- [ ] **Step 4: Commit** `docs(udp): describe the datagram setup failure contract (#152)`.

---

## Task 6: Real-binary end-to-end coverage — implements 6

**Covers:** 6.1, 6.2, 6.3, 6.4
**Difficulty:** Hard
**Blocked by:** Task 2, Task 3, Task 4

**Files:**
- Create: `tests/udp_datagram_setup.rs`

**Interfaces:**
- Consumes: `IROH_TUNNEL_TEST_DISABLE_DATAGRAMS`; log texts from Tasks 3-4; `iroh::test_utils::run_relay_server`.

Fixture (copy minimal helpers from `tests/quick_run.rs`; no cross-test imports): roles are `CARGO_BIN_EXE_iroh-tunnel` processes, each with its own `HOME`, `IROH_TUNNEL_STATE_DIR` and stdout log file; single-service roles use quick mode, multi-service roles `run --config`. Milestones, each polled every 100 ms up to 30 s: Serve status file has `node_id`; Access log has "listening for local udp clients" with its address; then the scenario's failure line. Traffic is sent only after the listener milestone. Error assertions check the line contains `ERROR`, the service name and the full peer id. Cleanup: on Unix, stop with SIGTERM and assert a successful exit within 10 s (as `tests/cli_golden.rs::stop`); on Windows, `Child::kill` is used for cleanup only and graceful exit is not asserted (documented limitation; graceful shutdown is covered by the existing in-process tests).

- [ ] **Step 1: `serve_without_datagrams_fails_access_setup`** (Serve with the variable, UDP `dns`). After the listener milestone, send one datagram every 200 ms. Wait for Access "udp datagrams unsupported by serve peer" (ERROR, `dns`, Serve id) and Serve "access peer closed the udp connection: datagrams unsupported" (ERROR, `dns`, Access id). Keep sending for 5 s after that milestone. Assert: target received 0 datagrams; both processes still running; Access log has exactly one "connected to serve peer" line and exactly one "dialing resumes in" warning, and no "connect attempt" line after the failure milestone.
- [ ] **Step 2: `access_without_datagrams_is_closed_by_serve`** (Access with the variable). Serve via `run --config` with `dns` (udp, echo target) and `web` (tcp, echo target). Same traffic and milestones with roles swapped: Serve "udp datagrams unsupported by access peer" (ERROR, `dns`, Access id); Access "serve peer closed the udp connection: datagrams unsupported" (ERROR, `dns`, Serve id). Assert: UDP target received 0; a TCP round trip through a second normal Access (own log, excluded from the counts) to `web` succeeds; both processes running; the disabled Access log has exactly one "connected to serve peer" line, exactly one "dialing resumes in" warning, and no "connect attempt" line after the failure milestone.
- [ ] **Step 3: `one_failing_udp_service_does_not_stop_another`.** Serve A (with the variable) offers UDP `bad`; Serve B (normal) offers UDP `good`; one normal Access via `run --config` with `bad` → A and `good` → B. Send to `bad` until the Access setup-failure line for `bad` and its "dialing resumes in" warning appear; keep sending to `bad` while doing 3 round trips through `good`, all of which succeed. Then wait one status flush interval plus margin (`STATUS_FLUSH_INTERVAL` + 2 s; the file is rewritten only on change, so do not wait for a new version) and assert, in the latest Access status file, every drop counter of `bad` is 0. The setup-failure and warning milestones prove the failure path was exercised.
- [ ] **Step 4: Run:** `cargo test --test udp_datagram_setup --all-features --locked` → PASS, each test under 90 s.
- [ ] **Step 5: Platform note.** If a Windows CI run shows the log milestones are flaky, raise the poll deadline, never the assertions.
- [ ] **Step 6: Full gate** (Global Constraints) → all green; `tests/udp_tunnel.rs`, `tests/quick_run.rs`, `tests/cli_golden.rs` pass unchanged.
- [ ] **Step 7: Commit** `test(udp): cover datagram setup failure with real binaries (#152)`.

---

## Scenario → test

`udp-datagram-forwarding`:
- Usability boundary → `carries_datagrams_at_the_boundary` (T1)
- Serve peer does not allow datagrams → `serve_without_datagrams_fails_access_setup` (T6), `adoption_check_closes_and_backs_off` (T4)
- Access peer does not allow datagrams → `access_without_datagrams_is_closed_by_serve` (T6)
- Failure is isolated to the connection → `access_without_datagrams_is_closed_by_serve` (TCP on the same Serve; T6)
- Setup failure does not count as limit shrink → `one_failing_udp_service_does_not_stop_another` (T6)
- Serve receives the close → `serve_without_datagrams_fails_access_setup` (T6)
- Access receives the close → `access_without_datagrams_is_closed_by_serve` (T6), `reader_close_evicts_and_resumes_after_one_window` (T4)
- Limit lost mid-session → `send_decision_drops_as_limit_shrink_when_unusable` (T1, unit level by design D8)
- Smaller usable limit counts as oversize → `send_decision_drops_as_oversize_over_a_usable_limit` (T1)
- Limit reported as absent → existing absent-limit snapshot test in `udp/types.rs`
- No immediate redial → `backoff_waits_30_seconds_and_warns_once` (T1), `adoption_check_closes_and_backs_off`, `reader_close_racing_a_send_cannot_redial` (T4), one-connect assertion in `serve_without_datagrams_fails_access_setup` (T6)
- One warning per wait → `backoff_waits_30_seconds_and_warns_once`, `backoff_ignores_a_repeat_of_the_same_failure` (T1), one-warning assertions (T6)
- Drops during the wait are not counted → `one_failing_udp_service_does_not_stop_another` (T6)
- Dialing resumes after the wait → `reader_close_evicts_and_resumes_after_one_window` (T4), `backoff_waits_30_seconds_and_warns_once` (T1)
- Other services are unaffected → `one_failing_udp_service_does_not_stop_another` (T6)
- Serve restarts (modified requirement) → existing `forwarding_resumes_after_serve_restarts`

`workspace-boundaries`:
- Mid-session meaning visible in API docs → T5 Step 3 (positive grep plus doc build)
- Seam disables datagrams in one role → T6 Steps 1-2
- Production build ignores the variable → `datagram_disable_seam_is_test_utils_only` (T2) plus `cargo build -p iroh-tunnel-engine --locked`
