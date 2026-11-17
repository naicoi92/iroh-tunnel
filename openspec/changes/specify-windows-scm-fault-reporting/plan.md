# Windows SCM Fault Reporting Spec Traceability Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** Every scenario of the new and modified requirements maps to a passing test against the behaviour already shipped in `c0dcf47`/`d317572`.

**Architecture:** No production change is expected. `Session` (`src/scm_runtime/session.rs`) is the pure status-reporting unit tested with `FakeHandle`; `RecordingPort` and `install_with` (`crates/host/src/service/backend/windows_scm.rs`) are tested with `FakeScm`/`FakeClock`. The Windows dispatcher binding stays covered by cross-target clippy and the Windows e2e on CI.

**Tech Stack:** Rust 2021, `tracing` + `tracing-subscriber` (already dependencies), std threads and `Barrier` for the race test, OpenSpec CLI.

---

## Contracts

None – the change adds tests and spec text only; no type, signature, CLI argument, config key or file format changes. Codes 100–102 already exist as `Fault::code` and become a documented contract, not a new shape.

---

## Task 1: Session status-report scenarios — implements 1

**Covers:** 1.1, 1.2, 1.3, 1.4, 1.5
**Difficulty:** Normal
**Blocked by:** none
**Files:** modify `src/scm_runtime/session.rs` (tests module)

