# Verification Report

**Change**: `busybox-script-hardening`
**Verified at**: `2026-11-17 11:09`
**Iteration**: `1`
**Verifier**: parent session (fresh test runs, structural/task/tree checks) + read-only verifier agent `code-reviewer` 44fc610f (requirement, scenario and contract evidence; spot-checked by the parent)

---

## 1. Structural Validation (`openspec validate busybox-script-hardening`)

- [x] The command passes for this change

**Result**:

```text
Change 'busybox-script-hardening' is valid   (exit 0)
```

---

## 2. Task Completion (`tasks.md`)

- [x] Every box is ticked, or each open box has a recorded reason

13 of 13 ticked (`openspec instructions apply` progress: total 13, complete 13, remaining 0).

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
| BusyBox init script content and lifecycle | MODIFIED | Pass | `busybox_script.rs:37-39` paths, `:86-96` variables, `:157-205` verbs (SIGTERM, 5 s, re-check before SIGKILL `:179-182`), PID file `:166`; `busybox.rs:52-58` mode 0755, `:156` restart |
| BusyBox init script values are shell literals | ADDED | Pass | `busybox_script.rs:70-76, 90-96` via `policy.rs:110` `shell_quote`; pin/uid lines `:79-84` |
| BusyBox install refuses non-UTF-8 paths | ADDED | Pass | `busybox_script.rs:46-54` (`CliError::Config`), `busybox.rs:146` render before `:148` write; `src/app.rs:111` → exit 2 |
| BusyBox init script verifies process identity | ADDED | Pass | `busybox_script.rs:110-112` (` (deleted)`), `:145-149`; status never writes `:188-196` |
| BusyBox init script discards stale PID files | ADDED | Pass | `:136` malformed/0, `:137-140` dead, `:144` zombie, `:147` other exe; start `:163`, stop `:185`, status exit 3 `:195` |
| BusyBox init script refuses an unverifiable identity | ADDED | Pass | returns 2 at `:134, :139, :142, :145`; start exit 1 `:161`, stop exit 1 `:172`, status exit 4 `:192`; `restart) stop && start` `:201` |
| BusyBox init script recognises legacy PID files | ADDED | Pass | `busybox_script.rs:116-127` (current DAEMON ± ` (deleted)`, `iroh-tunnel*` + cmdline tail, combined `:123`, standalone `:124`, unreadable cmdline → 2 `:120`) |
| CLI start and restart re-check the registered binary | MODIFIED | Pass | `effective_binary.rs:100-151`; `busybox.rs:113`; `verb.rs:253` Unknown → Permission (exit 3, `app.rs:112`) |

---

## 4. Scenario Coverage

- [x] Every scenario maps to a test
- [x] The project test command was run fresh in this step

