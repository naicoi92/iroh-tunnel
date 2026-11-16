# Independent Role Binaries Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** Make each role executable independently service-capable and independently installable and maintainable.

**Architecture:** Preserve the shared CLI and host boundaries while passing explicit executable invocation identity into service registration. Release three co-installable casks and report update guidance for the actual executable.

**Tech Stack:** Rust, clap, Tokio, platform service managers, shell, GitHub Actions, Homebrew.

## Global Constraints

Preserve role isolation, existing combined CLI and service registrations, supported platform limitations, binary/config trust, signature verification and ownership policies. Never use test-utils with production relays. Use isolated worktrees and TDD.

## Review Focus

- A renamed standalone executable or managed copy retains its invocation form → Task 1 managed-copy test.
- A combined executable on PATH must not replace the invoked standalone role → Task 1 path precedence test.
- Windows SCM must parse standalone startup as the correct role → Task 1 Windows lifecycle test.
- One cask removal must not delete another alias → Task 3 coexistence/uninstall test.
- A Homebrew update or locked Windows executable must name the correct binary → Task 2 guidance tests.

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `ServiceInvocation` | new | `enum ServiceInvocation { Combined, Standalone }`, `Debug + Clone + Copy + PartialEq + Eq`; standalone role is `InstallRequest.role` | yes |
| `InstallRequest` | modified | Existing fields unchanged; add `pub invocation: ServiceInvocation`; `tunnel_executable` names selected executable, `running_executable` remains managed-copy source | yes |
| Role service CLI | modified | `its service <install\|uninstall\|start\|stop\|restart\|status>` and equivalent `ita`; existing scope/config flags unchanged | yes |
| Cask renderer | modified | `packaging/homebrew-cask.sh <version> <sha256> [combined\|serve\|access]`; omitted role means combined for compatibility; output cask names `iroh-tunnel`, `iroh-tunnel-serve`, `iroh-tunnel-access` | yes |
| Homebrew cask artifacts | modified | Each cask links its named executable and matching alias `it`, `its`, `ita` from its role-specific archive | yes |

## Task 1: Standalone service lifecycle — implements 1

**Covers:** 1.1, 1.2, 1.3  
**Difficulty:** Very Hard  
**Blocked by:** none  
**Files:** Modify `src/app.rs`, `src/cli_service.rs`, `src/scm_runtime/dispatcher.rs`, `crates/host/src/service/types.rs`, `crates/host/src/service/mod.rs`, `crates/host/src/service/install.rs`, relevant files in `crates/host/src/service/backend/`, `crates/host/src/service/task_xml.rs`, fixtures and callers; test `tests/role_binaries.rs`, `tests/service_user.rs`, `tests/service_system.rs`, `tests/service_system_windows.rs`, backend unit tests.  
**Interfaces:** Produces `ServiceInvocation`, modified `InstallRequest` and role service CLI.

- [ ] **Step 1:** Replace `standalone_service_install_is_refused_instead_of_writing_a_broken_unit` with tests asserting each isolated role dispatches service, rejects the other role, and never selects combined binary even if it is on PATH.
- [ ] **Step 2:** Add backend tests asserting combined registration has `serve run`/`access run`, standalone registration has `run`, and managed-copy paths preserve that form, including paths containing spaces.
- [ ] **Step 3:** Run `cargo test --test role_binaries --features test-utils` and `cargo test -p iroh-tunnel-host service`; confirm the new cases fail for the missing behavior.
- [ ] **Step 4:** Add `ServiceInvocation` and `InstallRequest.invocation`; thread it from CLI dispatch to every platform registration; preserve existing privilege and trusted-copy checks.
- [ ] **Step 5:** Add Windows SCM standalone parser and real service startup/stop tests in `tests/service_system_windows.rs`; route each standalone entry point into SCM correctly.
- [ ] **Step 6:** Re-run the tests from Step 3 and platform-appropriate `cargo test --test service_user` / `cargo test --test service_system` and Windows `cargo test --test service_system_windows`; expect pass and report unavailable platforms.
- [ ] **Step 7:** Commit the service implementation and tests.

## Task 2: Role-specific update guidance — implements 2

