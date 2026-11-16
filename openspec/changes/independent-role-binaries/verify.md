# Verification Report

**Change**: `independent-role-binaries`
**Verified at**: `2026-11-16`
**Iteration**: `1`
**Verifier**: parent plus read-only `superspec-verifier` (agent `1ad46983-78d6-4b9`)

## 1. Structural Validation

**Pass.** In the authoritative worktree, `openspec validate independent-role-binaries` exited 0: `Change 'independent-role-binaries' is valid`.

## 2. Task Completion

**Pass.** Apply instruction reports 9/9 complete, 0 remaining; `tasks.md` contains nine checked task boxes. The plan's step checkboxes are an implementation ledger, not the task tracking source; they remain unchecked.

## 3. Requirement Evidence

| Requirement | Operation | Result and evidence |
|---|---|---|
| Standalone binaries refuse service management | Removed | Pass: baseline `openspec/specs/cli-entrypoints/spec.md:241`; dispatch now handles standalone `src/app.rs:411-420`. |
| Service management refused for standalone role binaries | Removed | Pass: baseline `openspec/specs/system-service-management/spec.md:78`; same standalone dispatch. |
| Standalone role service commands | Added | Pass: role parsing `src/cli.rs:90-155`, dispatch `src/cli_service.rs:93-104,145-194`. |
| Exit status mapping | Modified | Pass: `src/app.rs:108-117`. |
| Homebrew cask rendering | Modified | Pass by implementation inspection: `packaging/homebrew-cask.sh:11-52`, `.github/workflows/release.yml:528-569`. Native Homebrew execution unverified. |
| Locked executable on Windows fails cleanly | Modified | Pass by code inspection: `crates/host/src/update/replace.rs:23-57`. Native Windows unverified. |
| Managed installs are updated through their owner | Modified | Pass: `crates/host/src/update/types.rs:30-52`, `check_owner.rs:10-25`. |
| systemd unit content and lifecycle | Modified | Pass: `crates/host/src/service/backend/systemd.rs:69-120,233-245`. |
| BusyBox init script content and lifecycle | Modified | Pass: `crates/host/src/service/backend/busybox.rs:71-159,197-233`. |
| Windows system service registration | Modified | Pass by code inspection: `crates/host/src/service/backend/windows_scm.rs:119-145`; native Windows unverified. |
| Binary resolution for registrations | Modified | Pass: `src/cli_service.rs:60-70,222-232`, `crates/host/src/service/install.rs:121-152`. |
| Unsafe registered binary refuses CLI start and restart | Modified | Pass: `crates/host/src/service/verb.rs:225-269`. |

RENAMED requirements: not applicable.

## 4. Scenario Coverage and Fresh Test

**Fresh test:** in `/root/projects/iroh-tunnel.independent-role-binaries`, after sourcing `/tmp/iroh-review-fixes/env.sh` to isolate HOME/XDG/state and supply local Ruby/Cargo, ran `unshare -m sh -c 'mount -t tmpfs tmpfs /run && cargo test --all-features --locked'`. Exit **0**; **39 suites, 954 passed, 0 failed, 4 ignored**. Output: `/tmp/iroh-verify-fresh-tests.log`. The separate `/run` mount prevents collision with host service locks; service-manager opt-ins were not enabled.

