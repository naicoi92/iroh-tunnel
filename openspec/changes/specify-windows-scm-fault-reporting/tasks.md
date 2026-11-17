## 1. Session status-report scenarios

Blocked by: none

Owns `src/scm_runtime/session.rs` (tests module only; production code changes only if a test exposes a gap). Platform: compiled and run on every OS; the dispatcher binding is Windows-only and checked with `cargo xwin clippy`.

- [x] 1.1 Add a test that a refused stop-pending report is logged with its state and checkpoint (scenario "Refusal is logged"), capturing `tracing` output with a scoped subscriber
- [x] 1.2 Extend `pending_reports_are_not_retried` to assert the next tick carries a higher checkpoint (scenario "Pending report refused")
- [x] 1.3 Add a test where a stop request arrives while the running report is being retried, proven by a gate in `FakeHandle` that pauses the first `Running` attempt, for both outcomes (accepted → clean stop; refused → code 101, no start-pending after) (scenario "Stop request while the running report is retried")
- [x] 1.4 Assert in `running_always_refused_stops_with_a_failure_code` that no start-pending report follows the stop-pending one (scenario "Running report always refused")
- [x] 1.5 Rename the existing session tests to scenario names, add log assertions to the running-refused-once and stopped-always-refused tests, and assert in the stop-during-retry test that a fault code wins over a requested stop (exit 130 still reports 101)

## 2. SCM effect-recording scenarios

Blocked by: none

Owns `crates/host/src/service/backend/windows_scm.rs` (tests module only). Platform: pure orchestration, run on every OS with the fake SCM.

- [x] 2.1 Add a test where the started service stops by itself before running: applied effects are enabled, started, disabled, with no stopped (scenario "Windows system install rolls back a service that stopped by itself")
- [x] 2.2 Rename `rolled_back_install_records_the_rollback` to match scenario "Windows system install rolls back a service stuck starting" and `failed_create_after_delete_reports_the_deletion` to match "Windows system reinstall fails after deleting the old service"

## 3. Verification and traceability

Blocked by: 1, 2

Owns `CHANGELOG.md` only if wording needs the codes; otherwise no file. Platform: macOS run plus Windows cross-target clippy; Windows e2e left to CI.

- [ ] 3.1 Run `openspec validate specify-windows-scm-fault-reporting --strict`
- [ ] 3.2 Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo xwin clippy --workspace --all-targets --all-features --locked --target x86_64-pc-windows-msvc -- -D warnings`, and `cargo test --all-features --locked` (with `RUBY` set to the Homebrew portable Ruby)
- [ ] 3.3 Confirm every scenario in `specs/` maps to a passing test per plan.md "Scenario → test", and record in the apply receipt the parts covered only by review (dispatcher process exit, ticker-spawn skip, log-open conditions), the indirect recovery coverage, and the `#[cfg(unix)]` tests