**Covers:** 2.1, 2.2  
**Difficulty:** Normal  
**Blocked by:** none  
**Files:** Modify `crates/host/src/update/types.rs`, `crates/host/src/update/replace.rs` and other update guidance implementation as necessary; test `tests/update.rs`, `tests/update_ownership.rs`, `tests/update_zip_windows.rs`.  
**Interfaces:** Consumes existing executable identity; produces role-specific guidance without changing owner policy.

- [ ] **Step 1:** Add tests asserting the three Homebrew names map to their own casks; absent `--yes` leaves bytes unchanged, with `--yes` warns and only changes the invoked file.
- [ ] **Step 2:** Add Windows locked-file tests requiring standalone `service stop --system` guidance and unchanged combined commands; verify no process is stopped.
- [ ] **Step 3:** Run `cargo test --test update_ownership` and platform-specific `cargo test --test update_zip_windows`; confirm new assertions fail.
- [ ] **Step 4:** Implement guidance keyed by invoked executable while preserving existing checksum, signature and container behavior.
- [ ] **Step 5:** Run `cargo test --test update --test update_ownership` and Windows `cargo test --test update_zip_windows`; expect pass.
- [ ] **Step 6:** Commit update changes and tests.

## Task 3: Independent Homebrew casks — implements 3

**Covers:** 3.1, 3.2  
**Difficulty:** Hard  
**Blocked by:** none  
**Files:** Modify `packaging/homebrew-cask.sh`, `.github/workflows/release.yml`; create `tests/homebrew_casks.rs`.  
**Interfaces:** Produces Cask renderer and Homebrew cask artifacts.

- [ ] **Step 1:** In `tests/homebrew_casks.rs` render each role with distinct checksum and assert exact cask, URL, executable, alias and SHA; assert invalid/missing inputs fail and omitted role preserves combined output.
- [ ] **Step 2:** Add tests for all casks' disjoint alias install/uninstall paths and release workflow selecting three matching checksums and publishing three files.
- [ ] **Step 3:** Run `cargo test --test homebrew_casks`; confirm new cases fail.
- [ ] **Step 4:** Implement role-aware renderer and stable-release tap update; fail publication if any matching checksum is absent.
- [ ] **Step 5:** Run `cargo test --test homebrew_casks`; on macOS run Homebrew install/coexistence/uninstall smoke tests against the rendered casks and record if unavailable.
- [ ] **Step 6:** Commit packaging and tests.

## Task 4: Documentation and verification — implements 4

**Covers:** 4.1, 4.2  
**Difficulty:** Normal  
**Blocked by:** Task 1, Task 2, Task 3  
**Files:** Modify `README.md`, `docs/install.md`, `docs/services.md` and `docs/cli.md` as needed.  
**Interfaces:** Consumes role service CLI and Homebrew cask artifacts.

- [ ] **Step 1:** Document three cask names, aliases, independent installation, role service commands and update ownership guidance.
- [ ] **Step 2:** Run `cargo fmt --check`, `cargo test --workspace`, `openspec validate independent-role-binaries`, and the focused commands above; record command, platform and result.
- [ ] **Step 3:** Run available macOS and Windows service and cask integration checks; explicitly report missing platform coverage rather than assuming success.
- [ ] **Step 4:** Commit documentation and verification-related changes.

## Scenario → test

- cli-entrypoints / Service operations without combined executable → `tests/role_binaries.rs` isolated service dispatch.
- cli-entrypoints / Usage error, Config error, Service error → existing exit tests and replacement role service error test.
- system-service-management / Not on PATH, Root-run registration from a user-writable binary → existing resolution and managed-copy tests.
- system-service-management / Standalone executable alone → isolated service and per-backend registration tests in Task 1.
- system-service-management / Windows standalone service startup → `tests/service_system_windows.rs`.
- install-release-packaging / Render, Release publication → `tests/homebrew_casks.rs` renderer and workflow tests.
- install-release-packaging / Independent installation and removal → `tests/homebrew_casks.rs` path isolation plus macOS Homebrew smoke tests.
- self-update / Running image, Guidance for a system service, Standalone guidance for a system service → `tests/update_zip_windows.rs`.
- self-update / Default, Insecure only, Confirmed, Role-specific Homebrew guidance → `tests/update_ownership.rs`.
