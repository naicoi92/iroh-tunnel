# Elevation-Derived Tiers Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** Derive a user or system tier from privilege for every command, move system-tier config/state to machine-wide paths, delete the user-pin/scope machinery, and let both tiers of a role coexist with a notice.

**Architecture:** One `Tier` value (host crate), resolved per process by `Tier::current()` from `is_elevated()`, drives paths (`paths`), ownership (`ownership`), run-path trust (`service::trust*`, `service::platform::load_config`) and service operations (`service::*`). Service operations derive the tier from the injected `Environment { elevated }` so no caller can request another tier. Ownership keeps one lock per tier and a machine-directory presence file for other-tier detection.

**Tech Stack:** Rust 2021 (MSRV 1.91), clap, anyhow + `CliError`, libc / windows-sys, `tempfile`, `assert_cmd`-style process tests already in `tests/`.

## Global Constraints

- Exit codes unchanged: config 2, permission 3, service 5, conflict 6 (`CliError`).
- No new dependencies; no `--user`/`--system` flag anywhere (clap rejects them as unexpected).
- System-tier paths (design D2): Linux `/etc/iroh-tunnel/<role>.toml`, state `/run/iroh-tunnel`; macOS `/Library/Application Support/iroh-tunnel/config/<role>.toml`, state `/Library/Application Support/iroh-tunnel/state` (locks stay in `/var/run/iroh-tunnel`; deviation decided by the user during Task 4: `/private/var/run` is group-writable and fails the trust walk); Windows `%ProgramData%\iroh-tunnel\config\<role>.toml`, state `%ProgramData%\iroh-tunnel\state`.
- User-tier paths and behaviour unchanged.
- Never chmod/chown/repair DACLs of existing files; never write secrets to owner records, notices or logs.
- Test-only seams are gated by `feature = "test-utils"`: `IROH_TUNNEL_TEST_TIER`, `IROH_TUNNEL_TEST_TRUSTED_OWNER`, `IROH_TUNNEL_MACHINE_LOCK_DIR`, `IROH_TUNNEL_TEST_CONFIG_DIR`, `IROH_TUNNEL_TEST_MANAGED_BIN_DIR`.
- Every task ends with the whole workspace compiling and `cargo test --workspace --all-features` green; signatures removed in Task 4 stay available until then.
- Scheduling: Tasks 2 and 3 may run in parallel — their file lists are disjoint (Task 2: `ownership.rs`, `status/mod.rs`, `lib.rs` export line, `src/status_cmd.rs`; Task 3: `service/{trust*,platform,mod}.rs`, `status/writer.rs`, `config/{store,dto}.rs`). If `lib.rs` conflicts, Task 3 does not touch it.
- Standards: `docs/agents/rust-coding-standards.md` (RS-01–RS-16), `~/.agents/skills/coding-standards/SKILL.md`, `rust-best-practices`; TDD per step.
- Verify with `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo test --workspace --all-features --locked`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`.

## Review Focus

1. `sudo -E` keeps `HOME=/home/alice`: system tier must still resolve `/etc/iroh-tunnel` and never touch alice's home → pinned in Task 1 (`system_tier_ignores_home_and_sudo_user`).
2. A user-tier process on a host where the machine directory does not exist or is unreadable must start without a notice and without creating anything → Task 2 (`user_claim_without_machine_dir_creates_nothing`, `probe_io_error_is_none`).
6. A user systemd unit / LaunchAgent carries `IROH_TUNNEL_SERVICE=1` but runs unelevated; it must keep reading the user's own files → Task 3 (`user_service_with_marker_is_not_walked`).
7. With both tiers running, `status` and Windows `restart` must act on their own tier only → Task 4 (`status_reports_own_tier_owner_when_both_run`, `system_restart_waits_for_system_holder_only`).
3. macOS default system paths must pass the trust walk (`/private/var/run` is group-writable, so the state dir lives under `/Library/Application Support`) → Task 4 (`macos_system_state_dir_passes_the_trust_walk`).
4. A Windows `config` directory created by an earlier admin with inherited Users-read must be refused before a key is written → Task 3 (`config_dir_readable_by_users_is_refused`).
5. A legacy unit restarting in a loop (systemd `Restart=on-failure`) must exit with config error 2 every time without touching the old user file → Task 3 (`legacy_trusted_uid_is_refused_before_reading_config`) and Task 6 E2E.

---

## Contracts

Process-scoped entry points keep their signatures and resolve `Tier::current()` internally (design D1); `*_for` / `*_in` variants take an explicit tier for the service layer.

| Name | new / modified | Shape | public |
|---|---|---|---|
| `iroh_tunnel_host::Tier` | new | `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Tier { User, System }`; `pub fn from_elevated(elevated: bool) -> Tier`; `pub fn current() -> Tier`; `pub fn label(self) -> &'static str` (`"user session"` / `"system service"`) | yes |
| `paths::default_config_path_for` | new | `pub fn default_config_path_for(role: &str, tier: Tier) -> Result<PathBuf>` | yes |
| `paths::default_state_dir_for` | new | `pub fn default_state_dir_for(tier: Tier) -> Result<PathBuf>` | yes |
| `paths::default_config_path` / `default_state_dir` | modified (behaviour) | signatures unchanged; delegate to `*_for(…, Tier::current())` | yes |
| `ownership::Tier` | removed | replaced by `crate::Tier`; `status` keeps `pub use crate::Tier` where it re-exported the old type; `Liveness::Running { tier: Tier, owner: Option<OwnerRecord> }` | yes |
| `ownership::claim` | modified (behaviour) | `pub(crate) fn claim(role: Role) -> Result<RoleClaim>` claims `Tier::current()` only | no |
| `ownership::probe_tier` | new | `pub(crate) fn probe_tier(role: Role, tier: Tier) -> Result<Liveness>`; `probe(role)` delegates with `Tier::current()` | no |
| `ownership::wait_released_in` | new | `pub(crate) fn wait_released_in(role: Role, tier: Tier, timeout: Duration) -> Result<()>`; the current-tier `wait_released(role, timeout)` was removed during Task 4 (recorded in apply.md) | no |
| `iroh_tunnel_host::other_tier_running` | new | `pub fn other_tier_running(role: Role, tier: Tier) -> Option<Tier>` re-exported from `lib.rs`; I/O errors → debug log, `None` | yes |
| `ownership::ensure_presence_file` | new | `pub(crate) fn ensure_presence_file(role: Role) -> Result<()>` | no |
| Presence file | new | `<machine dir>/<role>.user.lock`, empty, mode 0644 set on the open file; user tier holds it shared via read-only open | file format |
| Lock files in the machine dir | modified | `<role>.claim.lock` new, 0600 / admin-only DACL, system holds exclusive; `<role>.lock` 0644 set on the open file, system holds shared (detection only). Deviation (final review, user-approved): split to stop non-admin DoS | file format |
| `service::ENV_SERVICE` | new | `pub const ENV_SERVICE: &str = "IROH_TUNNEL_SERVICE";` value `"1"` | yes |
| `IROH_TUNNEL_SERVICE` env | new | written to systemd `Environment=`, launchd `EnvironmentVariables`, BusyBox `export`, for both tiers | env contract |
| `service::under_system_service` | new | `pub(crate) fn under_system_service() -> bool` defined in `service/mod.rs` (reachable from `status::writer` and `service::platform`): `Tier::current() == Tier::System && (ENV_SERVICE == "1" || ENV_SCM == "1")` | no |
| `IROH_TUNNEL_TRUSTED_UID` / `IROH_TUNNEL_TRUSTED_SID` | modified | no longer written; presence ⇒ legacy refusal (exit 2) | env contract |
| `IROH_TUNNEL_TEST_TIER` | new | `test-utils` only: `system` / `user` overrides `Tier::current()`; unset or other value → privilege | no |
| `IROH_TUNNEL_TEST_TRUSTED_OWNER` | new | `test-utils` or `cfg(test)` only: a numeric uid (Unix) or SID string (Windows) added to the admin-only trusted set, as `TrustedOwners::TestOwner`. Deviation (internal, Task 3): also honoured under `cfg(test)` so host unit tests need no feature flag; variant named `TestOwner` (clippy enum-variant-names) | no |
| `service::platform::load_config` | modified (behaviour) | `pub(crate) fn load_config<C: RoleDoc>(path: &Path) -> Result<C>`; `RoleDoc` gains `const ROLE: Role` so the legacy message names `default_config_path_for(C::ROLE.name(), Tier::System)` | no |
| `trust_windows::verify_confidential_with` | new | `pub(crate) fn verify_confidential_with(path: &Path, trusted: &TrustedSet) -> Result<()>` (renamed from `verify_confidential(path)` during apply: the trusted set is passed explicitly): refuse when the DACL grants any read right to a SID outside SYSTEM, Administrators, TrustedInstaller (and the test owner) | no |
| Windows verified config read | modified (behaviour) | install's config check and `load_config` under `under_system_service()` call `verify_confidential(dir)` and `verify_confidential(file)` after the ownership walk and before parsing, whether or not a key exists | no |
| `config::store::ConfigLock::acquire` | modified (behaviour) | unchanged signature; when `is_elevated()` on Windows **and the target lies in the system-tier config directory** (`%ProgramData%\iroh-tunnel\config`), calls `verify_confidential(parent)` and, if the file exists, `verify_confidential(file)` before creating any file. Deviation (internal, Task 3): narrowed from "every elevated write incl. explicit --config" to the spec's scope — elevated CI writes to temp dirs must not be refused | no |
| Pinned-user trust APIs | removed (Task 4) | `TrustedOwners::RootAnd`, `trusted_uid_from_env`, `verify_dir(path, uid)`, `read_verified(path, uid)`, `TrustedSet::for_user_files`, `trusted_sid_from_env` | no |
| `service::Environment` | modified | `pub struct Environment { pub elevated: bool }` (`can_answer` removed) | yes |
| `service::InstallRequest` | modified | `{ role: Role, invocation: ServiceInvocation, config: Option<PathBuf>, tunnel_executable: PathBuf, running_executable: RunningExecutable, state_dir_override: Option<PathBuf> }` | yes |
| `service::install` | modified | `pub fn install(req: &InstallRequest, env: Environment, call: &mut ServiceCall<'_>) -> Result<ServiceReport, ServiceFailure>` | yes |
| `service::{uninstall,start,stop,restart}` | modified | `pub fn <verb>(role: Role, invocation: &ServiceInvocation, env: Environment, call: &mut ServiceCall<'_>) -> Result<ServiceReport, ServiceFailure>` | yes |
| `service::status` | modified | `pub fn status(role: Role, invocation: &ServiceInvocation, env: Environment, call: &mut ServiceCall<'_>) -> Result<ServiceStatusReport, ServiceFailure>` | yes |
| `service::{ServiceScope, InstallOutcome, InstallNeed, ScopeDecision, Pin, PinOwner, RetargetConsent, decide_install_scope, default_scope, parse_scope_answer, unknown_user, account_name}` | removed | — | yes |
| Windows always-elevated refusal | new | user-tier Windows `install` → `CliError::Permission` (exit 3) when the account is the built-in Administrator or an Administrators member with `EnableLUA=0`; message: `this account always runs elevated, so its logon task would run as a system process; install from an elevated terminal (Run as administrator) to get a system service`. Added after apply (Windows CI), user-approved | yes |
| `ServiceNote::OtherTierRunning` | new | `OtherTierRunning { role: String, other: Tier }` | yes |
| `ServiceNote::BusyboxSystemWide` | removed | — | yes |
| Registration pin comment | removed | `iroh-tunnel-pin config=… uid=…/sid=…` no longer written or parsed | file format |
| CLI `service <verb>` | modified | `--user` / `--system` removed from `install|uninstall|start|stop|restart|status` | yes |
| Other-tier notice | new | stderr; user tier: `note: <role> is also running as a system service; manage it with sudo or from an elevated terminal`; system tier: `note: <role> is also running in a user session` | yes |
| Legacy refusal message | new | `this service was registered by an earlier version and points at a user's files; create <system config path> (for example \`sudo iroh-tunnel <role> config keygen\`) and re-run \`sudo iroh-tunnel <role> service install\``; Windows replaces `sudo …` with "from an elevated terminal (Run as administrator)" | yes |
| Windows config-dir SDDL | new | `O:BAG:SYD:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)` for `%ProgramData%\iroh-tunnel\config` | file format |

