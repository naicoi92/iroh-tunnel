# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `unix-system-binary-trust`
**Iteration**: `1`
**Applied at**: `2026-11-12 00:58` (2026-11-11T17:58Z)
**Executor**: `subagent-driven-development`

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.unix-system-binary-trust`
- **Worktree branch**: `unix-system-binary-trust`
- **Feature branch**: `spec/unix-system-binary-trust`
- **Base branch**: `main`

---

## Commits

- **Range**: `cf80f4d..<this receipt commit>` (implementation through `cb67bfe`, plus the receipt commits `70bf042` and this one)
- **Count**: `32` implementation commits, plus the receipt commits

---

## Tasks

- **Completed**: `27 of 27` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: none

### macOS manual check (task 7.5)

The developer ran this on an Apple silicon Mac with `sudo`, using the worktree binary copied to the user-owned `~/usbt-bin`, which was first on `PATH`.

- **Install** printed the managed-copy note. `launchctl print system/dev.iroh-tunnel.serve` showed `state = running` and `program = /Library/Application Support/iroh-tunnel/bin/serve/iroh-tunnel`. The copy is owned by `root` with mode `-rwxr-xr-x`.
- **Restart** left the bytes unchanged: the copy and the source have the same `shasum`, `cc60b2d9…`.
- **Uninstall of a running service** found a bug.
  - The first run removed the plist and the job but kept the copy.
  - Diagnosis: `launchctl bootout` returns before launchd drops the job. For at least 0.4 s afterwards, `launchctl print` exits 0 with `state = SIGTERMed` and `program = <copy>`. The reference check therefore kept the copy as still referenced.
  - Fixed in `6655726`: after a successful `bootout`, uninstall waits, bounded to 50 probes 100 ms apart, until `print` reports "Could not find service".
  - Re-run with the fix: exit 0 in 1.1 s, `/Library/Application Support/iroh-tunnel/bin/` is empty (the `serve/` role directory was removed), and launchd has no job.

### Verification evidence (this run)

- macOS: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`, `cargo test --all-features --locked` all pass. `cargo +1.91 check --all-targets --all-features --locked` passes.
- Windows: `cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets --all-features --locked -- -D warnings` passes. It was run with Homebrew LLVM on `PATH` and `AWS_LC_SYS_PREBUILT_NASM=1`. Plain `cargo check --target x86_64-pc-windows-msvc` cannot build `ring` on macOS.
- Linux: a disposable root container (`rust:1.91-bookworm`, no systemd, so the BusyBox backend).
  - `IROH_TUNNEL_E2E_PRIVILEGED=1 cargo test --all-features --locked --test service_system -- --test-threads=1` reports 6 passed: `sm_bin_01` to `sm_bin_05` and the existing `system_service_across_the_privilege_boundary`.
  - clippy with `-D warnings` is clean.
  - An unprivileged BusyBox `service install --user` exits 3 with "Re-run with sudo", and `/var/lib/iroh-tunnel` stays absent.
- Known issues that are not regressions from this change:
  - `service::trust::tests::an_owner_that_is_neither_root_nor_trusted_is_refused` fails when run as root, and fails the same way on `main`.
  - `system_service_across_the_privilege_boundary` fails when re-run in the same container, also on `main`. It leaves the user's config at mode 0666.
  - `update_does_not_stop_a_running_process_of_the_old_binary` failed once in nine runs in the container. That code path is untouched here; it then passed 20 runs in a row.

---

## Contract deviations

All entries are internal (`public: no`) and are recorded in plan.md `## Contracts`.

- `verb::verb_steps`: dropped the planned `role: &str` parameter, because the closures capture the role and the orchestration never reads it.
- `trust::EntryKind`: became `pub(crate)` because `check_entry` takes it.
- `install::existing_registration` seam: implemented as `plan_system_install_with(req, env, lock_dirs, existing, verify)` behind `plan_system_install`. It also takes a `verify` seam.
- `service_binary::release_unreferenced_copy`: a new helper, the one cleanup policy shared by install (`Direct`) and uninstall.
- `verb::check_effective`: a new private function that formats refusals. It prefixes the registered binary path when the trust message does not already name it, so the spec's "names the binary path" holds.
- `install::plan_user_install` and `policy::ensure_root_run_privilege`: new. They refuse an unprivileged root-run (BusyBox) install during planning, keeping the previous exit 3 and text. The final review found this regression.
- `Native::output`: now also compiled on Linux, for the systemd query.
- `platform::effective_binary`, `RegistrationSources` and `managed_copy_referenced`: exist only on Linux and macOS.
- `launchd` private helpers `UnloadPoll`/`UNLOAD_POLL`, `bootout_job`, `wait_until_unloaded`, `print_definition`: new in `6655726`, a bounded wait after `bootout` so uninstall of a running job releases the copy. They are private, and `uninstall` keeps its signature. Recorded after verify W1.

---

## Lanes

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| Task 1 | sequential | `cf80f4d..aa3981d` | approved after 1 fix round (previous session) | n/a (sequential) |
| Task 2 | sequential | `aa3981d..01f418b` | approved after 2 fix rounds (previous session) | n/a (sequential) |
| Task 3 | sequential | `01f418b..9fa506c` | approved (previous session) | n/a (sequential) |
| Task 4 | sequential | `9fa506c..3ce9fdc` | approved (previous session) | n/a (sequential) |
| Task 5 | sequential | `3ce9fdc..b01443d` | APPROVE after 1 fix round | n/a (sequential) |
| Task 6 | sequential | `b01443d..039c000` | APPROVE_WITH_NOTES, notes applied | n/a (sequential) |
| Task 7 | sequential | `039c000..3683b11` | APPROVE_WITH_NOTES after 2 fix rounds | n/a (sequential) |
| Final whole-branch review | sequential | `cf80f4d..0658bb9` | APPROVE_WITH_NOTES; I-1, M-3, nits and the follow-up Windows lint fixed in `40f8f46..0658bb9`. M-1 (a failed cleanup after a `Direct` install reports failure) and M-2 (launchd may keep the copy if the job is still listed right after `bootout`; fail-safe) are left for verify. | n/a (sequential) |
| 7.5 fix (launchd unload wait) | sequential | `70bf042..cb67bfe` | APPROVE_WITH_NOTES, comment notes applied. M-2 is confirmed and fixed. The parent mutation check (removing the wait) fails `bootout_waits_until_launchd_reports_the_job_gone`. Re-verified on a real Mac. | n/a (sequential) |

---

## Next step

Run `/opsx-verify`.
