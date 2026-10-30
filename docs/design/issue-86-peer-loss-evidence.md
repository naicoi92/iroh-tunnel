# Issue #86 — bounded peer loss and no replay

## Pre-implementation contract and test matrix

Baseline `191d531e03dd7003619eb83ada2bcc00a474f68d`. Live #86 and #76 refreshed: #76 closed; #86 blockers #76 and #81 closed. #86 handoff confirms peer loss closes affected channels without replay and new clients may connect after recovery. Numeric retry/drain defaults remain unapproved. This slice excludes retry deadline and shutdown implementation.

Process seam: actual configured Serve and Access children, real TCP clients and target, isolated loopback relay. Existing configured secret-key fixture supplies same Serve identity after restart; no quick-key policy assumed. Test bounds are observation limits, not product defaults.

| Criterion / source | Test / independent expected value |
|---|---|
| TCP-02, #86 AC2 / #76: peer loss closes affected channels without replay; recovery permits new channel | `configured_peer_loss_closes_active_channels_without_replay`: two overlapping channels with distinct literal markers and replies, kill and reap Serve while both remain active, observe each client's EOF/error, restart same configured identity, exact new exchange, reject extra target accepts or unsolicited target bytes during bounded window. Confirm shared connection via Serve log before kill. |

Applicable standards: RS-01 focused fixture, RS-05 contextual errors, RS-08/09 bounded resource cleanup, RS-10 no key logs, RS-13 isolated local relay, RS-14/15 spec-derived actual-process E2E, RS-16 MSRV gates. No claim of full TCP-FAILURE-01, platform matrix or release acceptance.

## Results

Existing production behavior passed first valid run; no production change and no behavioral RED claimed. Matrix criterion covered by test; first run and strengthened causation run both passed on unchanged production. Both active client channels exchanged exact distinct markers and replies before Serve was killed and reaped; both clients and both old target sockets reached EOF/error, Access remained alive. Restarted configured Serve reported the same public identity; new client exchanged exact new marker/reply. Two-second window saw neither additional target connection nor bytes/closure on new target socket. Old target sockets were read to closure before restart, excluding trailing old bytes. Shared connection log counted one peer and stream 2 before loss.

Commands on macOS arm64, uid 501, Rust 1.91.1, 2026-10-29 UTC, locked Iroh/iroh-relay 1.1.0, Tokio 1.53.0:

- `cargo +1.91.1 test --all-features --locked --test configured_multiplex configured_peer_loss -- --nocapture`: passed twice, 44.04s and 44.05s respectively (second includes strengthened process/target closure assertions).
- `cargo +1.91.1 fmt --all -- --check`: passed.
- `cargo +1.91.1 clippy --all-targets --all-features --locked -- -D warnings`: passed.
- `cargo +1.91.1 test --all-features --locked`: 145 passed, 5 ignored, 0 failed. Ignored: authentic 0.4 interoperability and four public relay tests. Log `/tmp/issue86-peer-full.log`.
- `git diff --check`: passed.

Test-feature executable SHA-256 `e59c5b3f2eb33d7794f54d37cdf6078e3d226ebde1522b45dd88336f1be9de76`. Processes used `serve run --config` and `access run --config`, temporary configured keys, local target and local test relay only. Test TLS override is insecure and not production. Runtime path not forced; successful local relay configuration does not prove relay-only traffic. Test waits up to 35 seconds for each client and 100 seconds for scenario; these are fixture limits, not approved retry defaults. No claim about arbitrary future replay, in-flight bytes, retry expiry, graceful shutdown, public relay, production TLS or other platforms.

Normal and scenario-error paths kill/reap roles with five-second polling and shut down relay with five-second bound; existing process drop guard is fallback for early setup failures or panics. Independent Standards and Spec reviews remain required; no PR, push, merge or delegation. This is partial #86 evidence, not full issue or release acceptance.