---

## Task 1: Tier type and tier-aware default paths — implements 1

**Covers:** 1.1, 1.2, 1.3, 1.4
**Difficulty:** Normal
**Blocked by:** none

**Files:**
- Create: `crates/host/src/tier.rs`
- Modify: `crates/host/src/lib.rs`, `crates/host/src/paths/{mod,locations,default_config_path,default_state_dir}.rs`
- Test: `tests/config_paths.rs`, `tests/config_paths_windows.rs`, unit tests in `tier.rs` and `paths/*`

**Interfaces:**
- Produces: `Tier`, `default_config_path_for`, `default_state_dir_for`; existing `default_config_path`/`default_state_dir` keep their signatures.

- [ ] **Step 1:** Unit tests in `tier.rs`: `from_elevated_maps_privilege` (`true → System`, `false → User`); `test_tier_override_selects_the_tier_in_a_subprocess` (in `tests/config_paths.rs`) (`IROH_TUNNEL_TEST_TIER=system` → `System`, `user` → `User`, `bogus` → privilege).
- [ ] **Step 2:** `cargo test -p iroh-tunnel-host --all-features tier::` → FAIL (module missing).
- [ ] **Step 3:** Implement `Tier` per Contracts; re-export from `lib.rs`; rustdoc on every public item.
- [ ] **Step 4:** Unit tests in `paths`: `system_config_path_is_platform_machine_path` (Linux `/etc/iroh-tunnel/serve.toml`; macOS `/Library/Application Support/iroh-tunnel/config/serve.toml`; Windows `<ProgramData>\iroh-tunnel\config\serve.toml`), `system_state_dir_is_platform_runtime_path` (`/run/iroh-tunnel`, `/var/run/iroh-tunnel`, `<ProgramData>\iroh-tunnel\state`) — both with `HOME` removed.
- [ ] **Step 5:** Run → FAIL. Implement `*_for` (system branch from constants / `service::program_data_dir()`, no `home_dir()`/`local_app_data()`, no `migrate_legacy`; `IROH_TUNNEL_TEST_CONFIG_DIR` keeps priority) and make the old functions delegate with `Tier::current()`.
- [ ] **Step 6:** Process tests in `tests/config_paths.rs` with `IROH_TUNNEL_TEST_TIER=system`: `system_tier_ignores_home_and_sudo_user` (`HOME=/home/alice`, `SUDO_USER=alice`; `config path` output lacks `/home/alice`), `system_tier_without_home_succeeds`, `system_tier_does_not_migrate_macos_legacy` (macOS; unreadable legacy file present → `config keygen` with `IROH_TUNNEL_TEST_CONFIG_DIR` succeeds, no "migrated"), `system_tier_state_dir_is_runtime_dir_without_home`.
- [ ] **Step 7:** `cargo test --workspace --all-features` → PASS (whole workspace compiles: no signature changed).
- [ ] **Step 8:** Commit `feat(host): derive tier from privilege for default paths`.

