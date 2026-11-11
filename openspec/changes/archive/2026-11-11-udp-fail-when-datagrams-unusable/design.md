## Context

UDP forwarding lives in `iroh-tunnel-engine`. Serve handles each accepted UDP-service connection in `handle_udp_connection` (`crates/engine/src/serve/run_serve.rs`); Access forwards local client datagrams in `udp_listen_loop` (`crates/engine/src/access/run_access.rs`). Access dials lazily through `ServiceDialer::get_or_dial`, shared with TCP, and spawns `relay_peer_replies` per adopted connection. Both sides call `udp::payload_limit(conn.max_datagram_size())` per datagram and, on `None`, drop it as `limit_shrink`.

`max_datagram_size()` (noq 1.4) returns `None` when the peer's transport parameters carry no `max_datagram_frame_size`, that is, when the peer has datagrams disabled. A side that disables datagrams locally still sees `Some(..)` for its peer; only the peer notices. Each side therefore detects only the other side's lack of support, so both sides must check. The transport parameters are known once the handshake completes, so the check is reliable right after `accept` or `connect`.

Today's iroh-tunnel endpoints always enable datagrams (`transport_config` in `crates/engine/src/endpoint/create.rs` keeps iroh's default receive buffer), so the failure needs a test seam to reproduce.

## Goals / Non-Goals

**Goals:**
- Classify datagram usability once per UDP-service connection right after the handshake, on both sides, and close an unusable connection with a dedicated application close code.
- Log a clear error on the side that closes and on the side that receives that close.
- Make Access back off for 30 seconds before redialing a service after such a close, without changing the shared dialer.
- Keep `limit_shrink` for mid-session drops only.
- Reproduce the failure end to end with real binaries through a `test-utils`-only seam.

**Non-Goals:**
- Status schema, host API, CLI or config changes.
- Any wire-format change or probe message.
- Changes to TCP forwarding or to `ServiceDialer`.
- Detecting datagrams blocked on the network path while both peers advertise support.

## Decisions

### D1. Bounded context and invariant
The change stays inside the engine's UDP forwarding context. New invariant: **a UDP session incarnation exists only on a connection that can carry datagrams.** Setup enforces it; mid-session shrinkage stays a per-datagram concern (`limit_shrink`). TCP and the dialer, the transport-neutral connection management, are not touched.

### D2. Pure usability rule in `engine::udp`
Add to `crates/engine/src/udp/` a small module holding:
- a pure classifier over `max_datagram_size: Option<usize>` that answers usable/unusable, defined through the existing `payload_limit` so the contract's single rule (`None` or ≤ 4 is unusable) has one implementation;
- the dedicated application close code and reason (`udp datagrams unsupported`) as constants;
- a pure predicate that recognises this close in a `ConnectionError` (application close with that code).

*Why:* the rule and the code are shared by Serve and Access and are unit-testable without a network. *Alternative rejected:* inline checks at each call site duplicate the rule and the code.

### D3. Serve: check at the top of `handle_udp_connection`
Before entering its datagram loop, Serve classifies `conn.max_datagram_size()`. If unusable, it logs an error naming the service and the peer, closes the connection with the D2 code and reason, and returns. No flow table, sweep or reader task is created. The accept loop, other connections and other services are unaffected. The peer tracker and disconnect watcher behave as for any short-lived connection.

### D4. Access: check on adoption, learn of a peer close, back off in the UDP loop
- **Own check.** When `udp_listen_loop` adopts a new connection (the existing `current != Some(conn.stable_id())` branch), it classifies `conn.max_datagram_size()` first. If unusable, it logs an error, closes the connection with the D2 code, invalidates it in the dialer, starts the back-off and drops the triggering datagram.
- **Peer close, Serve side.** When Access closes with the D2 code, Serve's `handle_udp_connection` loop ends on a `read_datagram` error. Today that logs at debug; with the D2 predicate matching, it logs an error naming the service and the peer instead.
- **Peer close, Access side.** When Serve closes, Access learns it in `relay_peer_replies`, whose `read_datagram` loop ends when the connection closes. After that loop, the task checks the close reason with the D2 predicate; on a match it logs an error naming the service and peer and starts the back-off. The send-error path that already invalidates on `ConnectionLost` checks the same predicate, so a datagram that races the reader still starts the back-off.
- **Back-off state.** A shared deadline owned by the service's UDP loop (behind the same kind of `std::sync::Mutex` the loop already uses), set to now + 30 s and keyed by a loop-owned incarnation token begun for each newly seen connection (not `stable_id()`, which is an allocation address that can be reused): recording the same connection's failure again (reader and send path both observing it) does not restart the window. While it lies in the future, the loop drops client datagrams before dialing. Whichever path records the failure also evicts that connection from the dialer, so the first datagram after the wait dials afresh. The lock is never held across an `await`.
- **Reader retirement.** Readers of the previous connection are retired whenever a new connection is dialed, whether it is adopted or rejected by the setup check, so a stale reader cannot touch the back-off of a newer connection.
- **Logs during back-off.** The first dropped datagram of a back-off window logs a warning that says when dialing resumes; later drops in the same window log at debug. No drop counter is incremented.

