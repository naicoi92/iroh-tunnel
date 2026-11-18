# Verification Report

**Change**: `specify-windows-scm-fault-reporting`
**Verified at**: `2026-11-17T15:16Z`
**Iteration**: `1`
**Verifier**: parent session (inline checks; no verifier child dispatched)

---

## 1. Structural Validation (`openspec validate <change>`)

- [x] The command passes for this change

**Result**:

```text
$ openspec validate specify-windows-scm-fault-reporting --strict
Change 'specify-windows-scm-fault-reporting' is valid
exit 0
```

---

## 2. Task Completion (`tasks.md`)

- [x] Every box is ticked (10 ticked, 0 open; `openspec instructions apply` progress: 10/10)

| Open task | Reason | Blocks archive? |
|---|---|---|
| — | — | — |

---

## 3. Requirement Evidence

- [x] ADDED / MODIFIED: implementation evidence (file / line)
- [x] REMOVED: not applicable (none)
- [x] RENAMED: not applicable (none)

| Requirement | Operation | Status | Evidence (file:line) or reason |
|---|---|---|---|
| Windows service logs refused status reports | ADDED | Pass | `src/scm_runtime/session.rs:115` (`warn!` with `Status::describe`, state and pending checkpoint); log open after `init_tracing_with` `src/scm_runtime/dispatcher.rs:219` |
| Windows service retries running and stopped reports | ADDED | Pass | `session.rs:15` (`REPORT_ATTEMPTS = 3`), `:122` `send_retrying`, `:177` `on_ready` → `begin_stop` `:146` with code 101 `:61`; pending not retried `:152`, `:198`; `abandon` `:236` does not resend after `StatusReport` |
| Windows service internal faults end the service with fixed codes | ADDED | Pass | `session.rs:60-62` (100/101/102), `:100-102` poisoned lock → `Fault::StatePoisoned` (no `into_inner` in `src/scm_runtime*`); `dispatcher.rs:220-222` ticker spawn failure → `Fault::TickerSpawn` before `run_to_code`; `dispatcher.rs:86-87` process exit |
| Windows service fault codes take precedence over the exit mapping | ADDED | Pass | `session.rs:210-217` (recorded fault replaces `exit_to_scm`); recovery: `crates/host/src/service/backend/windows_scm.rs:69` (`on_non_crash_failures: true`) |
| Explicit lifecycle operations keep composite semantics | MODIFIED | Pass | `windows_scm.rs:221-256` `RecordingPort` (create→Enabled, delete→Disabled, start→Started, stop→Stopped, on success only); wired at `windows_scm.rs:407` (install) and `crates/host/src/service/system_verb.rs:122` (verbs); rollback failure logged `windows_scm.rs:437` |

---

## 4. Scenario Coverage

- [x] Every scenario maps to a test
- [x] The project test command was run fresh in this step

**Test command**: `RUBY=/opt/homebrew/Library/Homebrew/vendor/portable-ruby/current/bin/ruby cargo test --all-features --locked` (cwd: worktree)
**Result**: exit 0; 1068 passed, 0 failed, 4 ignored

Also run fresh: `cargo fmt --all -- --check` (exit 0), `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` (exit 0), `cargo xwin clippy --workspace --all-targets --all-features --locked --target x86_64-pc-windows-msvc -- -D warnings` (exit 0).

