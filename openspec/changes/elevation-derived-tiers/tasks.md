## 1. Tier type and tier-aware default paths

Blocked by: none

Files: `crates/host/src/tier.rs` (new), `crates/host/src/lib.rs`, `crates/host/src/paths/{mod,locations,default_config_path,default_state_dir}.rs`, `tests/config_paths.rs`, `tests/config_paths_windows.rs`.

- [x] 1.1 Add public `Tier { User, System }` with `from_elevated`, `current()` (production: `is_elevated()`; `test-utils`: `IROH_TUNNEL_TEST_TIER=system|user`) and `label()` (design D1; spec tunnel-configuration "Tier is derived from privilege").
- [x] 1.2 Add `default_config_path_for(role, tier)` and `default_state_dir_for(tier)` returning the D2 table; system tier never reads `HOME`/`SUDO_USER`/profile and skips the macOS legacy migration; `IROH_TUNNEL_TEST_CONFIG_DIR` still overrides the config dir in test builds.
- [x] 1.3 Keep `default_config_path(role)` / `default_state_dir()` signatures, now delegating with `Tier::current()`, so every existing caller (config ops, status, ownership, install) compiles unchanged and gains system-tier behaviour; leave `paths/in_home.rs` for its current callers (deleted in 4.8).
- [x] 1.4 Process tests with `IROH_TUNNEL_TEST_TIER=system`: system paths, `HOME`/`SUDO_USER` ignored, no `HOME` needed, no macOS migration, status names the runtime dir; existing user-tier tests unchanged.

## 2. Ownership per tier, presence lock and other-tier probe

Blocked by: 1

Files: `crates/host/src/ownership.rs`, `crates/host/src/status/mod.rs` (re-export), `crates/host/src/lib.rs` (export `other_tier_running`), `src/status_cmd.rs` (tier label), `crates/host/src/service/install.rs` (compile adaptation `Tier::Machine` → `Tier::System` only), `tests/role_ownership.rs`.

- [x] 2.1 Replace `ownership::Tier` with `crate::Tier` (`Machine` → `System`), keeping a `pub use crate::Tier` where `status` re-exported it; `claim(role)` claims only the current tier: user = user-dir lock exclusive + presence file shared read-only when present; system = machine `<role>.lock` exclusive + create `<role>.user.lock` 0644 unlocked (design D5).
- [x] 2.2 Restrict `publish_identity`/`running_owner` to the claimant's tier; remove cross-tier refusal messages.
- [x] 2.3 Add `probe_tier(role, tier)`, `wait_released_in(role, tier, timeout)`; `probe(role)`/`wait_released(role, timeout)` delegate with `Tier::current()` so status and Windows restart see only their own tier.
- [x] 2.4 Add public `other_tier_running(role, tier) -> Option<Tier>` (I/O errors → debug + `None`) and `pub(crate) ensure_presence_file(role)`; lock and presence files 0644 with the mode set on the open file; machine-dir failure fails a system claim only.
- [x] 2.5 Keep `LockDirs::resolve(home)` and `stop_hint_scoped` compiling for `service/install.rs` until Task 4 removes their callers; update `tests/role_ownership.rs` (coexistence, second system instance, state override, several user holders incl. `kill -9`, blind spot, umask 077).

## 3. Run-path trust, key persistence and legacy refusal

Blocked by: 1

Files: `crates/host/src/service/trust.rs`, `crates/host/src/service/trust_windows/{mod,model,win32}.rs`, `crates/host/src/service/platform.rs` (`load_config` family), `crates/host/src/service/mod.rs` (`ENV_SERVICE`), `crates/host/src/status/writer.rs` (trust branches), `crates/host/src/config/store.rs` (Windows confidentiality at the lock boundary), `crates/host/src/config/dto.rs` (`RoleDoc::ROLE`).

- [x] 3.1 Add `ENV_SERVICE = "IROH_TUNNEL_SERVICE"`; in `load_config` refuse legacy registrations before any read (`IROH_TUNNEL_TRUSTED_UID`/`_SID` present, or root under `INVOCATION_ID`/`XPC_SERVICE_NAME` without the marker) with the Contracts text naming `default_config_path_for(role, System)`; the role comes from `RoleDoc` (design D4).
- [x] 3.2 Walk config + state with admin-only owners only when `Tier::current() == System` and the service marker (`IROH_TUNNEL_SERVICE=1` / `IROH_TUNNEL_SCM=1`) is set; user services and foreground elevated runs are not walked; `test-utils` may add one trusted owner via `IROH_TUNNEL_TEST_TRUSTED_OWNER`.
- [x] 3.3 Remove the empty-key refusal so a system service persists a generated key through the store (owner root/Administrators, 0600).
- [x] 3.4 Windows: confidentiality check (non-admin read on the config dir or file → permission error, never repaired) in `trust_windows` and called from `ConfigLock::acquire` for elevated writes before any lock/temp/config is created, explicit `--config` included.
- [x] 3.5 Keep the uid/SID-pinned verify functions compiling for `service/install.rs` until Task 4 deletes them; status writer re-verifies with admin-only owners under the same condition as 3.2.

## 4. Service layer: tier from privilege, no pin, no cross-tier refusal

Blocked by: 2, 3