*Why:* this keeps UDP policy out of the TCP-shared dialer (user decision) and reuses the existing per-connection reader and invalidate paths. *Alternative rejected:* back-off inside `ServiceDialer`, which would make the shared dialer UDP-aware.

### D5. Close code is a wire-visible contract
The close code travels to the peer, so it is a public wire contract between iroh-tunnel versions: one fixed application error code, chosen not to collide with codes already in use (none are used for connection close today), and the reason bytes `udp datagrams unsupported`. An older peer that does not know the code still sees a closed connection and a reason string in its own log. The exact value and its shape go in plan.md Contracts.

### D6. `limit_shrink` semantics
The per-datagram `None` branches in Access and Serve stay, now reachable only when the limit becomes unusable mid-session. Their counter remains `limit_shrink`. A payload over a smaller but still usable limit keeps its existing `oversize` branch. The rustdoc of `limit_shrink` in `crates/engine/src/observation/types.rs` and `crates/host/src/status/schema.rs` drops the #152 deviation sentence, and the contract table row is rewritten.

### D7. Test seam: disable datagrams by environment variable, `test-utils` only
Under `#[cfg(feature = "test-utils")]`, `transport_config` sets `datagram_receive_buffer_size(None)` when `IROH_TUNNEL_TEST_DISABLE_DATAGRAMS=1`. This follows the existing `IROH_TUNNEL_TEST_*` env seams. Because it is per process, real-binary tests can disable datagrams on exactly one role. Production builds compile no reader of the variable.

*Alternative rejected:* the process-global atomic used by `set_relay_only`, which cannot target one role when both run in-process.

The seam is specified in `workspace-boundaries` next to the existing test-only TLS override requirement.

### D8. Verification
- Engine unit tests: classifier (`None`, 4, 5), close predicate (matching code, other code, non-application close).
- Back-off timing: the back-off deadline is a small pure type (start, `is_waiting(now)`, first-drop warning flag) unit-tested with explicit instants: waiting before 30 s, eligible at 30 s, one warning per window. Real-binary tests check only the absence of an immediate redial.
- Mid-session loss cannot be produced by the D7 seam, which acts before the handshake, and no runtime mutation seam is added. Instead, the per-datagram send decision on both sides is extracted into a pure function over the observed limit and the payload size, returning send, drop as `limit_shrink`, or drop as `oversize`. Unit tests drive it with an unusable observation (drop as `limit_shrink`, no close requested) and with a smaller usable limit (drop as `oversize`); the call sites only apply the decision (record the counter, log the warning, continue).
- "Failure is isolated" (Serve side): Access with datagrams disabled fails its UDP service, while a TCP service through the same Serve keeps working.
- "Other services are unaffected" (Access side): one normal Access with two UDP services, one pointing to a Serve with datagrams disabled and one to a normal Serve; the failing service backs off while the other keeps forwarding.
- Real-binary E2E (two directions): Serve with datagrams disabled, so Access detects and closes; and Access with datagrams disabled, so Serve detects and closes and Access logs the received close. Each asserts the error in both logs, that the role processes stay up, that the target never receives a datagram, and that no immediate redial occurs. The back-off duration is not waited out in CI.
- The existing UDP suites (`tests/udp_tunnel.rs`, `tests/quick_run.rs`) pass unchanged, showing the default path is unaffected.

## Risks / Trade-offs

- [Close races a datagram already queued on Access] → the send-error path also checks the close reason, so back-off starts on whichever path observes the close first.
- [A peer with datagrams on but blocked on the path still drains silently] → out of scope; documented as a non-goal and left to the contract's existing per-datagram behavior.
- [Fixed 30 s back-off delays recovery after the peer is fixed] → acceptable for a misconfiguration; it bounds redials, and restarting Access resets it.
- [Log-only visibility] → status field deferred by user decision; can be added later without undoing this design.
- [Env seam leaking into production] → read only under `cfg(feature = "test-utils")`; the root `test-utils` feature already carries a "never enable in production" warning.
- [E2E flakiness from log timing] → assertions poll logs with a bounded wait, as the existing real-binary tests do.

## Migration Plan

No data or config migration. Mixed versions: a new side talking to an old side that supports datagrams behaves as today. Rollback is a plain revert.

## Open Questions

None.