---

## Task 2: Ownership per tier, presence lock and other-tier probe — implements 2

**Covers:** 2.1, 2.2, 2.3, 2.4, 2.5
**Difficulty:** Hard
**Blocked by:** Task 1

**Files:**
- Modify: `crates/host/src/ownership.rs`, `crates/host/src/status/mod.rs`, `crates/host/src/lib.rs`, `src/status_cmd.rs` (label of `Tier::System`), `crates/host/src/service/install.rs` (compile adaptation only: `Tier::Machine` → `Tier::System` in `ensure_no_cross_scope_conflict_in`; logic untouched until Task 4)
- Test: `tests/role_ownership.rs`, unit tests in `ownership.rs`

**Interfaces:**
- Consumes: `Tier`, `default_state_dir_for(Tier::User)`.
- Produces: `probe_tier`, `wait_released_in`, `other_tier_running`, `ensure_presence_file`; `claim`/`probe`/`wait_released` signatures unchanged.

- [ ] **Step 1:** Unit tests via the existing `TierDirs` injection: `system_claim_creates_presence_file_unlocked`, `user_claim_holds_presence_shared_when_present`, `user_claim_without_machine_dir_creates_nothing`, `system_and_user_claims_coexist`, `second_system_claim_is_refused` (Conflict, "already running"), `identity_check_ignores_other_tier`, `corrupt_machine_record_ignored_for_user_claim`.
- [ ] **Step 2:** `cargo test -p iroh-tunnel-host --all-features ownership::` → FAIL.
- [ ] **Step 3:** Implement design D5 claim, records and identity scope; machine-dir resolution failure fails only a system claim.
- [ ] **Step 4:** Unit tests: `probe_tier_sees_only_its_tier` (both tiers held → each probe returns its own owner), `wait_released_in_ignores_other_tier`, `user_sees_system_holder`, `system_sees_shared_user_holder`, `system_sees_none_after_last_user_exits`, `probe_io_error_is_none` (lock path is a directory), `new_machine_dir_lock_and_presence_are_public_under_umask_077` and `machine_owner_record_is_world_readable_under_umask_077` (Unix), `machine_dir_is_platform_runtime_path` (`/run/iroh-tunnel` Linux, `/var/run/iroh-tunnel` macOS; non-`test-utils` resolver function exercised directly).
- [ ] **Step 5:** Implement `probe_tier`, `wait_released_in` (old functions delegate), `other_tier_running`, `ensure_presence_file`, 0644 on the open file. Keep `LockDirs::resolve(home)` and `stop_hint_scoped` untouched for `install.rs`.
- [ ] **Step 6:** `tests/role_ownership.rs`: replace `system_tier_lock_refuses_user_start_and_blocks_a_system_start` with `system_and_user_tiers_run_side_by_side`; add `second_system_instance_is_refused`, `system_lock_ignores_state_override`, `several_user_holders_keep_presence_until_last_exits` (last one `kill -9`), `blind_spot_user_before_presence_is_not_seen` — all with `IROH_TUNNEL_MACHINE_LOCK_DIR` + `IROH_TUNNEL_TEST_TIER`.
- [ ] **Step 7:** `cargo test --workspace --all-features` → PASS.
- [ ] **Step 8:** Commit `feat(host): per-tier ownership with presence lock`.

---

## Task 3: Run-path trust, key persistence and legacy refusal — implements 3

**Covers:** 3.1, 3.2, 3.3, 3.4, 3.5
**Difficulty:** Hard
**Blocked by:** Task 1

**Files:**
- Modify: `crates/host/src/service/trust.rs`, `crates/host/src/service/trust_windows/{mod,model,win32}.rs`, `crates/host/src/service/platform.rs`, `crates/host/src/service/mod.rs`, `crates/host/src/status/writer.rs`, `crates/host/src/config/store.rs`, `crates/host/src/config/dto.rs` (`RoleDoc::ROLE`)
- Test: unit tests in those files

**Interfaces:**
- Consumes: `Tier`, `default_config_path_for`.
- Produces: `ENV_SERVICE`, `under_system_service()`, `verify_confidential`, admin-only trusted set with the `test-utils` owner seam; pinned-user APIs still compile (deleted in Task 4).

- [ ] **Step 1:** Unit tests (env passed through closures as `is_unpinned_service` does): `legacy_trusted_uid_is_refused_before_reading_config` (Config class; text contains `/etc/iroh-tunnel/serve.toml` and `sudo iroh-tunnel serve service install`; config path does not exist and is not opened), `legacy_trusted_sid_is_refused` (Windows wording), `root_under_manager_without_service_marker_is_legacy`, `foreground_root_run_is_not_walked`, `user_service_with_marker_is_not_walked` (tier User + `IROH_TUNNEL_SERVICE=1` → plain read of a user-owned file).
- [ ] **Step 2:** `cargo test -p iroh-tunnel-host --all-features trust platform` → FAIL.
- [ ] **Step 3:** Add `RoleDoc::ROLE`, `ENV_SERVICE`, `under_system_service()`; implement the legacy check first in `load_config`.
- [ ] **Step 4:** Unit tests using `IROH_TUNNEL_TEST_TRUSTED_OWNER=<current uid/SID>` for success paths and without it for refusals: `system_service_reads_admin_owned_config`, `system_service_refuses_untrusted_owner`, `symlinked_component_is_refused`, `missing_key_is_persisted_under_system_service` (file mode 0600 after run), convert existing `trust.rs` success-path tests from `RootAnd(me())` to the test owner seam.
- [ ] **Step 5:** Implement admin-only verification under `under_system_service()`; remove the empty-key refusal (`empty_secret_key_under_service_is_config_error_and_file_unchanged` becomes `empty_secret_key_under_service_is_persisted`).
- [ ] **Step 6:** Windows unit tests: `config_dir_readable_by_users_is_refused`, `config_in_user_profile_is_refused`, `machine_dir_users_read_still_accepted`, `elevated_store_write_refuses_readable_dir_before_creating_files` (no lock/temp/config file afterwards, DACL unchanged), `service_start_refuses_readable_config_with_key` (non-empty key + Users-read on the file → permission error before parse, DACL unchanged). Implement `verify_confidential` and call it from `ConfigLock::acquire` when elevated and from the Windows verified-read path of `load_config` and of install's config check.
- [ ] **Step 7:** Status writer under `under_system_service()` uses the admin-only set; `publish_replaces_atomically_and_never_follows_a_planted_link` (Unix) and `publish_replaces_a_planted_symlink` (Windows) stay green.
- [ ] **Step 8:** `cargo test --workspace --all-features` → PASS.
- [ ] **Step 9:** Commit `feat(host): admin-only trust for system-tier files and legacy refusal`.

---

## Task 4: Service layer: tier from privilege, no pin, no cross-tier refusal — implements 4

**Covers:** 4.1, 4.2, 4.3, 4.4, 4.5, 4.6, 4.7, 4.8
**Difficulty:** Very Hard
**Blocked by:** Task 2, Task 3

**Files:**
- Modify: `crates/host/src/service/{mod,types,install,verb,start,stop,restart,status,uninstall,policy,platform,registration,native,fixtures,effective_binary,service_binary,task_xml,system_verb}.rs`, `crates/host/src/service/backend/{dispatch,systemd,launchd,busybox,busybox_script,windows,windows_scm,windows_scm_impl}.rs`, `crates/host/src/ownership.rs`, `crates/host/src/service/trust.rs`, `crates/host/src/service/trust_windows/*`, `crates/host/src/lib.rs`, `crates/host/examples/manage_service.rs`, `tests/service_user.rs`, `tests/service_user_windows.rs` (delete tests whose semantics are removed), `src/scm_runtime.rs`, `src/cli_service.rs` and `src/render.rs` (compile adaptation only: new signatures, `ServiceNote::OtherTierRunning` arm with the Contracts text, `BusyboxSystemWide` arm removed)
- Delete: `crates/host/src/service/{system,system_windows,pin,account,account_name,decide_install_scope,default_scope,parse_scope_answer}.rs`, `crates/host/src/paths/in_home.rs`
- Test: unit tests in the touched modules