Files: `crates/host/src/service/{mod,types,install,verb,start,stop,restart,status,uninstall,policy,platform,registration,native,fixtures,effective_binary,service_binary,task_xml,system_verb}.rs`, `crates/host/src/service/backend/*`, `crates/host/src/ownership.rs` (delete `LockDirs::resolve(home)`, `stop_hint_scoped`), `crates/host/src/service/trust*.rs` (delete pinned-user variants), `crates/host/src/paths/in_home.rs` (delete), `crates/host/src/lib.rs`, `crates/host/examples/manage_service.rs`, `src/scm_runtime.rs`, `src/cli_service.rs` and `src/render.rs` (compile adaptation only), `tests/service_user.rs`, `tests/service_user_windows.rs` (delete tests whose semantics are removed).

- [ ] 4.1 Replace `ServiceScope` with `Tier` derived from `Environment { elevated }`; remove scope parameters, `InstallRequest.{scope,sudo_user,invoking_user,retarget}`, `InstallOutcome::Needs`, `InstallNeed`, `ScopeDecision`; `install` returns `Result<ServiceReport, ServiceFailure>`; `src/cli_service.rs` adapted just enough to compile (prompts removed in group 5).
- [ ] 4.2 System install: default paths via `*_for(…, System)`, create the missing state dir, canonicalize, trust-check, `ensure_presence_file`; missing config → config error naming the path and `sudo iroh-tunnel <role> config keygen`.
- [ ] 4.3 Registrations: drop pin comment and trust env; add `IROH_TUNNEL_SERVICE=1` (systemd, launchd, BusyBox); Windows Service env keeps state dir + `IROH_TUNNEL_SCM=1`.
- [ ] 4.4 Remove cross-tier and Windows cross-scope refusals; same-tier start refusal via `probe_tier`; `system_verb` waits with `wait_released_in(role, System, …)`; emit `ServiceNote::OtherTierRunning` from install/start/restart/status.
- [ ] 4.5 BusyBox: non-root mutating verbs → permission error; remove the user-scope note.
- [ ] 4.6 Windows SCM: exit 6 only from a same-tier conflict.
- [ ] 4.7 Host guidance strings (managed-copy note, binary refusal, launchd not-loaded hint) without scope flags.
- [ ] 4.8 Delete the scope-refusal/prompt tests in `tests/service_user*.rs`; delete `{system,system_windows,pin,account,account_name,decide_install_scope,default_scope,parse_scope_answer}.rs`, `paths/in_home.rs`, pinned-user trust variants, `LockDirs::resolve(home)`, `stop_hint_scoped`; update `examples/manage_service.rs`; host tests and clippy green.

## 5. CLI surface and other-tier notice

Blocked by: 4

Files: `src/cli.rs`, `src/cli_args.rs`, `src/cli_service.rs`, `src/app.rs`, `src/render.rs`, `src/status_cmd.rs`, `src/config_cmd.rs`, `crates/host/src/config/store.rs` (system config dir creation), `crates/host/src/service/mod.rs` (Windows config-dir SDDL), `crates/host/src/update/replace.rs`, `tests/cli_golden.rs`, `tests/cli_usage.rs`, `tests/role_binaries.rs`, `tests/config_lifecycle.rs`, `tests/update_zip_windows.rs` (guidance assertions moved here from 6.2), `tests/service_user*.rs` and `tests/service_system*.rs` (flag removal and guidance assertions moved here from 6.x).

- [ ] 5.1 Remove `ScopeArgs`/`--user`/`--system`; short-option test asserts no `system`/`user`.
- [ ] 5.2 Remove scope, username and retarget prompts; service dispatch builds `Environment { elevated: is_elevated() }`.
- [ ] 5.3 Print the other-tier notice before bind/manager calls for `run` (configured and quick), `status`, and render `ServiceNote::OtherTierRunning` for service verbs.
- [ ] 5.4 Rewrite guidance (`render.rs`, `update/replace.rs`) without scope flags; Windows update guidance names both elevated `service stop` commands.
- [ ] 5.5 Elevated config writes create `/etc/iroh-tunnel` (macOS config dir) 0755 root, Windows `config` dir with the admin-only SDDL.
- [ ] 5.6 Golden/usage tests updated, scope flags dropped from every test invocation, guidance assertions updated; `cargo test --workspace --all-features` green.

## 6. End-to-end suites, docs and changelog

Blocked by: 5

Files: `tests/service_system.rs`, `tests/service_system_windows.rs`, `tests/service_user.rs`, `tests/service_user_windows.rs`, `tests/update_ownership.rs`, `README.md`, `CHANGELOG.md`, `docs/GLOSSARY.md`, `docs/**` mentioning scope flags.

- [ ] 6.1 Rewrite `tests/service_system.rs` (privileged BusyBox container) for system-tier paths, trust, legacy refusal, key persistence, coexistence; keep SM-BIN cases with new guidance.
- [ ] 6.2 Rewrite `tests/service_system_windows.rs` and `tests/service_user_windows.rs` for ProgramData config, readable-config refusal, coexistence, flagless verbs.
- [ ] 6.3 Update `tests/service_user.rs` for flagless verbs and no linger calls.
- [ ] 6.4 Docs: README tier section, GLOSSARY User tier / System tier, CHANGELOG Breaking entries with the manual migration steps.
- [ ] 6.5 Fresh verification: fmt, clippy `--locked`, tests `--locked`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`; record which opt-in E2E suites ran where.
