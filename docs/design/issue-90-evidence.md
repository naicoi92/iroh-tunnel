# Issue #90 — install and manage a user service

Baseline `89ee63a` (origin/main). Branch `redesign/issue-90-user-service`.
Blockers #84, #87 closed at pickup. #79/#92 (Windows) remain out of scope.

## Acceptance map

| Criterion | Test (`tests/service_user.rs` unless noted) |
|---|---|
| 90.AC1 lifecycle with pinned absolute paths (SM-USER-01) | `user_service_lifecycle_uses_the_pinned_absolute_paths` (real launchd/systemd --user, opt-in `IROH_TUNNEL_E2E_SERVICE=1`) |
| 90.AC2 missing permission, no silent change (SM-PERMISSION-01) | `denied_registration_fails_with_guidance_and_keeps_scope`, `system_scope_operations_without_privilege_are_refused_with_guidance` |
| 90.CL1 interactive user/system, unprivileged system refused (SM-SCOPE-01) | `interactive_install_offers_user_or_system_and_system_is_refused_unprivileged`, `system_install_without_privilege_stops_with_rerun_guidance`; unit tests in `src/service/scope.rs` |
| Ownership rule (#77) | Covered by #87 (`tests/role_ownership.rs`); `service start` refusal unchanged |

## Evidence

- Red: all five non-lifecycle tests failed on the baseline (no `--user`, exit 1, no guidance).
- Green: `cargo test` full suite, `cargo clippy --all-targets -D warnings`, `cargo fmt` on macOS (Darwin 24.6, launchd `gui/501`).
- Lifecycle ran against the real macOS user launchd with a unique test suffix (`test-utils`), CWD `/`, config path containing a space; install, status, stop (status file removed), start, restart (same node id), uninstall (plist removed).

## Limits

- Linux systemd `--user` and BusyBox backends compile and have template unit tests only; not run on a real host here (no systemd available).
- Windows: no service backend (gated by #79/#92).
- Real root/system install not exercised (no privilege); privilege gating is exercised unprivileged only.
- Retarget confirmation of another user's service (SM-02 second half) and unknown-user selection belong to #91.

## Review disposition (head reviewed: `d15469e`)

Independent Standards and Spec reviews: no P0; one P1, fixed.

- P1 BusyBox refusal exited 1 and implied `--system`: now `CliError::Permission` (exit 3), no silent scope change. Compiled only by reading (no Linux cross toolchain locally); needs Linux CI.
- P2 fixed: reuse `scope::is_elevated`/`libc::getuid`, stale launchd docs and hint.
- P2 noted, not changed: `restart`/`install` do not call the ownership check (0.4.2 lifecycle, spec says only start is refused); registration write is not atomic; role still passed as string in `check_installable_config`; stdin prompt is a one-shot blocking read.
- Permission test covers a denied home directory only, not inaccessible config/state or ACL before/after evidence.

## Re-review disposition (head reviewed: `0ca75c8`)

Independent Standards and Spec re-review: no P0/P1. Verdict OK with notes.

- Fixed: systemd `Environment=` must not double `$` (only `ExecStart=` expands it); stale test comment; `uid()` doc wording. Covered by a unit test; the quoting logic was also run standalone because `systemd.rs` is not compiled on macOS.
- Open, deferred: non-atomic registration write (RS-11); string role in `check_installable_config`; blocking stdin read; rerun-command builder could move into `scope.rs`; `(role, config, state)` could become an `InstallSpec`.
- Spec notes: non-interactive unprivileged install defaults to `--user` (not spec-backed, confirm intent); `sudo` install pins root's paths (SM-02, #91); `check_installable_config` is an added guard.