**Interfaces:**
- Consumes: `Tier`, `*_for`, `probe_tier`, `wait_released_in`, `other_tier_running`, `ensure_presence_file`, `ENV_SERVICE`, admin-only trust, `under_system_service()`.
- Produces: service API per Contracts.

- [ ] **Step 1:** Registration render tests: `systemd_unit_has_service_marker_and_no_pin`, `reinstall_restarts_a_running_unit`, `launchd_plist_has_service_marker_and_no_pin`, `busybox_script_exports_service_marker_and_no_pin`, `windows_service_env_has_scm_marker_and_no_sid`. Run → FAIL. Implement.
- [ ] **Step 2:** Install planning tests (fixtures, `IROH_TUNNEL_TEST_TRUSTED_OWNER`): `system_install_defaults_to_machine_paths`, `system_install_creates_missing_state_dir_before_canonicalizing`, `system_install_pins_canonical_state_dir` (canonical pinning of a symlinked override; no launchd needed), `system_install_missing_config_names_path_and_keygen` (exit 2), `system_install_refuses_untrusted_config` (exit 3), `system_install_creates_presence_file`, `install_returns_report_not_need`, `failed_trust_check_writes_no_managed_copy`. Run → FAIL.
- [ ] **Step 3:** Implement tier from `Environment`, system defaults, create-if-missing state dir (Unix 0755; Windows under the machine dir) → canonicalize → verify → presence → registration; remove `Needs`/scope/pin/retarget paths; adapt `src/cli_service.rs` to the new signatures (prompt code paths stay deleted-or-unreachable until Task 5 removes them).
- [ ] **Step 4:** Tests: `install_proceeds_when_other_tier_runs_with_note`, `start_refused_only_in_same_tier`, `status_reports_own_tier_owner_when_both_run`, `system_restart_waits_for_system_holder_only`, `busybox_non_root_mutation_is_permission_error`, `busybox_root_install_has_no_user_scope_note`, `scm_conflict_exit_only_for_same_tier`, `task_restart_times_out_without_starting`. Run → FAIL. Implement (cross-tier and Windows cross-scope refusals removed; `system_verb` uses `wait_released_in(role, Tier::System, …)`; `status` uses `probe_tier`).
- [ ] **Step 5:** Host guidance strings (managed-copy note, binary refusal, launchd not-loaded hint) without scope flags; update their existing tests (`standalone_privilege_guidance_for_all_verbs_uses_supplied_executable`, `standalone_guard_guidance_names_invoked_executable_and_config`, managed-copy note tests).
- [ ] **Step 6:** Delete tests whose semantics this task removes: `tests/service_user.rs::{system_install_without_privilege_stops_with_rerun_guidance, system_scope_operations_without_privilege_are_refused_with_guidance, interactive_install_offers_user_or_system_and_system_is_refused_unprivileged}`, `tests/service_user_windows.rs::non_elevated_system_scope_is_permission_error`, and the unit tests named in Step 4's rewrite list. Delete the listed modules, pinned-user trust variants, `LockDirs::resolve(home)`, `stop_hint_scoped`, `windows_empty_key_text_names_keygen`, `user_scope_start_skips_guard` (rewritten as `user_tier_start_skips_guard`); update `examples/manage_service.rs`.
- [ ] **Step 7:** `cargo test --workspace --all-features --locked` and `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` → PASS.
- [ ] **Step 8:** Commit `feat(host)!: service tier follows privilege; drop owner pin and scope`.

---

## Task 5: CLI surface and other-tier notice — implements 5

**Covers:** 5.1, 5.2, 5.3, 5.4, 5.5, 5.6
**Difficulty:** Normal
**Blocked by:** Task 4

**Files:**
- Modify: `src/cli.rs`, `src/cli_args.rs`, `src/cli_service.rs`, `src/app.rs`, `src/render.rs`, `src/status_cmd.rs`, `src/config_cmd.rs`, `crates/host/src/config/store.rs`, `crates/host/src/service/mod.rs`, `crates/host/src/update/replace.rs`
- Test: `tests/cli_golden.rs`, `tests/cli_usage.rs`, `tests/role_binaries.rs`, `tests/config_lifecycle.rs`, `tests/update_zip_windows.rs` (guidance assertions, moved here from 6.2), `tests/service_user.rs`, `tests/service_user_windows.rs` and `tests/service_system*.rs` (flag removal from invocations and guidance assertions, moved from 6.x so the workspace stays green), unit tests in `src/*`

**Interfaces:**
- Consumes: service API, `other_tier_running`, `Tier::current()`.

- [ ] **Step 1:** Tests: `service_scope_flags_are_rejected` (`install --system`, `stop --user` → usage error, nothing registered); short-option unit test asserts no `system`/`user` long option; `install_reads_nothing_from_stdin` (stdin closed, user tier, fixture backend). Run → FAIL. Remove `ScopeArgs` and the prompt functions; drop `--user`/`--system` from every remaining invocation under `tests/` and update `tests/service_user.rs::denied_registration_fails_with_guidance_and_keeps_scope` to assert the new guidance (no `--system`).
- [ ] **Step 2:** Tests in `tests/role_ownership.rs` (they need real processes): `run_prints_other_tier_notice_before_bind`, `status_prints_user_session_notice_in_system_tier`, `status_reads_own_tier_snapshot_when_both_run`, `no_notice_when_other_tier_idle`, `port_clash_error_follows_notice` (system-tier access holds `127.0.0.1:<port>`; user access with the same listen → stderr notice line precedes the bind error naming the address). Run → FAIL. Print the notice in `app.rs` for configured and quick `run` and for `status`; render `ServiceNote::OtherTierRunning`.
- [ ] **Step 3:** Add `src/render.rs::launchd_bootstrap_hint_names_print_disabled`; rewrite guidance in `render.rs` and `update/replace.rs`; update `replace.rs` unit tests and `tests/update_zip_windows.rs::assert_stop_guidance` to require `Run as administrator`, `iroh-tunnel serve service stop`, `iroh-tunnel access service stop`, and no `--system`.
- [ ] **Step 4:** Elevated config writes: create `/etc/iroh-tunnel` / macOS config dir 0755 and Windows `config` dir with the Contracts SDDL; tests `elevated_keygen_writes_system_config` (`IROH_TUNNEL_TEST_TIER=system` + temp `IROH_TUNNEL_TEST_CONFIG_DIR`; Unix asserts file 0600, dir 0755; owner-root assertion only under real root) and `read_only_config_mutation_is_refused_unchanged` (new: `chmod 0444` target → "read-only" message, bytes unchanged), `config_list_unparseable_exits_2` (new).
- [ ] **Step 5:** Update golden files; `cargo test --workspace --all-features --locked` → PASS.
- [ ] **Step 6:** Commit `feat(cli)!: remove scope flags and prompts; print other-tier notice`.

---

## Task 6: End-to-end suites, docs and changelog — implements 6

**Covers:** 6.1, 6.2, 6.3, 6.4, 6.5
**Difficulty:** Hard
**Blocked by:** Task 5

**Files:**
- Modify: `tests/service_system.rs`, `tests/service_system_windows.rs`, `tests/service_user.rs`, `tests/service_user_windows.rs`, `tests/update_ownership.rs`, `README.md`, `CHANGELOG.md`, `docs/GLOSSARY.md`, every `docs/**` page that mentions `--system`, `--user` or `SUDO_USER`