| Scenario(s) | Test evidence |
|---|---|
| Service operations without combined executable | `tests/role_binaries.rs:944`, `src/cli_service.rs:363`, managed install `tests/role_binaries.rs:308` |
| Usage error; Config error; Service error | `tests/cli_usage.rs:99`, `tests/config_paths.rs:158`, `tests/service_system_windows.rs:561-565` |
| Render; Independent installation and removal; Release publication | `tests/homebrew_casks.rs:127,152,413,426,888,918` (simulated Homebrew, not native) |
| Running image; Guidance for a system service; Standalone guidance for a system service | `tests/update_zip_windows.rs:247-317` (Windows-specific cases not executed on Linux) |
| Default; Insecure only; Confirmed; Role-specific Homebrew guidance | `tests/update_ownership.rs:154-196,255-318` |
| Unit registration shape; Unit text; Re-install over a running unit | `crates/host/src/service/backend/systemd.rs:342,444-475,491`; fake runner checks restart sequence, not a real running systemd unit |
| BusyBox paths and stopped status; Script verbs | `crates/host/src/service/backend/busybox.rs:336,388-402` |
| Service definition; Standalone service definition | `tests/service_system_windows.rs:430-540`; native/opt-in execution unavailable |
| Not on PATH; Root-run registration from a user-writable binary; Standalone executable alone; Windows standalone service startup | `src/cli_service.rs:388-414`, `tests/service_system.rs:692`, `tests/role_binaries.rs:308`, `tests/service_system_windows.rs:430,459` |
| Legacy registration with a user-writable binary; Directory loosened after install; Unreadable binary in the registration; Standalone invocation guidance | `tests/service_system.rs:732`, `crates/host/src/service/verb.rs:838,1043,1059` |

Scenario mapping is supported by test source, but native macOS/Windows and real service-manager execution are **not verified**; the opt-in tests can return early. Four ignored tests were not executed.

## 5. Design and Contract Coherence

| Entry | Result / evidence |
|---|---|
| `ServiceInvocation` | Match: `crates/host/src/service/types.rs:132-140` |
| `InstallRequest` and migration | Match: `crates/host/src/service/types.rs:142-156`; migration documented in `apply.md`; `crates/host/Cargo.toml:8` is unpublished |
| Invocation-aware lifecycle functions | Match: `crates/host/src/service/mod.rs:65-83`, e.g. `start_with_invocation.rs:7-14`, `status_with_invocation.rs:6-13`; old wrapper `start.rs:18-23` |
| Role service CLI | Match: `src/app.rs:411-420`, `src/cli.rs:90-155` |
| Cask renderer | Match: `packaging/homebrew-cask.sh:11-30` |
| Homebrew cask artifacts | Match by rendered declarations: `packaging/homebrew-cask.sh:33-52`; native Homebrew not verified |
| Apply deviation: lifecycle APIs | Match: exported invocation-aware functions and combined wrappers above |
| Apply deviation: InstallRequest migration | Match: field and documentation above |
| Apply deviation: Homebrew alias semantics | Documented `docs/install.md`; native Homebrew behavior not verified |
| Apply deviation: spec alignment | Revised delta files present; implementation mapped in section 3 |

Design decisions match inspected code, subject to platform runtime limitations. No contract mismatch identified.

## 6. Clean Tree and Commit Range

**Pass before receipt:** `git status --porcelain` returned empty; HEAD `aa8b13e2f57281351f9c73b4b9ed9ced349cf738`. Apply implementation range `9745aeb2a7a858986b13c560fd78277f6e6c932a..e865f02eb80536d67631aac97e05be5e8f1b80f1`, 16 commits; apply receipt commit follows.

## Issues

### CRITICAL

None identified.

### WARNING

- Native Windows SCM lifecycle/locked-file behavior and macOS Homebrew/launchd behavior were not executed (`tests/service_system_windows.rs:430-540`, `tests/homebrew_casks.rs:413-426`). Run native Windows and macOS checks before release.
- Real service-manager lifecycle was not exercised: systemd checks use a fake runner (`crates/host/src/service/backend/systemd.rs:444-475`), and privileged integration tests are opt-in. Run disposable-host lifecycle tests before release.
- Four tests were ignored in the fresh run. Review their prerequisites and run where safe.

### SUGGESTION

- The systemd re-install scenario is covered by a fake-runner assertion of restart ordering rather than a running unit; add disposable-systemd end-to-end coverage if stronger evidence is needed.

## Overall Decision

**PASS_WITH_WARNINGS** — fresh test suite passed; native-platform and real service-manager checks remain unverified.

**Next step:** Run `/opsx-continue` from the authoritative worktree to create finalize, then `/opsx-archive`.