**Test command**: `RUBY=/opt/homebrew/Library/Homebrew/vendor/portable-ruby/current/bin/ruby cargo test --all-features --locked` (macOS, worktree, HEAD e0c756c)
**Result**: exit 0 — 979 passed, 0 failed, 4 ignored. Also fresh: `cargo fmt --check` exit 0; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` exit 0; Linux docker (rust:1.91-bookworm, dash) clippy workspace + `cargo test -p iroh-tunnel-host --all-features --locked service::backend` exit 0, 87 passed; privileged Linux E2E `IROH_TUNNEL_E2E_PRIVILEGED=1 cargo test --all-features --locked --test service_system -- --test-threads=1` exit 0, 11 passed.

Tests below are in `crates/host/src/service/backend/busybox_script.rs` unless prefixed.

| Scenario | Test |
|---|---|
| BusyBox paths and stopped status | `format_script_embeds_paths_and_all_verbs` :231, `status_without_pid_file_is_not_running` :742 |
| Script verbs | `format_script_embeds_paths_and_all_verbs` :231 |
| Start records the launched executable | `start_records_pid_and_daemon` :709 |
| Stop of the running daemon | `stop_terminates_the_recorded_daemon` :719, `stubborn_daemon_is_killed_after_grace` :729 |
| PID reused during the stop grace window | `pid_reused_during_grace_is_not_killed` :916 |
| Config path with shell metacharacters | `metachar_config_path_reaches_daemon_literally` :642 |
| State and binary paths with shell metacharacters | `metachar_binary_and_state_paths_are_literal` :661 |
| Non-UTF-8 path refused | `non_utf8_config_is_refused` :319 (render level; see WARNING 2) |
| Running daemon replaced on disk | `replaced_daemon_with_deleted_suffix_is_ours` :797 |
| Stale PID file names an unrelated live process | `stale_pid_of_unrelated_process_is_not_signalled` :748 |
| Start with a stale PID file | `start_with_stale_pid_launches_daemon` :757 |
| Status with a stale PID file | `status_with_stale_pid_reports_not_running` :771 |
| Dead or malformed PID file | `stale_garbage_pid_file_is_removed_without_signal` :779, `zombie_daemon_is_stale` :860 |
| Unverifiable identity | `unverifiable_identity_fails_loud` :810, `missing_exe_link_is_unverifiable` :849, `running_state_without_exe_link_is_unverifiable` :883, `unreadable_pid_file_is_unverifiable` :893, `legacy_pid_with_unreadable_cmdline_is_unverifiable` :948 |
| Status of a root daemon by a non-root user | `unsignalable_pid_with_proc_entry_is_unverifiable` :825 (simulated EPERM) |
| Legacy PID file of the same binary | `legacy_pid_of_current_daemon_is_restarted` :928 |
| Legacy PID file after a binary path change | `legacy_pid_after_binary_change_is_stopped` :959 |
| Legacy PID file of an unrelated process | `legacy_pid_of_unrelated_process_is_kept` :991 |
| Re-install migrates an existing registration | `reinstall_over_legacy_registration_replaces_old_daemon` :1089 + privileged E2E `tests/service_system.rs` (re-install/restart flows, 11 passed) |
| Restart after an upgrade of a managed install | existing `verb.rs:1126 restart_never_writes_a_copy` |
| Effective definition per backend | `effective_binary.rs` busybox tests, `effective_binary_round_trips_the_generated_script` :307 |
| Loaded definition differs from the file | existing `verb.rs:859, :1089` |
| Trusted registered binary | existing `verb.rs:1026 trusted_effective_binary_is_started` |
| BusyBox script generations | `effective_binary.rs` `busybox_single_quoted_daemon_yields_exact_path`, `busybox_legacy_double_quoted_daemon_still_parses` |
| BusyBox DAEMON with shell expansion | `effective_binary.rs:353 busybox_daemon_with_expansion_is_unknown` + `verb.rs:1043 unknown_definition_is_permission` |

---

## 5. Design and Contract Coherence (warning only)

- [x] Decisions in design.md match the code (D1 `shell_quote`; D2 Config error before write; D3 `busybox_script.rs` seam with `backend/mod.rs:5-6` cfg gate; D4 three-way identity, zombie stale; D5 both DAEMON generations)
- [x] Contracts in plan.md, with the Contract deviations recorded in apply.md, match the code

| Contract | Result |
|---|---|
| `InitScript<'a>` | Matched (`busybox_script.rs:15-24, 46-62`) |
| `ScriptLayout` + `for_role` | Matched (`:27-42`) |
| `format_script`, `format_script_invocation` removed | Matched (no remaining definitions) |
| BusyBox init script variables | Matched (`:70-96`) |
| BusyBox PID file format | Matched (`:166`, read `:103-107`) |
| BusyBox init script exit statuses | Matched (`:157-196`) |
| `parse_busybox_script` | Matched (`effective_binary.rs:107-151`) |
| apply.md "Contract deviations: none" | Matched for shapes; see drift warnings |

**Drift warnings** (non-blocking):

- `busybox_script.rs:134` `[ -r "$PIDFILE" ] || return 2` (unreadable PID file → unverifiable) and `:120` (unreadable legacy cmdline → 2, plan draft said 1) are behaviour refinements, both covered by tests and consistent with the spec's "unreadable identity unverifiable", but not listed in design D4 or apply.md.

---

## 6. Clean Tree and Commit Range

- [x] Working tree is clean (`git status --porcelain` empty before writing this receipt)

**Commit range**: `b35bde1..e0c756c` (base `82d7d92`)

---

## Issues

**CRITICAL**: none.

**WARNING**
1. Design/plan drift (check 5): unreadable PID file and unreadable legacy cmdline are unverifiable in code but absent from design D4/plan. Recommendation: add one line to design.md D4 step 3 ("an unreadable PID file or legacy cmdline is unverifiable") before archive.
2. "Non-UTF-8 path refused" is tested at render level only; the exit-code-2 / no-script path through `install` is argued from code order (`busybox.rs:146-148`, `app.rs:111`). Recommendation: if a privileged E2E is extended later, add a non-UTF-8 config case to `tests/service_system.rs`.

**SUGGESTION**
1. `docs/services.md:160` says "single-quoted shell literal", but plain paths are written unquoted; say "shell literal".
2. Parked from final review: new-format identity compares only `exe`; also match `cmdline` when readable (follow-up issue).
3. Parked: `uninstall` silently tolerates a failing `stop` (unverifiable); emit a warning note (follow-up, #225 overlap).
4. Real NanoKVM/BusyBox hardware run remains a manual E2E check.

---

## Overall Decision

- [ ] PASS: proceed with `/opsx-continue` to finalize
- [x] PASS_WITH_WARNINGS: proceed, noting: design/plan drift on two unverifiable refinements and render-level-only non-UTF-8 test; no CRITICAL, all fresh tests green.
- [ ] FAIL: fix the cause, then re-apply (iteration N+1)

**Next step**:

Run `/opsx-continue` in this worktree to create the finalize artifact, then `/opsx-archive`.