| Scenario | Test |
|---|---|
| Refusal is logged | `session.rs:435` `refused_pending_report_is_logged_with_state_and_checkpoint` — ok |
| Running report refused once | `session.rs:445` `running_report_refused_once_is_retried` — ok |
| Running report always refused | `session.rs:458` `running_report_always_refused_ends_with_101` — ok |
| Stopped report refused once | `session.rs:533` `stopped_report_refused_once_is_retried` — ok |
| Stopped report always refused | `session.rs:542` `stopped_report_always_refused_exits_with_101` — ok; process exit itself Not verified (dispatcher, Windows only) |
| Pending report refused | `session.rs:590` `pending_report_refused_is_not_resent_and_next_tick_advances` — ok |
| Stop request while the running report is retried | `session.rs:480` `stop_request_while_running_is_retried_then_stops_cleanly`, `session.rs:502` `stop_request_while_running_keeps_failing_ends_with_101` — ok |
| Poisoned service state | `session.rs:606` `poisoned_service_state_ends_with_100` — ok; process exit Not verified (dispatcher, Windows only) |
| Status ticker cannot start | `session.rs:577` `status_ticker_cannot_start_ends_with_102` — ok; "role does not run" Not verified by test (review of `dispatcher.rs:218-224`) |
| Fault overrides a clean drain | `session.rs:458` (exit 0 → 101) and `session.rs:502` (exit 130 → 101) — ok |
| Fault triggers recovery | `windows_scm.rs:958` `failure_plan_from_default_policy` — ok; e2e `tests/service_system_windows.rs:808` `crash_is_restarted` Not verified (Windows only, not run here; exit 1 stands in for 101) |
| Start refused when the role already runs | `crates/host/src/service/verb.rs:241` `start_is_refused_before_manager_when_role_runs` — ok (`#[cfg(unix)]`) |
| Partial install reports applied effects | `crates/host/src/service/install.rs:1136` `partial_install_reports_written_registration` — ok (`#[cfg(unix)]`) |
| Windows system reinstall fails after deleting the old service | `windows_scm.rs:1076` `reinstall_failing_after_delete_reports_stop_and_delete` — ok |
| Windows system install rolls back a service stuck starting | `windows_scm.rs:1101` `install_rolls_back_a_service_stuck_starting_with_all_effects` — ok |
| Windows system install rolls back a service that stopped by itself | `windows_scm.rs:1118` `install_rolls_back_a_service_that_stopped_by_itself_without_a_stop_effect` — ok |

---

## 5. Design and Contract Coherence (warning only)

- [x] Decisions in design.md match the code — Not applicable: design.md is "No design needed"; the brainstorm decision (fixed codes 100/101/102) matches `session.rs:60-62`
- [x] Contracts in plan.md, with the Contract deviations recorded in apply.md, match the code — plan.md Contracts: "None – tests and spec text only"; apply.md deviations: none; `git diff 5809ac7..HEAD -- src crates` touches only `#[cfg(test)]` modules of `session.rs` and `windows_scm.rs` (matched)

**Drift warnings** (non-blocking):

- None

---

## 6. Clean Tree and Commit Range

- [x] Working tree is clean (`git status --porcelain` empty at `1368d01`)

**Commit range**: `5809ac7..1368d01` (this receipt adds one commit after it)

---

## Overall Decision

- [ ] PASS: proceed with `/opsx-continue` to finalize
- [x] PASS_WITH_WARNINGS: proceed, noting: parts of four scenarios run only on Windows and are Not verified here
- [ ] FAIL: fix the cause, then re-apply (iteration N+1)

### Issues

**CRITICAL**: none.

**WARNING**
- Not verified on this host: the dispatcher's process exit after faults 100/101 (`dispatcher.rs:86-87`), the ticker-spawn skip of `run_to_code` (`dispatcher.rs:218-224`), the service-log open/quiet/untrusted conditions, and the Windows e2e `tests/service_system_windows.rs` (incl. `crash_is_restarted`). Recommendation: confirm the Windows CI job (`service_system_windows`) passes on the PR before merge.
- `#[cfg(unix)]` tests cover the two generic scenarios of the MODIFIED requirement; Windows coverage of them is the effect-recording tests only. Recommendation: none needed for this change; note for future Windows test work.

**SUGGESTION**
- `session.rs:281-282`: doc of `gated_on_first_running` still says "barriers"; reword to the entered/release channels.
- `session.rs:545-549`: assert `"SCM refused status Stopped"` and `"times; giving up"` to pin the final error line.
- `windows_scm.rs:1101`, `:1118`: rollback tests assert only `is_err`; optionally check the error message (`StartPending` / `Stopped`).

**Next step**:

Run `/opsx-continue` with the worktree as working directory to create the finalize artifact, then `/opsx-archive`.
