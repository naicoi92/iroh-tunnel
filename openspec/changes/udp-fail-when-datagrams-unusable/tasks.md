## 1. Pure UDP setup and send policy in `engine::udp`

Blocked by: none

Owner: one writer for new files under `crates/engine/src/udp/` and `crates/engine/src/udp/mod.rs`. Platforms: pure code, unit tests run on Linux, macOS and Windows CI.

- [x] 1.1 Write failing unit tests for the usability classifier for scenario "Usability boundary": absent, 4 and 5 byte maxima classify as unable, unable, able; the classifier is defined through `payload_limit`
- [x] 1.2 Add the classifier, the dedicated application close code and the reason `udp datagrams unsupported` as one module (design D2, D5); make 1.1 pass
- [x] 1.3 Write failing unit tests for the close predicate: an application close with the dedicated code matches; another application code, a locally closed connection and a transport-level close do not; then implement the predicate over `iroh::endpoint::ConnectionError`
- [x] 1.4 Write failing unit tests for the per-datagram send decision for scenarios "Limit lost mid-session" and "Smaller usable limit counts as oversize": unusable observation gives drop as `limit_shrink` with no close requested, payload over a usable limit gives drop as `oversize`, payload within the limit gives send; then implement it (design D8)
- [x] 1.5 Write failing unit tests for the redial back-off type for scenarios "No immediate redial", "One warning per wait" and "Dialing resumes after the wait", driven by explicit instants: waiting before 30 s, eligible at exactly 30 s, only the first drop of a window asks for a warning, a new failure starts a new window; then implement it (design D4, D8)
- [x] 1.6 Confirm the existing absent-limit snapshot test in `crates/engine/src/udp/types.rs` still passes (scenario "Limit reported as absent"); run `cargo test -p iroh-tunnel-engine --all-features --locked udp::`, `cargo fmt --all -- --check` and the workspace layout test. Clippy with `-D warnings` runs in 4.5, once every crate-private item has its consumer

## 2. Datagram-disable test seam

Blocked by: none

Owner: one writer for `crates/engine/src/testing.rs`, `crates/engine/src/endpoint/create.rs` and `tests/workspace_layout.rs`. Platforms: shared code; the source check runs on every CI platform.

- [x] 2.1 Add a `test-utils`-only reader of `IROH_TUNNEL_TEST_DISABLE_DATAGRAMS` in `crates/engine/src/testing.rs`, and in `transport_config` set `datagram_receive_buffer_size(None)` when it returns true, inside `#[cfg(feature = "test-utils")]` only (design D7)
- [x] 2.2 Add a source check to `tests/workspace_layout.rs` for scenario "Production build ignores the variable": the variable name appears in engine sources only inside the `test-utils`-gated reader in `testing.rs`
- [x] 2.3 Run `cargo build -p iroh-tunnel-engine --locked` (no `test-utils`) and `cargo test --test workspace_layout --all-features --locked`

## 3. Serve setup check and close reporting

Blocked by: 1

Owner: one writer for `crates/engine/src/serve/run_serve.rs`. Platforms: shared code; covered by group 6 on all CI platforms.

- [ ] 3.1 At the top of `handle_udp_connection`, classify `conn.max_datagram_size()`; when unusable, log an error naming the service and peer, close the connection with the dedicated code and reason, and return before any flow table, sweep or reader task exists (scenario "Access peer does not allow datagrams", design D3)
- [ ] 3.2 When the datagram loop ends on a `read_datagram` error that matches the close predicate, log an error naming the service and the Access peer instead of the debug line (scenario "Serve receives the close")
- [ ] 3.3 Route the per-reply decision in `relay_upstream_replies` through the group 1 send decision, keeping the `limit_shrink` and `oversize` counters, warning and `set_datagram_limit` behavior
- [ ] 3.4 Run `cargo test -p iroh-tunnel-engine --all-features --locked` and `cargo test --test udp_tunnel --all-features --locked` (clippy runs in 4.5, after the back-off has its consumer)

## 4. Access setup check, close reporting and back-off

Blocked by: 1

Owner: one writer for `crates/engine/src/access/run_access.rs`. Production `ServiceDialer` methods and policy and TCP paths stay unchanged; private read-only `#[cfg(test)]` observers are permitted. Platforms: shared code; covered by group 6 on all CI platforms.