- [ ] **Step 1:** Add a test helper `capture_logs(f: impl FnOnce()) -> String`: a small `LogBuffer(Arc<Mutex<Vec<u8>>>)` implementing `io::Write` and `MakeWriter` (as in `tests/access_path_logging.rs:57-80`), a `tracing_subscriber::fmt()` subscriber with `.with_writer(buffer).with_ansi(false)`, run `f` under `tracing::subscriber::with_default`, return the text. Write `refused_pending_report_is_logged_with_state_and_checkpoint`: `FakeHandle` refusing pending reports, `capture_logs(|| session.on_stop_control().unwrap())`, assert the text contains `StopPending` and `checkpoint 1`. Run `cargo test --lib scm_runtime::session` → pass (behaviour exists). If it fails, fixing `Status::describe` or the `warn!` call is the one permitted production change.
- [ ] **Step 2:** Rename `pending_reports_are_not_retried` → `pending_report_refused_is_not_resent_and_next_tick_advances`; assert the second attempt's `Status::Pending` checkpoint is greater than the first.
- [ ] **Step 3:** Add an optional one-shot gate to `FakeLog` (no change to the `FakeHandle(..)` tuple): `gate: Option<(Arc<Barrier>, Arc<Barrier>)>`. In `set`, when `status == Status::Running`, `take()` the gate under the lock, drop the guard, wait on barrier 1 ("inside the retry") then barrier 2 ("released"), re-lock and apply the normal refusal logic. Constructor `gated_on_first_running(refusals) -> (FakeHandle, Arc<Barrier>, Arc<Barrier>)`, both barriers with count 2 so the test thread takes part.
- [ ] **Step 4:** Write `stop_request_while_running_is_retried_then_stops_cleanly`: gated handle refusing `Running` once; inside `std::thread::scope`, spawn `on_ready`; wait barrier 1; assert `session.machine.try_lock().is_err()` (the retry holds the lock while the stop control is issued); spawn `on_stop_control`; wait barrier 2; scope joins both. Then `tick()` and `on_finished(0)`. Assert attempts are exactly `[Running, Running, Pending(StopPending cp1), Pending(StopPending cp2), Stopped(Clean)]`: no `Pending` between the `Running` attempts, no `StartPending`.
- [ ] **Step 5:** Write `stop_request_while_running_keeps_failing_ends_with_101`: the same harness, with `Running` refused on all three attempts (the gate pauses only the first). After the scope joins, assert the drain was requested once and exactly one `StopPending` was sent (the stop control adds none). Then `tick()`: assert no `StartPending` and a higher stop-pending checkpoint. Then `on_finished(130)` (a requested stop's exit): assert `Stopped(Specific(101))`, so the fault code wins over a requested stop.
- [ ] **Step 6:** In `running_always_refused_stops_with_a_failure_code` (renamed `running_report_always_refused_ends_with_101`), add a `tick()` before `on_finished` and assert no `StartPending` follows the stop-pending report.
- [ ] **Step 7:** Rename the remaining tests to scenario names and add log assertions with `capture_logs`:
  - `running_report_refused_once_is_retried`: the log contains `Running`;
  - `stopped_report_refused_once_is_retried`;
  - `stopped_report_always_refused_exits_with_101`: the log contains `Stopped` and `giving up`;
  - `poisoned_service_state_ends_with_100`;
  - `status_ticker_cannot_start_ends_with_102`.
- [ ] **Step 8:** `cargo test --lib scm_runtime` all pass; `cargo clippy --all-targets --all-features --locked -- -D warnings` clean; commit `test(scm): trace Windows status-report scenarios to Session tests`.

---

## Task 2: SCM effect-recording scenarios — implements 2

**Covers:** 2.1, 2.2
**Difficulty:** Easy
**Blocked by:** none
**Files:** modify `crates/host/src/service/backend/windows_scm.rs` (tests module)

- [ ] **Step 1:** Write `install_rolls_back_a_service_that_stopped_by_itself_without_a_stop_effect`: `FakeScm::with(None)`, `start_script = vec![StartPending, Stopped]`, run `install_recording`; assert error and `applied == [Enabled, Started, Disabled]`. Run `cargo test -p iroh-tunnel-host --lib windows_scm` → pass.
- [ ] **Step 2:** Rename `rolled_back_install_records_the_rollback` → `install_rolls_back_a_service_stuck_starting_with_all_effects` and `failed_create_after_delete_reports_the_deletion` → `reinstall_failing_after_delete_reports_stop_and_delete`.
- [ ] **Step 3:** Tests pass; clippy clean; commit `test(scm): trace Windows install effect scenarios`.

---

## Task 3: Verification and traceability — implements 3

**Covers:** 3.1, 3.2, 3.3
**Difficulty:** Easy
**Blocked by:** Task 1, Task 2
**Files:** none expected (`CHANGELOG.md` only if the codes need naming)

- [ ] **Step 1:** `openspec validate specify-windows-scm-fault-reporting --strict` → valid.
- [ ] **Step 2:** Run the four commands in tasks 3.2; record results.
- [ ] **Step 3:** Walk "Scenario → test" below; every listed test exists and passes; list the Windows-CI-only scenarios in the apply receipt.

---

## Scenario → test

- Refusal is logged → `scm_runtime::session::tests::refused_pending_report_is_logged_with_state_and_checkpoint`. Not tested: the "service log is open / quiet / untrusted directory" conditions, which live in `dispatcher.rs` (`init_tracing_with`, `log_writer`); review only.
- Running report refused once → `scm_runtime::session::tests::running_report_refused_once_is_retried` (includes the log assertion)
- Running report always refused → `scm_runtime::session::tests::running_report_always_refused_ends_with_101`
- Stopped report refused once → `scm_runtime::session::tests::stopped_report_refused_once_is_retried`
- Stopped report always refused → `scm_runtime::session::tests::stopped_report_always_refused_exits_with_101` (includes the log assertion; the process exit itself is `dispatcher.rs`, review only)
- Pending report refused → `scm_runtime::session::tests::pending_report_refused_is_not_resent_and_next_tick_advances`
- Stop request while the running report is retried → `scm_runtime::session::tests::stop_request_while_running_is_retried_then_stops_cleanly` and `stop_request_while_running_keeps_failing_ends_with_101`
- Poisoned service state → `scm_runtime::session::tests::poisoned_service_state_ends_with_100`. Not tested: that the process exits, which is `dispatcher.rs::abandon`; covered by review of `dispatcher.rs`, no CI test injects poisoning.
- Status ticker cannot start → `scm_runtime::session::tests::status_ticker_cannot_start_ends_with_102`. Not tested: that the role does not run (`dispatcher.rs` returns before `run_to_code`); covered by review, no seam injects a spawn failure.
- Fault overrides a clean drain → `scm_runtime::session::tests::running_report_always_refused_ends_with_101` (the drained run exits 0 and reports 101); `stop_request_while_running_keeps_failing_ends_with_101` adds the requested-stop case (exit 130 still reports 101)
- Fault triggers recovery → `service::backend::windows_scm::tests::failure_plan_from_default_policy` (non-crash failures act) plus `tests/service_system_windows.rs::crash_is_restarted` on Windows CI. Indirect: that e2e ends with code 1, standing in for 101; the SCM treats every service-specific code the same.
- Start refused when the role already runs → `service::verb::tests::start_is_refused_before_manager_when_role_runs` (`#[cfg(unix)]`, not run on Windows)
- Partial install reports applied effects → `service::install::tests::partial_install_reports_written_registration` (`#[cfg(unix)]`; on Linux without systemd it returns early)
- Windows system reinstall fails after deleting the old service → `service::backend::windows_scm::tests::reinstall_failing_after_delete_reports_stop_and_delete`
- Windows system install rolls back a service stuck starting → `service::backend::windows_scm::tests::install_rolls_back_a_service_stuck_starting_with_all_effects`
- Windows system install rolls back a service that stopped by itself → `service::backend::windows_scm::tests::install_rolls_back_a_service_that_stopped_by_itself_without_a_stop_effect`