- [ ] **Step 1:** `tests/service_system.rs` (BusyBox container, `IROH_TUNNEL_E2E_PRIVILEGED=1`, real root): remove retarget/unknown-user/user-pin cases and `sm_bin_05`, `sm_bin_10`; add `system_install_uses_etc_and_run`, `missing_system_config_names_keygen`, `user_owned_config_refused_at_install` (exit 3), `loosened_config_refused_at_service_start`, `admin_edit_applies_after_restart_keeping_root_0600`, `invalid_edit_never_serves_and_keeps_owner_mode`, `legacy_pinned_script_is_refused_with_guidance`, `system_service_persists_generated_key_root_0600`, `non_root_cannot_read_system_config`, `user_and_system_tiers_coexist_with_notices`, `busybox_non_root_mutation_refused`, `boot_start_of_current_registration_is_not_binary_checked`; keep `sm_bin_01..04,06..09` with new guidance.
- [ ] **Step 2:** Windows suites (`IROH_TUNNEL_E2E_SERVICE=1`, elevated runner): replace `elevated_without_flag_targets_system` with `elevated_stop_targets_scm_service` and add `elevated_install_uses_programdata_config` (asserts no scheduled task), `readable_config_dir_refused`, `user_task_and_service_coexist`, `legacy_sid_service_refused`, `empty_key_persisted_admin_only`, `non_elevated_targets_user_task`; delete `non_elevated_system_operations`, `config_owned_by_other_user`, `retarget_declined_and_approved`, `config_dir_owned_by_administrators`, `cross_scope_refused_both_ways`, `user_task_blocked_while_service_runs`, `pinless_service_refused`, `empty_key_never_written`; `already_running_is_clean_stop` fixture holds a second system-tier lock; move `unsafe_config_refused_then_fixed` and `junction_in_config_path_refused` to ProgramData paths.
- [ ] **Step 3:** `tests/service_user.rs` (flag/prompt tests removed in Task 4, guidance updated in Task 5): add `user_install_runs_no_loginctl` (PATH shim records invocations).
- [ ] **Step 4:** Docs: README tier section (paths table, who runs where, coexistence, port-clash note, admins run unelevated for their own user tier), GLOSSARY "User tier"/"System tier", CHANGELOG `### Breaking` (flags removed; system paths moved; legacy services refused with the manual steps from design Migration Plan).
- [ ] **Step 5:** Verification: `cargo fmt --check`; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; `cargo test --workspace --all-features --locked`; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`; privileged BusyBox suite in a disposable container via `docker` (Kiri). Record in `apply.md` which opt-in suites (Windows SCM/task, macOS launchd, systemd) ran where and which could not run locally.
- [ ] **Step 6:** Commit `test,docs: e2e and docs for elevation-derived tiers`.

---

## Scenario → test

Legend: **[new]** test written in the named task; **[mod]** existing test changed in the named task; **[keep]** existing test kept as is. Unit tests are `crates/host/src/<path>::<name>` unless a `tests/` file is named. Opt-in E2E: `service_system.rs` (BusyBox container, root), `service_*_windows.rs` (elevated Windows runner).

**cli-entrypoints**
- Scope flags are rejected → [new T5] `src/cli.rs::service_scope_flags_are_rejected`
- No prompt for a non-elevated user → [new T5] `src/cli_service.rs::install_reads_nothing_from_stdin`
- Short letters do not collide → [mod T5] `src/cli.rs::short_options_are_consistent_and_conflict_free`
- Long-only options → [mod T5] `src/cli.rs::long_only_options`
- Invalid config → [keep] `tests/service_user.rs::install_without_config_fails_with_guidance_and_registers_nothing`
- Role already running → [keep] `tests/role_ownership.rs::service_start_is_refused_while_the_role_runs`
- Status JSON → [keep] `tests/cli_golden.rs::status_json_forwards_file_bytes`
- Status from the system tier → [new T5] `tests/role_ownership.rs::status_reads_own_tier_snapshot_when_both_run`

**config-management-commands**
- Explicit path wins → [keep] `tests/config_lifecycle.rs::every_config_subcommand_honors_config_and_ignores_the_default`
- Access role uses the same flag → [keep] `tests/config_lifecycle.rs::set_creates_access_entry`
- Elevated keygen writes the system config → [new T5] `tests/config_lifecycle.rs::elevated_keygen_writes_system_config`
- Killed writer → [keep] `tests/config_lifecycle.rs::killed_or_concurrent_saves_never_expose_a_truncated_file`
- Permissions tightened → [keep] `tests/config_lifecycle.rs::valid_edit_is_applied_atomically_and_owner_only`
- Read-only target → [new T5] `tests/config_lifecycle.rs::read_only_config_mutation_is_refused_unchanged`
- Non-elevated user cannot read the system config → [new T6] `tests/service_system.rs::non_root_cannot_read_system_config`; Windows [new T6] `tests/service_system_windows.rs::empty_key_persisted_admin_only`

**frontend-independent-management**
- Install completes without a need → [new T4] `service/install.rs::install_returns_report_not_need`
- Unprivileged caller → [keep] `tests/service_user.rs::user_service_lifecycle_uses_the_pinned_absolute_paths`
- Elevated caller on Windows → [new T6] `tests/service_system_windows.rs::elevated_install_uses_programdata_config`
- BusyBox without privilege → [new T4] `service/backend/busybox.rs::busybox_non_root_mutation_is_permission_error`
- Other tier note → [new T4] `service/install.rs::install_proceeds_when_other_tier_runs_with_note`
- Windows system scope without elevation → [new T6] `tests/service_user_windows.rs::non_elevated_targets_user_task`
- Windows system scope uses only a Windows Service → [new T6] `tests/service_system_windows.rs::elevated_install_uses_programdata_config` (asserts no scheduled task)
- Start refused when the role already runs → [new T4] `service/verb.rs::start_refused_only_in_same_tier`
- Partial install reports applied effects → [keep] `service/install.rs::partial_install_reports_written_registration`
- Windows system reinstall fails after deleting the old service → [keep] `service/backend/windows_scm.rs::reinstall_failing_after_delete_reports_stop_and_delete`
- Windows system install rolls back a service stuck starting → [keep] `service/backend/windows_scm.rs::install_rolls_back_a_service_stuck_starting_with_all_effects`
- Windows system install rolls back a service that stopped by itself → [keep] `service/backend/windows_scm.rs::install_rolls_back_a_service_that_stopped_by_itself_without_a_stop_effect`

**role-ownership-shutdown**
- User run while the system service runs → [new T5] `tests/role_ownership.rs::run_prints_other_tier_notice_before_bind`
- System status while a user instance runs → [new T5] `tests/role_ownership.rs::status_prints_user_session_notice_in_system_tier`
- Probe failure → [new T2] `ownership.rs::probe_io_error_is_none`
- Accepted blind spot → [new T2] `tests/role_ownership.rs::blind_spot_user_before_presence_is_not_seen`
- Nothing running in the other tier → [new T5] `tests/role_ownership.rs::no_notice_when_other_tier_idle`
- Port clash is explained → [new T5] `tests/role_ownership.rs::port_clash_error_follows_notice`
- Four racing serve starts → [keep] `tests/role_ownership.rs::concurrent_starts_admit_exactly_one_owner`
- All-in-one first, standalone second → [keep] `tests/role_ownership.rs::second_serve_is_refused_across_binaries_overrides_and_start_order`
- Standalone first, all-in-one second → [keep] same test
- Second system-tier instance → [new T2] `tests/role_ownership.rs::second_system_instance_is_refused`
- Different state overrides share one lock → [keep] `tests/role_ownership.rs::second_serve_is_refused_across_binaries_overrides_and_start_order`
- System lock ignores the state override → [new T2] `tests/role_ownership.rs::system_lock_ignores_state_override`
- Access started with the serve key → [keep] `tests/role_ownership.rs::roles_with_distinct_keys_coexist_but_a_shared_key_is_rejected`
- Running other role with a corrupt owner record → [keep] `tests/role_ownership.rs::access_is_refused_when_the_running_serve_record_is_corrupt`
- Running other role with an unreadable owner record → [keep] `ownership.rs::publish_identity_refuses_when_the_running_other_role_record_is_unreadable`
- Running other role that has not published yet → [keep] `ownership.rs::publish_identity_proceeds_while_the_other_role_has_no_record`
- Corrupt record of a role that is not running → [keep] `ownership.rs::publish_identity_ignores_a_corrupt_record_of_a_stopped_role`
- Earlier-version system instance still running → [new fix] `ownership.rs::earlier_version_system_instance_refuses_claim`, `earlier_version_instance_is_probed_running`, `wait_released_in_waits_for_earlier_version_holder`; E2E `tests/service_system.rs::earlier_version_system_instance_refuses_the_new_claim`
- Non-admin exclusive holder is not mistaken for an earlier version → [new fix] `ownership.rs::non_admin_exclusive_holder_without_record_does_not_block`
- Corrupt machine record while the machine lock is held only shared → [new T2] `ownership.rs::corrupt_machine_record_ignored_for_user_claim` (the T2 rewrite folded the shared-hold case into it)
- Other tier is not compared → [new T2] `ownership.rs::identity_check_ignores_other_tier`
- Machine lock location → [keep] `ownership.rs::machine_dir_from_program_data_joins_iroh_tunnel_and_verifies` + Linux/macOS path assertion [new T2] `ownership.rs::machine_dir_is_platform_runtime_path`
- System lock held, user start → [new T2] `tests/role_ownership.rs::system_and_user_tiers_run_side_by_side` + notice [new T5] `run_prints_other_tier_notice_before_bind`
- User running, system start → [new T2] `tests/role_ownership.rs::system_and_user_tiers_run_side_by_side`
- Several user holders → [new T2] `tests/role_ownership.rs::several_user_holders_keep_presence_until_last_exits`
- Readable under a restrictive umask → [new T2] `ownership.rs::new_machine_dir_lock_and_presence_are_public_under_umask_077` (also asserts `serve.claim.lock` is `0600`) + [final] `ownership.rs::existing_claim_lock_keeps_its_mode`
- Non-admin holder cannot block the system service → [final] `ownership.rs::non_admin_holder_cannot_block_the_system_claim`, `ownership.rs::system_claim_lock_is_exclusive_and_detection_lock_shared` + [final] `tests/service_system.rs::non_admin_lock_holder_cannot_block_the_system_service`
- Second system instance is refused through the claim lock → [final] `ownership.rs::second_system_claim_is_refused_through_the_claim_lock` + [new T2] `tests/role_ownership.rs::second_system_instance_is_refused` + [final] `tests/service_system.rs::non_admin_lock_holder_cannot_block_the_system_service`
- Windows user task while the system service runs → [new T6] `tests/service_system_windows.rs::user_task_and_service_coexist` (user-task part skipped on always-elevated runners)
- Windows ProgramData cannot be resolved → [mod T2] `ownership.rs::tier_dirs_resolution_fails_when_program_data_cannot_be_resolved` (+ user claim proceeds)
- Install lock directories when ProgramData cannot be resolved → [mod T2] `ownership.rs::tier_dirs_resolution_fails_when_program_data_cannot_be_resolved` + `ownership.rs::machine_dir_from_program_data_propagates_a_resolution_failure`
- No machine lock file → [new T2] `ownership.rs::user_claim_without_machine_dir_creates_nothing`
- Presence file created by the system tier → [new T2] `ownership.rs::system_claim_creates_presence_file_unlocked` + [new T4] `service/install.rs::system_install_creates_presence_file`
- Owner record round trip → [keep] `ownership.rs::owner_record_round_trips_without_secret_material`
- Owner record cannot be written → [keep] `ownership.rs::claim_fails_when_the_owner_record_cannot_be_written`
- Machine record readable under a restrictive umask → [new T2] `ownership.rs::machine_owner_record_is_world_readable_under_umask_077`
- Lock held by another process → [keep] `tests/role_ownership.rs::service_start_is_refused_while_the_role_runs`
- Other tier running → [new T4] `service/verb.rs::start_refused_only_in_same_tier`

**self-update**
- Running image → [keep] `tests/update_zip_windows.rs::replacing_an_exe_that_another_process_runs_fails_and_keeps_it`
- Guidance for a system service → [mod T5] `tests/update_zip_windows.rs::assert_stop_guidance` + `update/replace.rs` hint unit tests

**system-service-management**
- Administrator edits config → [new T6] `tests/service_system.rs::admin_edit_applies_after_restart_keeping_root_0600`
- Invalid edit → [new T6] `tests/service_system.rs::invalid_edit_never_serves_and_keeps_owner_mode`; [keep] `service/platform.rs::invalid_toml_reports_failed_to_parse`
- Missing key → [mod T3] `service/platform.rs::empty_secret_key_under_service_is_persisted` + [new T6] `tests/service_system.rs::system_service_persists_generated_key_root_0600`
- Non-root BusyBox operation → [new T4] `service/backend/busybox.rs::busybox_non_root_mutation_is_permission_error` + [new T6] `tests/service_system.rs::busybox_non_root_mutation_refused`
- Root BusyBox install → [new T4] `service/backend/busybox.rs::busybox_root_install_has_no_user_scope_note`
- Service marker outside the SCM → [keep] `tests/service_system_windows.rs::scm_marker_outside_scm_is_config_error`
- Legacy pinned service → [new T3] `service/platform.rs::legacy_trusted_sid_is_refused` + [new T6] `tests/service_system_windows.rs::legacy_sid_service_refused`
- Role already running (start, same tier) → [new T4] `service/verb.rs::start_refused_only_in_same_tier`
- User install while the system service runs → [new T4] `service/install.rs::install_proceeds_when_other_tier_runs_with_note`
- System install while a user service runs → [new T6] `tests/service_system.rs::user_and_system_tiers_coexist_with_notices`
- Re-install over the same tier's own running service → [mod T4] `service/install.rs::a_hold_at_the_registering_tier_is_permitted`
- Root installs a system service → [new T4] `service/install.rs::system_install_defaults_to_machine_paths` (systemd unit path asserted through the fixture backend) + [new T6] `tests/service_system.rs::system_install_uses_etc_and_run` (BusyBox)
- User installs a user service → [keep] `tests/service_user.rs::user_service_lifecycle_uses_the_pinned_absolute_paths`
- Elevated Windows terminal → [new T6] `tests/service_system_windows.rs::elevated_stop_targets_scm_service` (user-task part skipped on always-elevated runners)
- Removed flags → [new T5] `src/cli.rs::service_scope_flags_are_rejected`
- Default system install → [new T4] `service/install.rs::system_install_defaults_to_machine_paths`
- macOS default paths pass the trust walk → [new T4 fix] `paths::macos_system_state_dir_is_library_application_support` + `service/install.rs::macos_system_state_dir_passes_the_trust_walk` (walks the real `/Library/Application Support` chain with the admin-only owners); reaching running needs launchd — not automated, recorded in apply.md
- Missing system config → [new T4] `service/install.rs::system_install_missing_config_names_path_and_keygen` + [new T6] `tests/service_system.rs::missing_system_config_names_keygen`
- User-owned config at install → [new T4] `service/install.rs::system_install_refuses_untrusted_config` + [new T6] `tests/service_system.rs::user_owned_config_refused_at_install`
- Config loosened after install → [new T6] `tests/service_system.rs::loosened_config_refused_at_service_start`
- Symlinked config directory → [new T3] `service/trust.rs::symlinked_component_is_refused`
- Link below the canonical root → [new T3] `service/trust.rs::symlinked_component_is_refused` (component below the canonical root)
- Foreground root run in a container → [new T3] `service/platform.rs::foreground_root_run_is_not_walked` + [final] `service/platform.rs::foreground_sudo_e_from_terminal_is_not_legacy` (`XPC_SERVICE_NAME=0` / empty)
- systemd unit environment → [new T4] `service/backend/systemd.rs::systemd_unit_has_service_marker_and_no_pin`
- Pinned registration after upgrade → [new T3] `service/platform.rs::legacy_trusted_uid_is_refused_before_reading_config` + [new T6] `tests/service_system.rs::legacy_pinned_script_is_refused_with_guidance`
- Windows pinned service after upgrade → [new T3] `service/platform.rs::legacy_trusted_sid_is_refused`
- Role already running (Windows recovery) → [mod T6] `tests/service_system_windows.rs::already_running_is_clean_stop` (fixture: second system-tier holder)
- User instance does not stop the service → [new T4] `service/backend/windows_scm.rs::scm_conflict_exit_only_for_same_tier`
- Crash recovery → [keep] `tests/service_system_windows.rs::crash_is_restarted`
- Config path with shell metacharacters → [keep] `service/backend/busybox_script.rs::metachar_config_path_reaches_daemon_literally`
- State and binary paths with shell metacharacters → [keep] `service/backend/busybox_script.rs::metachar_binary_and_state_paths_are_literal`
- Service marker without a pin → [new T4] `service/backend/busybox_script.rs::busybox_script_exports_service_marker_and_no_pin`
- Install from another working directory → [keep] `tests/service_user.rs::user_service_lifecycle_uses_the_pinned_absolute_paths`
- State directory override → [keep] `service/backend/systemd.rs::unit_pins_state_dir_and_quotes_paths_with_spaces`
- Missing config → [keep] `tests/service_user.rs::install_without_config_fails_with_guidance_and_registers_nothing`
- Read-only home → [mod T6] `tests/service_user.rs::denied_registration_fails_with_guidance_and_keeps_scope`
- Container without systemd → [keep] `service/backend/dispatch.rs::linux_backend_follows_the_init_system_for_every_tier`
- Windows scopes → [mod T4] `service/backend/dispatch.rs::windows_backend_follows_the_tier`
- Unit registration shape → [keep] `service/backend/systemd.rs::unit_matches_golden_for_every_case` (golden updated in T4)
- Unit text → [keep] `service/backend/systemd.rs::format_unit_uses_default_restart_policy`
- Re-install over a running unit → [new T4] `service/backend/systemd.rs::reinstall_restarts_a_running_unit` (asserts the `restart` command in the issued sequence)
- launchd registration shape → [new T4] `service/backend/launchd.rs::launchd_plist_has_service_marker_and_no_pin` + [keep] `plist_path_user_is_under_launchagents`, `service_target_uses_system_domain_for_system_scope`
- Plist document type → [keep] `service/backend/launchd.rs::plist_declares_apples_property_list_doctype`
- Load command order → [keep] `service/backend/launchd.rs::load_enables_the_label_before_bootout_and_bootstrap`
- Re-install over a loaded job → [keep] `service/backend/launchd.rs::load_enables_the_label_before_bootout_and_bootstrap`
- Enabling the label fails → [keep] `service/backend/launchd.rs::load_fails_without_bootstrapping_when_enable_fails`
- Booting out a job that is not loaded → [keep] `service/backend/launchd.rs::load_ignores_a_bootout_failure_and_still_bootstraps`
- Stop an already stopped job → [keep] `service/backend/launchd.rs::stop_captures_even_when_inheriting_and_reports_already_stopped`
- Bootstrap fails → [new T5] `src/render.rs::launchd_bootstrap_hint_names_print_disabled` (renders `ServiceNote::LaunchdBootstrapHint`; no existing test)
- Policy disabled → [keep] `service/backend/systemd.rs::format_unit_with_disabled_policy_omits_restart`
- systemd exempts the refused-start status → [keep] `service/backend/systemd.rs::unit_exempts_the_refused_start_status`
- launchd carries no exit-status exemption → [keep] `service/backend/launchd.rs::plist_carries_no_exit_status_exemption`
- Not on PATH → [keep] `src/cli_service.rs::cli_resolves_tunnel_binary_from_path_first`
- Root-run registration from a user-writable binary → [mod T4] `service/install.rs::plan_binary_system_tier_managed_when_refused`
- Full lifecycle → [keep] `tests/service_user.rs::user_service_lifecycle_uses_the_pinned_absolute_paths`
- No linger → [new T6] `tests/service_user.rs::user_install_runs_no_loginctl`
- Planted symlink → [keep] `service/trust.rs::publish_replaces_atomically_and_never_follows_a_planted_link`; Windows [keep] `service/trust_windows/win32.rs::publish_replaces_a_planted_symlink`
- Custom state directory → [keep] `tests/service_user_windows.rs::install_refuses_a_state_directory_the_task_cannot_carry`
- Not installed (task) → [keep] `tests/service_user_windows.rs::start_without_an_installed_task_names_the_install_command`
- Slow stop → [new T4] `service/backend/windows.rs::task_restart_times_out_without_starting` (task backend; `schtasks /End` with a lock still held past the wait → service error, no second start)
- System service restart → [keep] `tests/service_system_windows.rs::system_service_lifecycle`; [new T4] `service/system_verb.rs::system_restart_waits_for_system_holder_only`
- Service definition → [new T6] `tests/service_system_windows.rs::elevated_install_uses_programdata_config`
- Reinstall → [keep] `tests/service_system_windows.rs::reinstall_replaces_the_definition`
- Start failure rolls back → [keep] `tests/service_system_windows.rs::rollback_when_start_fails`
- Stop removes the status file → [keep] `tests/service_system_windows.rs::system_service_lifecycle`
- Stop pending while draining → [keep] `tests/service_system_windows.rs::stop_pending_reported_while_draining`
- Not installed (service) → [keep] `tests/service_system_windows.rs::missing_service_verbs`
- Idempotent stop and uninstall → [keep] `tests/service_system_windows.rs::missing_service_verbs`
- Directory pre-created by a user → [keep] `tests/service_system_windows.rs::untrusted_machine_dir_refused`
- Directory made untrusted after install → [keep] `tests/service_system_windows.rs::untrusted_machine_dir_refused`
- Readable config directory is refused → [new T3] `service/trust_windows/model.rs::config_dir_readable_by_users_is_refused` + `config/store.rs::elevated_store_write_refuses_readable_dir_before_creating_files` + [new T6] `tests/service_system_windows.rs::readable_config_dir_refused`
- Config directory hides secrets from users → [new T6] `tests/service_system_windows.rs::empty_key_persisted_admin_only`; key-present case [new T3] `service/platform.rs::service_start_refuses_readable_config_with_key`
- Task execution policy → [keep] `service/task_xml.rs::task_is_never_time_limited_nor_stopped_on_battery`
- Task definition → [keep] `service/task_xml.rs::task_runs_at_logon_of_the_current_user_with_a_limited_interactive_token`

**tunnel-configuration**
- Non-root user → [new T1] `tier.rs::from_elevated_maps_privilege`
- sudo → [new T1] `tests/config_paths.rs::system_tier_ignores_home_and_sudo_user`
- Administrator without elevation → [new T1] `tier.rs::from_elevated_maps_privilege` (non-elevated token → User); E2E [new T6] `tests/service_user_windows.rs::non_elevated_targets_user_task`
- Run as administrator → [new T6] `tests/service_system_windows.rs::elevated_install_uses_programdata_config`
- Unreadable legacy → [keep] `tests/config_paths.rs::copy_failure_never_creates_a_substitute_identity`
- System tier ignores the legacy file → [new T1] `tests/config_paths.rs::system_tier_does_not_migrate_macos_legacy`
- Unix default → [keep] `tests/config_paths.rs::config_default_is_home_dot_config_ignoring_xdg`
- Windows default → [keep] `tests/config_paths_windows.rs::config_default_is_local_app_data_ignoring_xdg`
- System tier defaults → [new T1] `paths::system_config_path_is_platform_machine_path`
- System tier ignores the invoking user's home → [new T1] `tests/config_paths.rs::system_tier_ignores_home_and_sudo_user`
- Unix status location → [keep] `tests/config_paths.rs::state_default_is_home_local_state_ignoring_xdg`
- System tier status location → [new T1] `tests/config_paths.rs::system_tier_state_dir_is_runtime_dir_without_home`
- macOS system tier status location → [new T4 fix] `paths::macos_system_state_dir_is_library_application_support`
- Relative HOME → [keep] `tests/config_paths.rs::relative_home_is_rejected_with_guidance`
- System tier without HOME → [new T1] `tests/config_paths.rs::system_tier_without_home_succeeds`
- Migration permissions and cleanup → [keep] `tests/config_paths.rs::legacy_is_copied_preserving_source_identity_and_permissions`
- Legacy copy → [keep] `tests/config_paths.rs::legacy_is_copied_preserving_source_identity_and_permissions`
- New config wins → [keep] `tests/config_paths.rs::new_config_wins_without_consulting_unreadable_legacy`
- Concurrent migration → [keep] `tests/config_paths.rs::concurrent_migrations_never_overwrite`
- Explicit config → [keep] `tests/config_paths.rs::explicit_config_skips_migration`
- System tier does not migrate → [new T1] `tests/config_paths.rs::system_tier_does_not_migrate_macos_legacy`

**unix-service-binary-trust**
- Binary in a root-owned directory → [keep] `service/trust.rs::root_only_accepts_a_system_binary`
- Binary owned by a user → [keep] `service/trust.rs::root_only_refuses_a_file_owned_by_the_test_user`
- Binary below a user-owned directory → [keep] `service/trust.rs::binary_refuses_untrusted_parent_naming_it`
- Group-writable binary → [keep] `service/trust.rs::binary_refuses_0775_naming_the_mode`
- Failed check writes no copy → [new T4] `service/install.rs::failed_trust_check_writes_no_managed_copy`
- Registration write fails after the copy → [keep] `service/install.rs::install_steps_register_failure_keeps_copy_in_applied`
- Boot start of a legacy registration → [new T6] `tests/service_system.rs::legacy_pinned_script_is_refused_with_guidance`
- Boot start of a current registration → [new T6] `tests/service_system.rs::boot_start_of_current_registration_is_not_binary_checked`
- Install from a root-owned location → [keep] `tests/service_system.rs::sm_bin_03_root_path_direct_and_uninstall_cleans`
- Install from a root-owned symbolic link → [keep] `service/service_binary.rs::choose_resolves_a_link_before_verifying`
- Install from a Homebrew prefix → [keep] `tests/service_system.rs::sm_bin_07_cargo_and_homebrew_prefixes_register_a_copy`
- Install from a cargo bin directory → [keep] same test
- Install note → [mod T4] `service/install.rs::install_steps_writes_copy_then_note_then_registers_copy` (note text without `--system`)
- Copy per role → [keep] `service/service_binary.rs::materialize_refreshes_and_leaves_other_role_bytes`
- Re-install refreshes the copy → [keep] `service/service_binary.rs::materialize_refreshes_and_leaves_other_role_bytes`
- Re-install from a trusted location → [keep] `service/install.rs::install_steps_direct_releases_copy`
- Restart after an upgrade of a managed install → [keep] `service/verb.rs::restart_never_writes_a_copy`
- Effective definition per backend → [keep] `service/effective_binary.rs::systemd_loaded_unit_yields_its_binary`, `launchd_program_line_wins`, `busybox_daemon_line_yields_binary`
- Loaded definition differs from the file → [keep] `service/verb.rs::guard_queries_the_loaded_definition_read_only`
- Trusted registered binary → [keep] `service/verb.rs::trusted_effective_binary_is_started`
- BusyBox script generations → [keep] `service/effective_binary.rs::busybox_single_quoted_daemon_yields_exact_path`, `busybox_legacy_double_quoted_daemon_still_parses`
- BusyBox DAEMON with shell expansion → [keep] `service/effective_binary.rs::busybox_daemon_with_expansion_is_unknown`
- Legacy registration with a user-writable binary → [mod T6] `tests/service_system.rs::sm_bin_02_legacy_unsafe_daemon_refuses_start` (guidance text)
- Directory loosened after install → [mod T4] `service/verb.rs::directory_loosened_names_the_directory` (guidance text)
- Unreadable binary in the registration → [mod T4] `service/verb.rs::unknown_definition_is_permission` (guidance text)
- Uninstall with a managed copy → [keep] `service/verb.rs::uninstall_releases_copy_after_registration_removed`
- Another registration still names the copy → [keep] `service/platform.rs::managed_copy_referenced_by_file`
- Drop-in still references the copy → [keep] `service/platform.rs::managed_copy_referenced_by_drop_in`
- Uninstall without a managed copy → [keep] `service/service_binary.rs::release_is_idempotent_when_absent`

**windows-service-trust**
- Config in the system-tier location → [new T6] `tests/service_system_windows.rs::elevated_install_uses_programdata_config`
- Config in a user's profile → [new T3] `service/trust_windows/model.rs::config_in_user_profile_is_refused`
- Binary writable by a user → [keep] `tests/service_system_windows.rs::binary_check_at_install_and_start`
- Unsafe config at service start → [mod T6] `tests/service_system_windows.rs::unsafe_config_refused_then_fixed` (ProgramData paths)
- Unsafe binary at CLI start → [keep] `tests/service_system_windows.rs::binary_check_at_install_and_start`
- Unsafe binary at install → [keep] `tests/service_system_windows.rs::binary_check_at_install_and_start`
- Fixed condition → [mod T6] `tests/service_system_windows.rs::unsafe_config_refused_then_fixed`

**workspace-boundaries**
- Permission failure keeps exit code 3 → [new T6] `tests/service_system.rs::user_owned_config_refused_at_install`
- Ownership conflict keeps exit code 6 → [keep] `tests/role_ownership.rs::second_serve_is_refused_across_binaries_overrides_and_start_order`
- Invalid configuration keeps exit code 2 → [new T5] `tests/config_lifecycle.rs::config_list_unparseable_exits_2`

**Added after apply (Windows CI):**
- Administrator with UAC disabled → [new fix] `service/always_elevated.rs::always_elevated_account_refuses_user_task` (injected token facts) + E2E `tests/service_user_windows.rs::user_task_refused_when_uac_is_disabled` (runs on UAC-off runners)
- Standard user with UAC disabled → [new fix] `service/always_elevated.rs::standard_user_with_uac_disabled_gets_a_user_task` (injected)
- Administrator with UAC enabled → [new fix] `service/always_elevated.rs::admin_with_uac_enabled_gets_a_user_task` (injected); E2E lifecycle tests skip on UAC-off runners