- [ ] 4.1 Give `udp_listen_loop` a shared back-off value (group 1 type); before dialing, drop client datagrams while it is waiting, warn once per window with the time dialing resumes and log later drops at debug, without recording any drop counter (scenarios "No immediate redial", "One warning per wait", "Drops during the wait are not counted")
- [ ] 4.2 When the loop adopts a new connection, classify `conn.max_datagram_size()` first; when unusable, log an error naming the service and peer, close it with the dedicated code, invalidate it in the dialer, start the back-off and drop the datagram (scenario "Serve peer does not allow datagrams")
- [ ] 4.3 After the `relay_peer_replies` read loop ends, check the close reason with the predicate; on a match log an error naming the service and the Serve peer, record the failure for that connection in the back-off and invalidate that connection in the dialer, so the first datagram after the wait dials afresh (scenarios "Access receives the close", "Dialing resumes after the wait"). On the send-error path, apply the same predicate before invalidating. Recording the same connection's failure twice must not restart its window
- [ ] 4.4 Route the per-datagram decision in `udp_listen_loop` through the group 1 send decision, keeping existing counters and logs
- [ ] 4.5 Run `cargo test -p iroh-tunnel-engine --all-features --locked` and `cargo clippy -p iroh-tunnel-engine --all-targets --all-features --locked -- -D warnings` (first clippy gate: group 4 consumes every group 1 item)

## 5. Contract doc and drop counter rustdoc

Blocked by: none

Owner: one writer for `docs/udp-integration-contract.md`, `crates/engine/src/observation/types.rs` (rustdoc of `limit_shrink` only), `crates/host/src/status/schema.rs` (rustdoc of `limit_shrink` only) and `crates/engine/src/udp/types.rs` (rustdoc of `DropReason::LimitShrink` only, the engine-internal twin of the same counter). Platforms: docs only.

- [ ] 5.1 Rewrite the failure-table row "Datagrams unusable on the connection" to the implemented behavior: connection closed at setup with the dedicated code, error logs on both sides, Access waits 30 s, no counter; keep "Limit shrinks mid-session" as the `limit_shrink` row and state that a smaller usable limit counts as `oversize`
- [ ] 5.2 Rewrite the `limit_shrink` rustdoc in both crates for scenario "Mid-session meaning visible in API docs": drops because the limit became unusable mid-session, no #152 reference
- [ ] 5.3 Run `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` and `grep -rn "#152" crates docs` (expect no hit in the `limit_shrink` docs or the contract row)

## 6. Real-binary end-to-end coverage

Blocked by: 2, 3, 4

Owner: one writer for a new `tests/udp_datagram_setup.rs`. Uses real role binaries (`CARGO_BIN_EXE_iroh-tunnel`, built with `test-utils` under `--all-features`) and the in-process relay, with bounded log polling. Platforms: Linux, macOS and Windows CI.

- [ ] 6.1 Serve started with `IROH_TUNNEL_TEST_DISABLE_DATAGRAMS=1`, normal Access, client sending: assert Access logs the setup error naming the service and peer, Serve logs the received close error, the UDP target receives nothing, both processes stay alive, and Access logs no second dial within a bounded window (scenarios "Serve peer does not allow datagrams", "Serve receives the close", "No immediate redial", "Seam disables datagrams in one role")
- [ ] 6.2 Access started with the variable, normal Serve with a UDP and a TCP service: assert Serve logs the setup error, Access logs the received close error, the TCP service through the same Serve still forwards, and both processes stay alive (scenarios "Access peer does not allow datagrams", "Access receives the close", "Failure is isolated to the connection")
- [ ] 6.3 One normal Access with two UDP services, one to a Serve with the variable and one to a normal Serve: the healthy service keeps forwarding while the failing one waits; the status file of the failing service shows `limit_shrink` 0 and no other drop counter raised by the wait (scenarios "Other services are unaffected", "Setup failure does not count as limit shrink", "Drops during the wait are not counted")
- [ ] 6.4 Run the full gate: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, `cargo test --all-features --locked` (including unchanged `tests/udp_tunnel.rs` and `tests/quick_run.rs`), `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`, and `cargo +1.91 test --workspace --all-features --locked` for MSRV
