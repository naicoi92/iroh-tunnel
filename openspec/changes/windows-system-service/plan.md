# Windows System Service Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md` (D1–D10); policy in `brainstorm.md`.

**Goal:** `iroh-tunnel <role> service … --system` on Windows registers and manages a LocalSystem Windows Service. The service reads the installing user's pinned config through a fail-closed DACL/reparse-point trust check, shares a machine-tier lock in `%ProgramData%\iroh-tunnel` with the user task, and drains on stop.

**Architecture:**
- **Policy modules.** Pure modules that compile on every platform carry every decision and are unit-tested on all CI hosts: `trust_windows::model`, the scope rules, `Pin`, `backend::windows_scm::{ServiceSpec, FailurePlan, ScmPort orchestration}` and `scm_runtime`'s state machine.
- **Adapters.** Thin `cfg(windows)` adapters do the effects: `trust_windows::win32`, the `windows-service` `ScmPort` impl, and the dispatcher. They plug into the existing seams: `install::plan_install`, `verb::backend_verb`, `status`, `ownership::{machine_dir, claim_machine}`, `service::load_config`, `status::writer`, `src/lifecycle.rs` and `src/app.rs`.
- **Unchanged.** The engine crate. Unix behaviour except the `Pin` type and the privilege-text helper.

**Tech Stack:** Rust 2021 (MSRV 1.91), tokio, `windows-service` (new, Windows only; version gated in Task 5), `windows-sys` 0.61 (direct, Windows only; already locked), `std::fs::File::try_lock*` (stable since 1.89), PowerShell 7 for `install.ps1`, GitHub Actions `windows-latest`.

## Global Constraints

- MSRV 1.91 (`Cargo.toml` `rust-version`). New crates must build on 1.91 and are `[target.'cfg(windows)'.dependencies]` only. A second `windows-sys` version pulled by `windows-service` is accepted (design risk).
- The engine crate is not modified. `iroh-tunnel-host` stays clap-free and never prints or exits; notes go through `ServiceNote` and errors through `CliError`.
- Exit codes are unchanged: 2 config, 3 permission, 5 service, 6 conflict.
- Every `unsafe` block lives in a `win32` or SCM adapter with a `// SAFETY:` comment (RS-12). Policy modules contain no `unsafe`.
- Never change the owner or DACL of user files. The only security descriptors this code writes are those of `%ProgramData%\iroh-tunnel` (D7) and of a directory `install.ps1` itself creates (D9).
- Never parse localized Windows text. State comes from structured APIs: SCM status, the existing locale-independent task listing in `backend/windows.rs::is_installed`, and SIDs.
- Trusted SIDs, exactly:
  - SYSTEM `S-1-5-18`
  - Administrators `S-1-5-32-544`
  - TrustedInstaller `S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464`

  The config/state set adds the pinned SID; the binary and machine sets do not.
- Service name: `iroh-tunnel-<role>` plus `instance_suffix()`. Environment keys: `IROH_TUNNEL_STATE_DIR`, `IROH_TUNNEL_TRUSTED_SID`, `IROH_TUNNEL_SCM` (= `1`).
- Timing values:

  | What | Value |
  |---|---|
  | Drain | 3 s |
  | Watchdog | 10 s |
  | StopPending wait hint | 15 s |
  | StartPending wait hint | 30 s |
  | Checkpoint ticker | 1 s |
  | Restart delay | 5 s |
  | Failure reset | 86 400 s |
  | Restart lock wait | 15 s |
  | Install wait for Running | 30 s |
  | Log cap | 10 MiB plus one `.1` rotation |

- Every commit is green on the local macOS host: `cargo build --workspace --all-targets && cargo test --workspace --all-features && cargo clippy --workspace --all-targets --all-features -- -D warnings`.
  - Pure modules that are consumed only by later tasks or only on Windows carry `#![cfg_attr(not(windows), allow(dead_code))]` until consumed.
  - Windows-only code is checked by the Windows CI unit cell of the pushed branch.
- **Push gate.** Pushing `feat/windows-system-service`, needed for `gh workflow run` and for the Windows CI cells, requires the owner's explicit approval in the Inbox. Until then, Windows results are recorded as "pending approval", never as passed.
- Commits that reuse #67 code carry `Co-authored-by: 23rd <23rd@vivaldi.net>`.

## Review Focus

1. **Paths with spaces or non-ASCII characters** (`C:\Users\Nguyễn Văn\…`, `C:\Program Files\…`) in the registered binary path and arguments, the pin record and the trust walk. Pinned by `service_spec_keeps_paths_with_spaces_and_unicode` (Task 7) and `pin_round_trips_unicode_path` (Task 4).
2. **A config on another volume or a UNC path.** The walk starts at that volume's root. UNC is refused, because a remote DACL cannot be verified. Pinned by `walk_starts_at_the_config_volume_root` and `unc_path_is_refused` (Task 3).
3. **Case and `\\?\` differences** between the pinned path and the path the service sees. Pinned by `pin_compares_paths_case_insensitively_on_windows` (Task 4) and `verbatim_prefix_is_normalized_before_walk` (Task 3).
4. **The SCM sends Stop twice, or Stop arrives before Running.** No panic, a single drain, and a clean final state. Pinned by `second_scm_stop_is_ignored` and `stop_before_running_ends_cleanly` (Task 10).
5. **`logs\` removed, or the log file locked.** Logging degrades to no log and never fails the role. Pinned by `log_writer_recreates_missing_logs_dir` and `log_writer_open_failure_is_not_fatal` (Task 6).

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `service::Pin` | modified | `pub struct Pin { pub config: PathBuf, pub owner: PinOwner }` (was `uid: u32`). `record()` emits `iroh-tunnel-pin config=<esc> uid=<n>` or `… sid=<S-1-…>`; `find()` accepts either and rejects both together. `pub(crate) fn same_target(&self, other: &Pin) -> bool` is exact on Unix, ASCII-case- and separator-insensitive on Windows. | public (host API) |
| `service::PinOwner` | new | `#[derive(Debug, Clone, PartialEq, Eq)] pub enum PinOwner { Uid(u32), Sid(String) }`; `Display`: `uid <n>` / `<SID>` | public |
| `service::account_name` | modified | `pub fn account_name(owner: &PinOwner) -> String`. Unix `Uid` → existing lookup. Windows `Sid` → `DOMAIN\name` via `LookupAccountSidW`. Otherwise `?`. | public |
| `trust_windows::model::Sid` | new | `#[derive(Debug, Clone, PartialEq, Eq, Hash)] pub(crate) struct Sid(pub String)`; consts `SYSTEM`, `ADMINISTRATORS`, `TRUSTED_INSTALLER`, `EVERYONE`, `CREATOR_OWNER`; `pub(crate) fn parse_sid(s: &str) -> Option<Sid>` (`^S-1-\d+(-\d+)+$`) | internal |
| `…::model::Ace` | new | `pub(crate) struct Ace { pub sid: Sid, pub mask: u32, pub inherit_only: bool, pub allow: bool }` | internal |
| `…::model::Descriptor` | new | `pub(crate) struct Descriptor { pub path: PathBuf, pub owner: Sid, pub dacl: Option<Vec<Ace>>, pub reparse: bool, pub is_dir: bool }` | internal |
| `…::model::TrustedSet` | new | `pub(crate) struct TrustedSet(Vec<Sid>)`; `fn for_user_files(pinned: &Sid) -> Self`; `fn for_binary() -> Self`; `fn machine() -> Self` (= binary set) | internal |
| `…::model::Refusal` | new | `pub(crate) enum Refusal { Reparse { path: PathBuf }, NoDacl { path: PathBuf }, Owner { path: PathBuf, owner: Sid }, Access { path: PathBuf, sid: Sid, rights: Vec<&'static str> }, Remote { path: PathBuf } }`. `Display` names the path, owner/SID and rights; `impl From<Refusal> for CliError` gives `CliError::Permission`. | internal |
| `…::model::components` / `classify` | new (deviated, Task 3) | `pub(crate) fn components(path: &str) -> Result<Vec<String>, Refusal>`: strips `\\?\`, refuses UNC with `Remote` (also relative, device and `..` paths), accepts `\` and `/`, returns root → target. `classify` is folded into it. **Deviation:** was `&Path -> Vec<PathBuf>`; changed to `&str`/`String` so Windows path splitting is tested on macOS, where `Path` does not treat `\` as a separator. | internal |
| `…::model::evaluate` | new | `pub(crate) fn evaluate(chain: &[Descriptor], trusted: &TrustedSet) -> Result<(), Refusal>`. The chain is root → target; masks are chosen per element by `is_dir` and position (target vs ancestor). | internal |
| `…::win32::{open_chain, Chain}` | new | `pub(crate) fn open_chain(path: &Path, last: LastAccess) -> anyhow::Result<Chain>`; `pub(crate) enum LastAccess { Inspect, ReadFile, WriteDir }`; `Chain { handles: Vec<OwnedHandle>, descriptors: Vec<Descriptor> }` | internal (cfg windows) |
| `trust_windows::{verify_dir, read_verified, verify_binary, publish, remove_in}` | new | `pub(crate) fn verify_dir(path: &Path, trusted: &TrustedSet) -> anyhow::Result<()>`; `pub(crate) fn read_verified(path: &Path, trusted: &TrustedSet) -> anyhow::Result<String>`; `pub(crate) fn verify_binary(path: &Path) -> anyhow::Result<()>`; `pub(crate) fn publish(dir: &Path, trusted: &TrustedSet, name: &str, temp: &str, bytes: &[u8]) -> anyhow::Result<()>`; `pub(crate) fn remove_in(dir: &Path, trusted: &TrustedSet, name: &str) -> anyhow::Result<()>` | internal (cfg windows) |
| `trust_windows::{token_user, account_of, owner_of}` | new | `pub(crate) fn token_user() -> anyhow::Result<Sid>`; `pub(crate) fn account_of(sid: &Sid) -> Option<String>`; `pub(crate) fn owner_of(path: &Path) -> anyhow::Result<Sid>` | internal (cfg windows) |
| `trust_windows::ENV_TRUSTED_SID` | new | `pub const ENV_TRUSTED_SID: &str = "IROH_TUNNEL_TRUSTED_SID"`; `pub(crate) fn trusted_sid_from_env() -> anyhow::Result<Option<Sid>>` (malformed → `CliError::Config`) | internal |
| `service::ENV_SCM` | new | `pub const ENV_SCM: &str = "IROH_TUNNEL_SCM"` | public (read by the root crate) |
| `backend::windows_scm::ServiceSpec` | new | `pub(crate) struct ServiceSpec { pub name: String, pub display_name: String, pub executable: PathBuf, pub launch_arguments: Vec<OsString>, pub env: Vec<(String, String)>, pub description: String, pub failure: Option<FailurePlan> }`; `pub(crate) fn render(role: &str, executable: &Path, config: &Path, state: &Path, pin: &Pin, policy: &RestartPolicy) -> anyhow::Result<ServiceSpec>` | internal |
| `…::FailurePlan` | new | `pub(crate) struct FailurePlan { pub restart_after: Duration, pub reset_after: Duration, pub on_non_crash_failures: bool }`; `fn from_policy(&RestartPolicy) -> Option<Self>` | internal |
| `…::ScmPort` | new | `pub(crate) trait ScmPort { fn state(&mut self, name: &str) -> anyhow::Result<Option<ScmState>>; fn create(&mut self, spec: &ServiceSpec) -> anyhow::Result<()>; fn delete(&mut self, name: &str) -> anyhow::Result<()>; fn start(&mut self, name: &str) -> anyhow::Result<()>; fn stop(&mut self, name: &str) -> anyhow::Result<()>; fn description(&mut self, name: &str) -> anyhow::Result<Option<String>>; fn executable(&mut self, name: &str) -> anyhow::Result<Option<PathBuf>>; }`; `pub(crate) enum ScmState { Stopped, StartPending, StopPending, Running, Other }`. Orchestration fns `install_with`, `stop_with`, `uninstall_with`, `start_with`, `restart_with` and `status_with` are generic over `ScmPort` and cross-platform. **Deviation (Task 7):** each takes `port: &mut dyn ScmPort, clock: &mut dyn Clock` (`trait Clock { now, sleep }`, `SystemClock` in production, `FakeClock` in tests). Time is injected through this one seam instead of separate closures. `restart_with` also takes `wait_released: FnOnce(Duration) -> Result<()>`. `status_with` returns `ScmStatus { Absent, Installed(ScmState) }`. Rollback and replace use `stop_and_delete` (stop, then wait for Stopped, then delete, then wait until gone), because a real `DeleteService` only marks a running service. | internal |
| `ownership::wait_released` | new | `pub(crate) fn wait_released(role: Role, timeout: Duration) -> anyhow::Result<()>` (polls `probe` every 100 ms; on timeout, a `CliError::Service` saying the old process did not stop) | internal |
| `ownership::windows_machine_dir` | new | `fn windows_machine_dir(program_data: &Path) -> PathBuf` (pure, `program_data.join("iroh-tunnel")`) | internal |
| `ownership::machine_dir` / `claim_machine` | modified (deviated, Task 6) | `pub(crate) fn machine_dir() -> Option<MachineDir>` with `pub(crate) struct MachineDir { pub path: PathBuf, pub verify: bool }`; `fn claim_machine(role, dir: &Path, verify: bool)`. `verify` is `true` only for the production Windows `%ProgramData%\iroh-tunnel`. Every test-utils location (override or implicit default) is `false` unless the test-only env `IROH_TUNNEL_TEST_VERIFY_MACHINE_DIR=1` is set. **Deviation:** added after Windows CI showed the test harness's user-owned `IROH_TUNNEL_MACHINE_LOCK_DIR` being refused by the trust check, which broke every role start in test builds. | internal |
| `trust_windows::map_create_error` | new (Task 6) | `pub(crate) fn map_create_error(code: u32) -> io::ErrorKind`: 5/1307/1314 → `PermissionDenied` (an unelevated create of the BA-owned dir), 183 → `AlreadyExists` (re-verified instead of hard-failing) | internal |
| `ownership::create_machine_dir` | new | `fn create_machine_dir(dir: &Path) -> std::io::Result<()>`. Unix = `create_dir(dir, 0o755)`. Windows = `CreateDirectoryW` with the D7 SDDL `O:BAG:SYD:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)`. The user tier keeps `create_dir`. | internal |
| `service::log_file::CappedLog` | new | `pub struct CappedLog`; `pub fn open(dir: &Path, role: &str, cap: u64) -> Option<CappedLog>`; `impl std::io::Write` (rotates to `.1` at the cap; open/reopen failures swallowed) | public (root-crate tracing) |
| `service::machine_log_dir` | new | `pub fn machine_log_dir() -> Option<PathBuf>` (`machine_dir()/logs`; `None` when the machine dir fails trust) | public |
| `service::is_elevated` (Windows) | modified | Same signature; Windows reads `TokenElevation` | public |
| `policy::ensure_privilege` text | modified | Unix text unchanged. Windows: ``a system service needs Administrator, and nothing was changed (no automatic elevation, no fallback to a user service). Run as administrator: open an elevated terminal and run `{argv}`, or use --user.``; argv joined with `windows_quote` (wrap in `"` when there is a space or `"`; inner `"` doubled) | public CLI text |
| `policy::ensure_supported` | modified (Task 9) | Windows branch removed | internal |
| `install::cross_scope_conflict` | new | `pub(crate) fn cross_scope_conflict(requested: ServiceScope, other_registered: bool, role: Role) -> anyhow::Result<()>` (`CliError::Service` naming `iroh-tunnel <role> service uninstall --user` or `--system`) | internal |
| `update::locked_hint` | new | `pub(crate) fn windows_locked_hint() -> &'static str` = `"; if a system service runs this executable, stop it first from an elevated terminal: iroh-tunnel <role> service stop --system"` (pure; appended on Windows) | public CLI text |
| root `cli::init_tracing_with` | new | `pub fn init_tracing_with(verbose: u8, quiet: bool, writer: tracing_subscriber::fmt::writer::BoxMakeWriter)`; `init_tracing` delegates with stderr | internal (root) |
| root `app::run_to_code` | new | `fn run_to_code(parsed: Cli, binary: Binary, ready: ReadyHook) -> i32`: body of today's `execute` after tracing init, returning the code instead of `process::exit`. `type ReadyHook = Option<Box<dyn FnOnce() + Send>>`, called from the `Run` arm's `on_ready` after `print_ready`. | internal (root) |
| root `lifecycle` SCM stop | modified | `pub fn scm_stop() -> &'static tokio::sync::watch::Sender<bool>` (cfg windows). `StopSignals::next()` also resolves when it flips to `true` and returns `"service stop request"`. It is a latch, so repeats are not delivered and `forced_exit` sees SCM stop only as the watchdog. | internal (root) |
| root `scm_runtime` | new | `pub fn run_as_service(parsed: Cli, binary: Binary) -> !` (cfg windows); `pub(crate) fn exit_to_scm(code: i32, stop_requested: bool) -> ScmExit` with `enum ScmExit { Clean, Specific(u32) }` (`0`, `6`, or any code after a stop request → `Clean`; else `Specific(code)`); `pub(crate) struct PendingReporter`; `pub(crate) struct StopLatch`; `pub(crate) fn dispatcher_error_code(raw_os: Option<i32>) -> i32` (`1063` → 2, else 1) | internal (root) |
| Test seams (test-utils only) | new | env `IROH_TUNNEL_TEST_EXIT_AFTER_MS` (the role exits 1 after N ms) and `IROH_TUNNEL_TEST_FAIL_BEFORE_READY` (exits 1 before `on_ready`); parsed by `pub(crate) fn test_fault() -> Option<TestFault>` in the root crate | test-only |
| `service::__test_verify_binary` | new | `#[cfg(all(windows, feature = "test-utils"))] pub fn __test_verify_binary(path: &Path) -> anyhow::Result<()>` | test-only |
| `install.ps1` `New-ProtectedBinDir` | new (conditional on Task 1) | Called only when the target dir did not exist: owner `BUILTIN\Administrators`; protected DACL = the parent's rules minus the current user SID and `S-1-3-0` | public script behaviour |
| Windows service registry values | new file format | `HKLM\SYSTEM\CurrentControlSet\Services\iroh-tunnel-<role>`: `Description` = pin record; `Environment` (REG_MULTI_SZ) = `IROH_TUNNEL_STATE_DIR=…`, `IROH_TUNNEL_TRUSTED_SID=…`, `IROH_TUNNEL_SCM=1` | public (registration) |
| `%ProgramData%\iroh-tunnel` layout | new file format | `<role>.lock`, `<role>.owner.json`, `logs\<role>.log`, `logs\<role>.log.1` | public |

---

## Task 1: Spike — measure the Windows assumptions — implements 1.1, 1.2

**Files:** Create `tests/windows_trust_probe.rs`. Modify `.github/workflows/e2e.yml` (temporary probe step), `openspec/changes/windows-system-service/design.md` (append `## Measurements`).

**Interfaces:** Consumes nothing. Produces the verdicts that gate Tasks 3, 5 and 12.

- [ ] **Step 1:** Write `#[cfg(windows)] #[test] fn probe_windows_trust_assumptions()`. It returns early with `eprintln!("skipped: …")` unless `IROH_TUNNEL_E2E_SERVICE=1`, as `tests/service_user_windows.rs:215` does. Through `powershell -NoProfile -Command`, it prints `EVIDENCE <label> <sddl>` lines:
  - `(Get-Acl <p>).Sddl` for each component of `%LOCALAPPDATA%\iroh-tunnel\config`;
  - the same for `C:\`, `%ProgramFiles%` and `%ProgramData%`;
  - the same for a fresh `%ProgramFiles%\iroh-tunnel-probe-<pid>` created with `New-Item -ItemType Directory`, then removed;
  - `whoami /user` and `whoami /groups /fo csv`. Only SIDs are read; the localized names are ignored.

  It has no assertions, and it uses no new dependency.
- [ ] **Step 2:** `cargo test --test windows_trust_probe` on macOS → `0 tests` (cfg windows). Commit (`test(windows): probe ACL and token assumptions for system services`).
- [ ] **Step 3 (stop condition):** Ask the owner in the Inbox for approval to push. Do not continue to Step 4 without it.
- [ ] **Step 4:** Add a temporary Windows step in `e2e.yml`: `cargo test --all-features --locked --test windows_trust_probe -- --nocapture` with `IROH_TUNNEL_E2E_SERVICE=1`. Then run `git push -u origin feat/windows-system-service` and `gh workflow run e2e.yml --ref feat/windows-system-service`.
- [ ] **Step 5:** Append `## Measurements` to `design.md`:
  - the raw `EVIDENCE` lines and the run URL;
  - a verdict per brainstorm assumption 1–4: profile ACL, UAC token user, `C:\` create-subdirectory, and whether a `Program Files` subdirectory created by the user grants that user write access (a CREATOR OWNER-derived entry);
  - the D9 decision (needed or not).

  If an assumption is false, send an Inbox `:::question` and stop before Task 3. Commit (`docs(openspec): record Windows trust measurements`).

---

## Task 2: Elevation and privilege guidance on Windows — implements 2.1, 2.3

**Files:** Modify `crates/host/src/service/is_elevated.rs`, `policy.rs`, `crates/host/Cargo.toml`.

**Interfaces:** Produces the Windows `is_elevated()` and the Windows `ensure_privilege` text. `ensure_supported` and the forced-user line in `install.rs:110` stay until Task 9, so Windows keeps refusing `--system` with exit 5 in between, and no intermediate commit reaches an unimplemented backend.

- [ ] **Step 1:** Add `windows-sys = { version = "0.61", features = ["Win32_Foundation", "Win32_Security", "Win32_System_Threading"] }` under a new `[target.'cfg(windows)'.dependencies]` in `crates/host/Cargo.toml`. Later tasks add their own features.
- [ ] **Step 2:** Failing tests in `policy.rs`:
  - `windows_quote_wraps_spaces_and_doubles_quotes` (all platforms): `windows_quote("C:\\Program Files\\x")` → `"\"C:\\Program Files\\x\""` and `windows_quote("a\"b")` → `"\"a\"\"b\""`;
  - `windows_privilege_text_names_run_as_administrator` (all platforms, through the pure `fn privilege_text(windows: bool, argv: &[String]) -> String`): `privilege_text(true, …)` contains `Run as administrator` and the quoted argv, and not `sudo`;
  - `unix_privilege_text_is_unchanged`: `privilege_text(false, …)` equals today's text byte for byte.
- [ ] **Step 3:** `cargo test -p iroh-tunnel-host policy::` → FAIL (functions missing).
- [ ] **Step 4:** Implement `windows_quote` and `privilege_text`. `ensure_privilege` calls `privilege_text(cfg!(windows), rerun)`. → PASS.
- [ ] **Step 5:** Implement `is_elevated()` for `cfg(windows)`: `OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY)`, then `GetTokenInformation(TokenElevation)`, then `TokenIsElevated != 0`. Any failure returns `false`; the handle is closed through `OwnedHandle`. Change the existing test's `cfg(not(unix))` branch to `cfg(all(not(unix), not(windows)))`, and add a `cfg(windows)` test: on the Windows CI runner, which is elevated, `is_elevated()` is `true`. Gate it on `IROH_TUNNEL_E2E_SERVICE=1`, because a developer's own shell may not be elevated.
- [ ] **Step 6:** Local gates are green. Commit (`feat(service): real elevation check and elevated-terminal guidance on Windows`).

---

## Task 3: Pure Windows trust model — implements 3.2

**Files:** Create `crates/host/src/service/trust_windows/mod.rs` and `crates/host/src/service/trust_windows/model.rs`. Modify `crates/host/src/service/mod.rs` (`mod trust_windows;` on all platforms, with `#![cfg_attr(not(windows), allow(dead_code))]` in `trust_windows/mod.rs`).

**Interfaces:** Produces `Sid`, `parse_sid`, `Ace`, `Descriptor`, `TrustedSet`, `Refusal`, `components` and `evaluate` (see Contracts), plus these rights constants:

| Constant | Value |
|---|---|
| `FILE_WRITE_DATA` / `FILE_ADD_FILE` | `0x2` |
| `FILE_APPEND_DATA` / `FILE_ADD_SUBDIRECTORY` | `0x4` |
| `FILE_WRITE_EA` | `0x10` |
| `FILE_DELETE_CHILD` | `0x40` |
| `DELETE` | `0x1_0000` |
| `WRITE_DAC` | `0x4_0000` |
| `WRITE_OWNER` | `0x8_0000` |
| `GENERIC_ALL` | `0x1000_0000` |
| `GENERIC_WRITE` | `0x4000_0000` |

**Mask rule:**

| Element | Refused mask |
|---|---|
| Target that is a file | `FILE_WRITE_DATA \| FILE_APPEND_DATA \| FILE_WRITE_EA \| DELETE \| WRITE_DAC \| WRITE_OWNER \| GENERIC_WRITE \| GENERIC_ALL` |
| Target that is a directory | the same mask plus `FILE_DELETE_CHILD`, minus `FILE_ADD_FILE`, `FILE_ADD_SUBDIRECTORY` and `FILE_WRITE_EA` (an untrusted SID may create entries, but not replace trusted ones) |
| Ancestor | `DELETE \| FILE_DELETE_CHILD \| WRITE_DAC \| WRITE_OWNER \| GENERIC_ALL` |

`FILE_WRITE_ATTRIBUTES` is deliberately accepted: it cannot change content or identity.

- [ ] **Step 1:** Write the failing tests in `model.rs`. They use the helper `fn d(path, owner, aces, reparse, is_dir) -> Descriptor`, with `pinned = Sid("S-1-5-21-1-2-3-1001")` and `bob = Sid("S-1-5-21-1-2-3-1002")`.

  | Test | Expected result |
  |---|---|
  | `trusted_profile_chain_passes` | `Ok(())` |
  | `reparse_component_is_refused` | `Refusal::Reparse` for the middle element |
  | `null_dacl_is_refused` | `Refusal::NoDacl` |
  | `untrusted_owner_is_refused` | `Refusal::Owner { owner: bob }` |
  | `inherit_only_full_control_is_ignored` | an inherit-only `CREATOR_OWNER` `GENERIC_ALL` entry → `Ok` |
  | `each_file_write_right_is_refused` | each of the 8 file rights for Everyone on a file target → `Refusal::Access` whose `rights` contains that right's name |
  | `directory_target_refuses_delete_child_but_allows_create` | |
  | `each_ancestor_replace_right_is_refused` | |
  | `create_rights_on_ancestor_are_accepted` | `FILE_ADD_FILE \| FILE_ADD_SUBDIRECTORY` for `S-1-5-11` on `C:\` → `Ok` |
  | `write_attributes_is_accepted` | |
  | `deny_aces_are_ignored` | |
  | `binary_set_excludes_pinned_user` | |
  | `refusal_names_path_owner_sid_and_rights` | |
  | `unc_path_is_refused` | `components("\\\\server\\share\\x")` → `Refusal::Remote` |
  | `verbatim_prefix_is_normalized_before_walk` | `components("\\\\?\\C:\\a")` equals `components("C:\\a")` |
  | `walk_starts_at_the_config_volume_root` | `components("D:\\cfg\\s.toml")` = `["D:\\", "D:\\cfg", "D:\\cfg\\s.toml"]` |
  | `parse_sid_accepts_and_rejects` | |

  `components` is written over `&str` with `\` separators, so the tests run on macOS too.
- [ ] **Step 2:** `cargo test -p iroh-tunnel-host trust_windows::model` → FAIL.
- [ ] **Step 3:** Implement the module per Contracts and the mask rule. `evaluate` checks, in order, reparse, DACL presence, owner, then allow entries that are not inherit-only and whose SID is untrusted, and returns the first offending element.
- [ ] **Step 4:** → PASS. Local gates are green. Commit (`feat(service): pure Windows trust model for system services`).

---

## Task 4: `Pin` owner and account names — implements 4.1

**Files:**
- Modify `crates/host/src/service/pin.rs`, `account_name.rs`, `system.rs` (lines 34 and 194 construct `PinOwner::Uid`; `retarget_decision` uses `same_target`).
- Modify `backend/{systemd.rs:64, launchd.rs:120, busybox.rs:70}` to read `PinOwner::Uid`, and `mod.rs` to export `PinOwner`.
- Modify `src/cli_service.rs:214-235`.

**Interfaces:** Produces `Pin`, `PinOwner`, `same_target` and `account_name(&PinOwner)`.

- [ ] **Step 1:** Failing tests in `pin.rs`:
  - `sid_record_round_trips`;
  - `legacy_uid_record_still_parses`, on `# iroh-tunnel-pin config=/home/a/c.toml uid=1000`;
  - `record_with_both_uid_and_sid_is_malformed` → `None`;
  - `pin_compares_paths_case_insensitively_on_windows`, through the pure `fn same_target_with(windows: bool, a, b)`, so it runs everywhere.

  Add `pin_round_trips_unicode_path` (`C:\Users\Nguyễn Văn\AppData\Local\iroh-tunnel\config\serve.toml`) as a regression guard. Escaping already supports it; it is not expected to fail first.
- [ ] **Step 2:** `cargo test -p iroh-tunnel-host pin::` → FAIL (compile: `owner` missing).
- [ ] **Step 3:** Implement. On Unix, a `PinOwner::Sid` pin counts as an unknown owner in `retarget_decision`. In the backends, `match &pin.owner { PinOwner::Uid(u) => …, PinOwner::Sid(_) => return Err(CliError::Service("internal: Unix registration with a SID pin".into()).into()) }`.
- [ ] **Step 4:** `account_name(&PinOwner)`: Windows `Sid` returns `?` until Task 5 provides `account_of`. Port `src/cli_service.rs` to `account_name(&p.owner)` and `{}` of `p.owner`.
- [ ] **Step 5:** Local gates green; `tests/service_system.rs` compiles unchanged. Commit (`refactor(service): pin carries a uid or a SID`).

---

## Task 5: Win32 trust adapter and verified I/O — implements 3.1, 3.3, 3.4

**Files:** Create `crates/host/src/service/trust_windows/win32.rs` (cfg windows). Modify `trust_windows/mod.rs`, `account_name.rs` (Windows `Sid` → `account_of`), `crates/host/Cargo.toml`.

**Interfaces:** Consumes `model::*`. Produces `open_chain`, `LastAccess`, `verify_dir`, `read_verified`, `verify_binary`, `publish`, `remove_in`, `token_user`, `account_of`, `owner_of` and `trusted_sid_from_env`.

- [ ] **Step 1 (dependency gate):** Resolve the newest `windows-service` whose `rust-version` is ≤ 1.91, with `cargo info windows-service@<v>` (this needs network). Add it to `crates/host/Cargo.toml` `[target.'cfg(windows)'.dependencies]` only; the root crate adds it in Task 10. Pin it with a comment if it is not the latest. Extend the `windows-sys` features with `Win32_Security_Authorization`, `Win32_Storage_FileSystem`, `Win32_System_Memory`, `Win32_UI_Shell`, `Win32_System_Com` and `Win32_System_IO`. Run `cargo tree --target x86_64-pc-windows-msvc -i windows-sys` (works offline from the lockfile), and record the resolved `windows-service` version, its `rust-version` and every `windows-sys` version in the commit message.
- [ ] **Step 2:** Failing Windows tests in `win32.rs` (`#[cfg(all(test, windows))]`). They use `tempfile` and shape ACLs with `icacls`, which is checked by exit status only.
  - `profile_temp_dir_passes_for_current_user`
  - `everyone_modify_on_config_is_refused` (`icacls f /grant *S-1-1-0:M`); the text contains `S-1-1-0` and the path.
  - `junction_in_chain_is_refused` (`cmd /c mklink /J`); the text contains `reparse point`.
  - `null_dacl_is_refused` (`SetSecurityInfo` with a null DACL); the text contains `access list`.
  - `read_verified_returns_content_of_the_verified_handle`
  - `publish_replaces_a_planted_symlink`: `mklink f target` (an elevated runner can create symlinks), then `publish`; `f` is a regular file and `target` is unchanged.
  - `publish_rename_falls_back_without_posix_semantics`: the injected rename returns `ERROR_INVALID_PARAMETER` → falls back to `FileRenameInfo` with `ReplaceIfExists`.
  - `owner_of_temp_file_is_current_user`
- [ ] **Step 3:** `open_chain(path, last)`:
  - `model::components` gives the component list.
  - For each element, `CreateFileW(access, FILE_SHARE_READ | FILE_SHARE_WRITE, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)`. Ancestors get `READ_CONTROL | FILE_READ_ATTRIBUTES`. The last element adds `FILE_READ_DATA` (`ReadFile`), or `FILE_ADD_FILE | FILE_LIST_DIRECTORY` (`WriteDir`), or nothing (`Inspect`).
  - `GetFileInformationByHandle` reads the attributes.
  - `GetSecurityInfo(h, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION)` reads owner and DACL; the ACL is walked with `GetAce`, and SIDs are converted with `ConvertSidToStringSidW` + `LocalFree`.
  - The `OwnedHandle`s are held in `Chain`.
- [ ] **Step 4:** The verified I/O functions:
  - `read_verified` = `open_chain(ReadFile)` + `evaluate` + `ReadFile` on the last handle.
  - `verify_binary` = `open_chain(Inspect)` + `evaluate(TrustedSet::for_binary())`.
  - `publish`:
    1. `open_chain(dir, WriteDir)` + `evaluate`.
    2. Create the temp file with `CreateFileW(dir\temp, GENERIC_WRITE | DELETE | SYNCHRONIZE, 0, CREATE_NEW, FILE_FLAG_OPEN_REPARSE_POINT)`.
    3. Write the bytes and call `FlushFileBuffers`.
    4. `SetFileInformationByHandle(FileRenameInfoEx, { RootDirectory: dir handle, flags: FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS, name })`. On `ERROR_INVALID_PARAMETER` or `ERROR_NOT_SUPPORTED`, retry with `FileRenameInfo { ReplaceIfExists: TRUE, RootDirectory, name }`.
    5. Delete the temp on any error.
  - `remove_in` opens the name with `DELETE` + `FILE_FLAG_OPEN_REPARSE_POINT` under the held directory, and sets `FileDispositionInfo`.

  Every `unsafe` block gets a `// SAFETY:` comment.
- [ ] **Step 5:** `token_user` (`TokenUser`), `account_of` (`LookupAccountSidW` → `DOMAIN\name`) and `owner_of` (`open_chain(Inspect)`, last owner). `trusted_sid_from_env` returns `Ok(None)` when the variable is absent. A malformed value gives `CliError::Config("IROH_TUNNEL_TRUSTED_SID must be a SID; re-run the service install")`; the pure parse is covered by `parse_sid_accepts_and_rejects`.
- [ ] **Step 6:** Local gates are green (the module is cfg windows); the Windows unit cell passes after push approval. Commit (`feat(service): Win32 trust adapter with verified reads and link-safe publish`).

---

## Task 6: Machine directory, machine lock, restart wait and service log — implements 5.1, 5.2, 5.3

**Files:** Modify `crates/host/src/ownership.rs`. Create `crates/host/src/service/log_file.rs`. Modify `crates/host/src/service/mod.rs` (export `CappedLog` and `machine_log_dir`).

**Interfaces:** Consumes `trust_windows::{verify_dir, TrustedSet::machine}`. Produces `windows_machine_dir`, `create_machine_dir`, `wait_released`, `CappedLog` and `machine_log_dir`.

- [ ] **Step 1:** Failing in-file tests in `ownership.rs`. They avoid the Unix-only `tests/role_ownership.rs` and call `claim_machine(role, &dir)` directly, since that function already takes the directory.
  - `windows_machine_dir_is_programdata_child` (all platforms): `windows_machine_dir(Path::new("C:\\ProgramData"))` ends with `iroh-tunnel`.
  - `machine_lock_exclusive_then_shared_is_refused` (all platforms, tempdir): a first `claim_machine` gives `Exclusive`. A second open of the same file read-only with `try_lock_shared` is busy. Dropping the first claim makes it free.
  - `untrusted_existing_machine_dir_is_refused` (cfg windows): pre-create a tempdir and `icacls /grant *S-1-1-0:M` it. `claim_machine(role, &dir)` gives `CliError::Permission` naming the dir and `S-1-1-0`, and its SDDL is identical before and after. The trust check runs before any create, so the error is never mistaken for `is_not_elevated`.
  - `wait_released_times_out` (all platforms): hold the user-tier lock in the test, then `wait_released(role, 200 ms)` gives `CliError::Service` containing `did not stop`. After a drop, it returns `Ok` quickly.
- [ ] **Step 2:** → FAIL.
- [ ] **Step 3:** The non-test-utils `machine_dir()` on Windows is `windows_machine_dir(&known_folder(FOLDERID_ProgramData))`, where `known_folder` uses `SHGetKnownFolderPath` + `CoTaskMemFree` inside `trust_windows::win32`. The test-utils variant is unchanged. In `claim_machine`'s elevated create path, call `trust_windows::verify_dir(dir, &TrustedSet::machine())` on Windows when the dir exists. Otherwise call `create_machine_dir(dir)` (the SDDL create on Windows; Unix is unchanged). Add `wait_released`, built on `probe`. → PASS.
- [ ] **Step 4:** Failing tests in `log_file.rs` (all platforms):
  - `writes_append_to_role_log`
  - `rotates_at_cap`: with a cap of 100, writing 150 bytes leaves `serve.log.1` and a `serve.log` under 100 bytes.
  - `log_writer_recreates_missing_logs_dir`
  - `log_writer_open_failure_is_not_fatal`: when `logs` is a regular file, `open` returns `None`, or `write` returns `Ok(n)` and drops the bytes.
- [ ] **Step 5:** Implement `CappedLog` with append mode, size tracking and rename-to-`.1`; errors are swallowed. `machine_log_dir()` returns `machine_dir()?/logs` only when the trust check of `machine_dir()` passes (Windows), and `None` otherwise. → PASS.
- [ ] **Step 6:** Local gates are green. Commit (`feat(ownership): Windows machine tier in ProgramData, restart wait and a capped service log`).

---

## Task 7: Windows Service spec and SCM orchestration (pure) — implements 6.1

**Files:** Create `crates/host/src/service/backend/windows_scm.rs`, compiled on all platforms with `#![cfg_attr(not(windows), allow(dead_code))]`. The name avoids clashing with the `windows_service` crate and the existing `backend/windows.rs`. Modify `backend/mod.rs` to add an unconditional `pub(super) mod windows_scm;`.

**Interfaces:** Consumes `Pin`/`PinOwner`, `RestartPolicy` (`types.rs:201`) and `instance_suffix()` (`registration.rs:12`). Produces `ServiceSpec::render`, `FailurePlan::from_policy`, `ScmPort`, `ScmState`, the `*_with` orchestration and `service_name(role) -> String`.

- [ ] **Step 1:** Failing tests, using a `FakeScm` that records calls and has programmable failures:

  | Test | Expected result |
  |---|---|
  | `service_spec_for_serve` | name `iroh-tunnel-serve`; `launch_arguments == ["serve", "run", "--config", <config>]`; `env` holds the three keys; `description == pin.record()` |
  | `service_spec_keeps_paths_with_spaces_and_unicode` | `executable == C:\Program Files\iroh-tunnel\iroh-tunnel.exe` and the config argument is `C:\Users\Nguyễn Văn\AppData\Local\iroh-tunnel\config\serve.toml`, both unmodified (`windows-service` quotes them when building the SCM command line) |
  | `failure_plan_from_default_policy` | `Some(FailurePlan { restart_after: 5 s, reset_after: 86400 s, on_non_crash_failures: true })` |
  | `failure_plan_disabled_is_none` | |
  | `render_requires_a_sid_pin` | |
  | `install_replaces_existing` | `stop`, then wait for `Stopped`, `delete`, wait for gone, `create`, `start`, wait for `Running` |
  | `install_rolls_back_when_start_fails` | `start` errs → `delete` called, the original error returned |
  | `install_rolls_back_when_never_running` | the state stays `StartPending` past 30 s (fake clock) → delete, then `CliError::Service` |
  | `stop_of_stopped_is_ok` | |
  | `uninstall_absent_is_ok` | |
  | `start_absent_is_service_error_with_install_hint` | contains `service install --system` |
  | `restart_waits_for_lock_then_starts` | the injected `wait_released` is called between stop and start |
  | `restart_times_out_without_starting` | |

  Time and the lock wait are injected through closures.
- [ ] **Step 2:** → FAIL.
- [ ] **Step 3:** Implement per Contracts, following #67's argument layout and failure-action shape.
- [ ] **Step 4:** → PASS. Local gates are green. Commit with the `Co-authored-by: 23rd <23rd@vivaldi.net>` trailer (`feat(service): Windows Service definition and SCM orchestration`).

---

## Task 8: Trusted config load and status writes under the service — implements 7.4

This task comes before any wiring, so no commit can run a service that reads the user's config unverified.

**Files:** Modify `crates/host/src/service/platform.rs` (`load_config`, Windows branch) and `crates/host/src/status/writer.rs` (`#[cfg(windows)]` branches beside those at lines 179 and 277).

**Interfaces:** Consumes `trusted_sid_from_env`, `verify_dir`, `read_verified`, `publish`, `remove_in`, `TrustedSet::for_user_files` and `ENV_SCM`.

- [ ] **Step 1:** Failing tests, cfg windows. They set the environment through a scoped guard and use a tempdir.

  | Test | Expected result |
  |---|---|
  | `service_without_sid_is_refused` | `IROH_TUNNEL_SCM=1` with no SID → `CliError::Permission` containing `service install --system` |
  | `empty_secret_key_under_service_is_config_error_and_file_unchanged` | message contains `config keygen`; file bytes and SDDL unchanged |
  | `invalid_toml_reports_failed_to_parse` | |
  | `writable_config_is_permission_error` | |
  | `status_publish_goes_through_verified_dir` | |

  Also add a pure all-platform test, `windows_empty_key_text_names_keygen`, over `fn empty_key_text(path) -> String`.
- [ ] **Step 2:** → FAIL.
- [ ] **Step 3:** Implement the Windows `load_config`:
  - **SID present:** verify the state dir, then `read_verified`, then `from_toml`. An empty key gives `empty_key_text`.
  - **SID absent and `ENV_SCM=1`:** permission refusal.
  - **Otherwise:** `C::load`.

  In `writer.rs`, take the `trust_windows::publish`/`remove_in` branch when `trusted_sid_from_env()?` is `Some`. → PASS.
- [ ] **Step 4:** Local gates are green. Commit (`feat(service): trusted config load and link-safe status under a Windows service`).

---

## Task 9: SCM adapter and wiring into the operations — implements 6.2, 6.3, 4.2, 2.2

**Files:**
- Modify `windows_scm.rs` (adds a `#[cfg(windows)] mod scm_impl` implementing `ScmPort` on `windows-service`).
- Create `crates/host/src/service/system_windows.rs` (pure resolver on all platforms, Win32 wrapper cfg windows).
- Modify `install.rs` (Windows `plan_system_install` replaces the stub at `install.rs:193`; remove the forced-user line at `:110`; rewrite the test at `:447`), `policy.rs` (remove the Windows branch of `ensure_supported`), `verb.rs` (dispatch by scope in the `cfg(windows)` arm at `:117`; binary re-check for `Start`/`Restart`), `status.rs`, `platform.rs` (new `cfg(windows)` `existing_registration`), `native.rs` (`InstalledAt::WindowsService`, with every exhaustive match updated), `start.rs`, `stop.rs`, `restart.rs` and `uninstall.rs` (the `cfg!(windows)` test branches at `:36`), and `tests/service_user_windows.rs:161`.

**Interfaces:** Consumes Tasks 4–8. Produces the Windows system scope end to end on the CLI side, plus `system_windows::resolve_target(req: &Request, token: &Sid, owner_of: &dyn Fn(&Path) -> Option<Sid>, local_appdata: &dyn Fn(&Sid) -> anyhow::Result<PathBuf>, account_of: &dyn Fn(&Sid) -> Option<String>) -> anyhow::Result<Target>`.

- [ ] **Step 1 (commit A, resolver):** Failing pure tests in `system_windows.rs`:
  - `defaults_use_token_user_paths`
  - `config_owned_by_other_user_is_config_error` (the message names both accounts; the class is `Config`)
  - `config_dir_owned_by_administrators_defers_to_token`
  - `config_dir_owned_by_system_defers_to_token`
  - `config_dir_owned_by_trusted_installer_defers_to_token`

  → FAIL. Implement → PASS. Commit (`feat(service): resolve the installing user for Windows system scope`).
- [ ] **Step 2 (commit B, SCM adapter):** Implement `scm_impl::Scm: ScmPort`:
  - `ServiceManager::local_computer(None, CONNECT | CREATE_SERVICE)`.
  - `create_service(ServiceInfo { name, display_name, service_type: OWN_PROCESS, start_type: AutoStart, error_control: Normal, executable_path: spec.executable, launch_arguments: spec.launch_arguments, dependencies: [], account_name: None, account_password: None }, CHANGE_CONFIG | START | STOP | DELETE | QUERY_STATUS | QUERY_CONFIG)`.
  - Then `set_description`; the `Environment` REG_MULTI_SZ through `RegSetKeyValueW(HKLM, "SYSTEM\\CurrentControlSet\\Services\\<name>", "Environment", REG_MULTI_SZ, …)`; `update_failure_actions` + `set_failure_actions_on_non_crash_failures(true)`.
  - `state` → `query_status().current_state`. `description` and `executable` → `query_config()` and the `Description` registry value.
  - "Absent" means `ERROR_SERVICE_DOES_NOT_EXIST` (1060).

  It adds the `Win32_System_Registry` feature. Windows test `scm_impl_round_trips_a_spec`, gated on the e2e env and using a suffixed name: create, read description and executable, delete. Commit (`feat(service): SCM adapter`, with the `Co-authored-by: 23rd` trailer).
- [ ] **Step 3 (commit C, wiring):** Remove the Windows branch of `ensure_supported` and the line `install.rs:110`. Implement the `cfg(windows)` `plan_system_install`, in order:
  1. `ensure_privilege`.
  2. `resolve_target`. The cross-scope check is not part of this commit; Task 11 inserts it right after `ensure_privilege`.
  3. The machine-dir trust check, when the dir exists → exit 3 naming the dir (spec: Directory pre-created by a user).
  4. `verify_dir(state)`, `read_verified(config)`, `check_installable_text`, `verify_binary(&req.tunnel_executable)`.
  5. Retarget, via `existing_registration` = `Scm.description(name)` and `Pin::find` + `same_target`.
  6. `InstallPlan { pin: Some(Pin { owner: PinOwner::Sid(..) }) }`.

  `backend_verb` on Windows matches the scope: `User` → `windows::*`, `System` → `windows_scm::*_with(&mut Scm, …)`. For system `Start`/`Restart`, `verify_binary(Scm.executable(name)?)` runs before the backend call. Rewrite the tests:
  - `install.rs:447` becomes `windows_elevated_install_without_flag_plans_system` (with an injected `ScmPort`).
  - `start.rs`, `stop.rs`, `restart.rs` and `uninstall.rs` `:36`: on Windows a non-elevated system scope is now `CliError::Permission`, the same as Unix.
  - `tests/service_user_windows.rs:161` `system_scope_is_unsupported_on_windows` becomes `non_elevated_system_scope_is_permission_error`, `#[ignore = "needs the restricted-token helper (Task 13)"]`.

  Note in the commit message that Windows CI loses non-elevated system coverage until Task 13. Add the pure unit test `verb_start_system_checks_binary_first`, with an injected verifier that fails, so the backend is never called. Local gates are green. Commit (`feat(service)!: Windows system scope through the Service Control Manager`).

---

## Task 10: Running under the SCM — implements 7.1, 7.2, 7.3

**Files:**
- Create `src/scm_runtime.rs`. The pure parts are cross-platform; the dispatcher is cfg windows.
- Modify `src/app.rs`: refactor `execute` into `run_to_code`, branch on `ENV_SCM` before tracing, add the `ReadyHook`.
- Modify `src/lifecycle.rs` (`scm_stop` watch, `StopSignals`), `src/cli.rs` (`init_tracing_with`), `src/main.rs` or `src/lib.rs` (`mod scm_runtime`), and the root `Cargo.toml` (adds `[target.'cfg(windows)'.dependencies] windows-service`, the same version as Task 5).

**Interfaces:** Consumes `ENV_SCM`, `CappedLog`, `machine_log_dir`, `forced_exit` and `wait_for_signal`. Produces `run_as_service`, `exit_to_scm`, `ScmExit`, `PendingReporter`, `StopLatch`, `dispatcher_error_code`, `run_to_code`, `ReadyHook`, `init_tracing_with` and `test_fault`.

- [ ] **Step 1 (refactor, behaviour unchanged):** Move the body of `execute` after `init_tracing` into `fn run_to_code(parsed, binary, ready: ReadyHook) -> i32`. `execute` becomes `init_tracing(...); std::process::exit(run_to_code(parsed, binary, None))`. Add `init_tracing_with`. The existing golden and CLI tests (`tests/cli_golden.rs`, `tests/cli_usage.rs`) pin the behaviour and stay green. Commit (`refactor(cli): return the exit code from the run path`).
- [ ] **Step 2:** Failing pure tests in `scm_runtime.rs`:
  - `exit_to_scm_maps_codes`: `(0, false)` → `Clean`, `(6, false)` → `Clean`, `(2, false)` → `Specific(2)`, `(1, false)` → `Specific(1)`, `(130, true)` → `Clean`, `(1, true)` → `Clean`.
  - `stop_pending_reports_15s_hint_and_increments_checkpoint`.
  - `start_pending_reports_30s_hint_and_ticks`.
  - `second_scm_stop_is_ignored`.
  - `stop_before_running_ends_cleanly`.
  - `dispatcher_error_1063_is_config_error`: `dispatcher_error_code(Some(1063))` = 2, `Some(5)` = 1.
  - `test_fault_parses_env` (cfg feature test-utils).

  Failing test in `lifecycle.rs` (cfg windows): `scm_stop_triggers_the_same_drain`. Setting `scm_stop()` to `true` makes `wait_for_signal()` resolve, and the log contains `received service stop request`.
- [ ] **Step 3:** → FAIL. Implement the pure parts, then add `scm_stop()` as a `OnceLock<watch::Sender<bool>>`. `StopSignals::next()` (cfg windows) adds a select arm that waits for the watch to be `true` and returns `"service stop request"`. Being a latch, it is delivered once. → PASS.
- [ ] **Step 4:** Implement `run_as_service(parsed, binary)` (cfg windows):
  - Stash `(parsed, binary)` in a `OnceLock`.
  - Call `service_dispatcher::start(service_name(role), ffi_service_main)`. On `Err`, `eprintln!` the message and exit with `dispatcher_error_code(raw_os_error)`.
  - `service_main` registers the control handler, accepting `STOP | SHUTDOWN | PRESHUTDOWN`. Those controls set the `StopLatch` and `scm_stop()`, and report StopPending via `PendingReporter`. `Interrogate` → `NoError`. The default preshutdown timeout is left untouched.
  - It reports StartPending, and a ticker thread advances the checkpoint every 1 s until Running, then again during StopPending.
  - It initialises tracing with `init_tracing_with(.., CappedLog::open(machine_log_dir()?, role, 10 MiB))`, falling back to `std::io::sink`.
  - It calls `run_to_code(parsed, binary, Some(Box::new(|| report Running)))` inside `catch_unwind`; a panic gives code 1.
  - It reports `Stopped` exactly once, with `exit_to_scm(code, latch.was_set())`.

  In `run_to_code`'s `Run` arm, `on_ready` calls `print_ready` and then the hook. The engine already calls `on_ready` only after the claim and the config load (`skeleton.rs:97,121`). In `execute`, `if std::env::var_os(ENV_SCM).as_deref() == Some("1".as_ref()) { #[cfg(windows)] scm_runtime::run_as_service(parsed, binary) }` runs before `init_tracing`; on other OSes it is ignored.
- [ ] **Step 5:** Test seams (test-utils): `test_fault()` reads `IROH_TUNNEL_TEST_EXIT_AFTER_MS` (spawns a task that makes the run return code 1 after N ms) and `IROH_TUNNEL_TEST_FAIL_BEFORE_READY=1` (returns 1 before `on_ready`). Both are applied in `run_to_code`'s `Run` arm.
- [ ] **Step 6:** Local gates are green. Commit (`feat(runtime): run roles under the Windows Service Control Manager`, with the `Co-authored-by: 23rd` trailer).

---

## Task 11: Cross-scope refusal and update guidance — implements 8.1, 8.2

**Files:** Modify `crates/host/src/service/install.rs` (`cross_scope_conflict` and its call), `backend/windows.rs` (expose the existing locale-independent `is_installed` as `pub(crate) fn task_registered(native, role) -> Result<bool>`), `windows_scm.rs` (`service_registered(port, role)`), `crates/host/src/update/replace.rs`, and `tests/update_zip_windows.rs`.

**Interfaces:** Produces `cross_scope_conflict` and `windows_locked_hint`.

- [ ] **Step 1:** Failing pure tests:
  - `system_install_refused_when_user_task_exists`: contains `iroh-tunnel serve service uninstall --user`; class `Service`.
  - `user_install_refused_when_system_service_exists`.
  - `no_conflict_when_other_absent`.
  - `windows_locked_hint_mentions_service_stop_system` (all platforms, on the pure `windows_locked_hint()`).
  - `unprivileged_system_install_still_exits_3_before_cross_scope`: through `plan_install` with injected probes; `ensure_privilege` fires first.
- [ ] **Step 2:** → FAIL. Implement. `plan_install` on Windows calls `cross_scope_conflict` after `ensure_privilege` and before any trust or registration work. `--system` probes `task_registered`; a listing failure is a `CliError::Service`, never "absent". `--user` probes `service_registered` (`SC_MANAGER_CONNECT` + `SERVICE_QUERY_STATUS`; 1060 means absent; any other error is a service error). Append `windows_locked_hint()` to the Windows locked-executable error in `replace.rs`. → PASS.
- [ ] **Step 3:** Extend the `tests/update_zip_windows.rs` locked module so it asserts that stderr contains `service stop --system` and that the locking process is still alive.
- [ ] **Step 4:** Local gates are green. Commit (`feat(service): refuse cross-scope installs on Windows; update hints at service stop`).

---

## Task 12: Installer directory protection — implements 9.1, 9.2

**Files:** Modify `install.ps1` (only if Task 1 says it is needed). Create `tests/installer_acl_windows.rs` (cfg windows, e2e-gated). Modify `crates/host/src/service/mod.rs` (`__test_verify_binary`).

**Interfaces:** Consumes `__test_verify_binary` and `tests/common/release_server.rs::ReleaseServer` (HTTP).

- [ ] **Step 1:** If the Task 1 verdict is "not needed", record that in `design.md` D9, leave `install.ps1` unchanged, and continue with Step 3 to pin the behaviour.
- [ ] **Step 2:** Otherwise add `function New-ProtectedBinDir([string]$Dir)`. It is called only when `-not (Test-Path -LiteralPath $Dir)`, for the default and for `-BinDir` alike:
  1. `New-Item -ItemType Directory`.
  2. Build a `DirectorySecurity` with `SetAccessRuleProtection($true, $false)`.
  3. Copy the parent's rules, except those whose SID is the current user SID or `S-1-3-0`.
  4. `SetOwner([Security.Principal.SecurityIdentifier]'S-1-5-32-544')`.
  5. `Set-Acl`.

  `Invoke-ScriptAnalyzer install.ps1` reports no findings.
- [ ] **Step 3:** Test `installer_creates_a_directory_that_passes_the_binary_check`:
  - Start `ReleaseServer` with the zip and checksum fixtures that `update_zip_windows.rs` builds.
  - Run `pwsh -NoProfile -File install.ps1 -Yes -BinDir "$env:ProgramFiles\iroh-tunnel-test-<pid>"` with `IROH_TUNNEL_BASE_URL=http://127.0.0.1:<port>` and `IROH_TUNNEL_PATH_FILE=<tmp>\path.txt`, so the machine PATH is not touched.
  - Assert `__test_verify_binary(<dir>\iroh-tunnel.exe)` → `Ok`.
  - A `Drop` guard removes the dir.
- [ ] **Step 4:** Test `installer_leaves_an_existing_directory_untouched`: pre-create the dir and record `(Get-Acl).Sddl`; after the install it is identical.
- [ ] **Step 5:** Commit (`feat(install): protect the install directory the Windows installer creates`).

---

## Task 13: Windows system-service E2E — implements 10.1, 10.2, 10.3

**Files:** Create `tests/service_system_windows.rs` and `tests/common/restricted.rs`. Modify `.github/workflows/e2e.yml` (add the new step, remove the Task 1 probe step) and `tests/service_user_windows.rs` (un-ignore the Task 9 case).

**Interfaces:**
- **CLI.** The tests drive the CLI the way `tests/service_user_windows.rs` does, with suffix `IROH_TUNNEL_TEST_SERVICE_SUFFIX=-e2e-<pid>`.
- **`Drop` guards.** They uninstall, wait for the lock to release, and delete any test user.
- **Prompt answers.** Fed through stdin, as `run_with_stdin` does in `tests/service_system.rs:144`.
- **Service environment.** The helper `set_service_env(name, &[(k, v)])` edits the service's `Environment` registry value for the test seams.
- **Gating.** Every test returns early unless `IROH_TUNNEL_E2E_SERVICE=1`, printing `skipped: …`, because the default e2e step runs `--test '*'` without the env.

- [ ] **Step 1 (commit A, restricted-token helper):** `restricted::run_non_elevated(cmd) -> Result<Output>`:
  1. Duplicate the process token and call `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE, sids_to_disable = [Administrators])`.
  2. `SetTokenInformation(TokenIntegrityLevel, S-1-16-8192)` (Medium).
  3. Run `CreateProcessAsUserW` with redirected pipes.

  Self-test `restricted_child_is_not_elevated`: the child runs `whoami /groups /fo csv`, and its output must contain `S-1-16-8192` and must not contain `S-1-16-12288`. Only SIDs are read. A failure is a hard test failure, never a skip. Un-ignore `non_elevated_system_scope_is_permission_error` in `service_user_windows.rs`. Commit.
- [ ] **Step 2 (commit B, lifecycle and stop):**

  | Test | Assertions | Scenarios |
  |---|---|---|
  | `system_service_lifecycle` | Install → `status --system` reports running and the status file exists. `stop --system` → stopped, status file removed. `start`, then `restart` → same node id. The registered executable read back with `query_config` equals the install binary, quoted correctly. `uninstall` → `status --system` exits 5. | Service definition; Reinstall; Stop removes the status file; System service restart; Windows scopes; Pin in the Windows service; Elevated user installs with defaults; Service stop request |
  | `stop_pending_reported_while_draining` | Open a TCP connection through the tunnel, then run `stop --system` in a thread. Poll `QueryServiceStatusEx` and observe StopPending with `dwWaitHint ≥ 13000`. | Stop pending while draining |
  | `elevated_without_flag_targets_system` | Install with no flag → the service exists and no task exists. `stop` with no flag stops the service. | Elevated Windows terminal installs without a scope prompt; Elevated Windows terminal without a flag; Windows system scope uses only a Windows Service |
  | `missing_service_verbs` | | Not installed; Idempotent stop and uninstall |
  | `scm_marker_outside_scm_is_config_error` | `serve run` with `IROH_TUNNEL_SCM=1` → exit 2 | Service marker outside the SCM |
  | `crash_is_restarted` | `set_service_env(IROH_TUNNEL_TEST_EXIT_AFTER_MS=500)` and restart → the PID changes within 10 s | Crash recovery |
  | `rollback_when_start_fails` | Install with the env pre-seeded through a test-utils install hook (`IROH_TUNNEL_TEST_FAIL_BEFORE_READY` added to `ServiceSpec.env` only under test-utils) → install fails and the service is absent | Start failure rolls back |

  Commit.
- [ ] **Step 3 (commit C, trust refusals):**

  | Test | Assertions | Scenarios |
  |---|---|---|
  | `unsafe_config_refused_then_fixed` | Grant Everyone modify on the config, then `restart --system` → never Running, and `logs\serve.log` contains `permission error` and the path. Remove the grant and `start` → Running; the config's SDDL equals the user's own. | Unsafe config at service start; Log available after a refusal; Fixed condition; Everyone may modify the config |
  | `junction_in_config_path_refused` | | Junction in the config path |
  | `invalid_config_never_runs` | | Invalid config at start |
  | `empty_key_never_written` | | Config without a key |
  | `pinless_service_refused` | `set_service_env` without `IROH_TUNNEL_TRUSTED_SID` | Service without a pin |
  | `untrusted_machine_dir_refused` | Uses the test-utils machine dir override. | Directory pre-created by a user; Directory made untrusted after install |
  | `binary_check_at_install_and_start` | | Unsafe binary at install; Unsafe binary at CLI start; Binary writable by the pinned user |

  Commit.
- [ ] **Step 4 (commit D, identity, scope and concurrency):**

  | Test | Assertions | Scenarios |
  |---|---|---|
  | `config_owned_by_other_user` | `net user itbob<pid> <generated 24-char password with upper/lower/digit/symbol> /add`, then `icacls <dir> /setowner itbob<pid>`; install with that `--config` → exit 2 naming both accounts. The guard runs `net user /delete`. | Config owned by another user |
  | `retarget_declined_and_approved` | Install as the runner user. Rewrite the service `Description` pin to bob's SID through the registry. Reinstall with stdin `n` → exit 5 with `Retarget`, and the description is unchanged. Reinstall with stdin `y` → the pin is the runner SID again. A further reinstall has no prompt. | Declined/Approved/Same user again on Windows |
  | `config_dir_owned_by_administrators` | | Config directory owned by Administrators |
  | `cross_scope_refused_both_ways` | | User task exists; System service exists |
  | `user_task_blocked_while_service_runs` | `restricted::run_non_elevated(serve run)` → exit 6 and `system service` | Windows user task while the system service runs; System lock held, user start |
  | `already_running_is_clean_stop` | A restricted foreground `serve run` holds the machine lock shared. Start the service through `sc.exe start <name>`, by exit status only, which bypasses the CLI's `ensure_not_running` precheck. The service ends Stopped with `dwWin32ExitCode == 0` and the log says `already running`; no restart follows within 8 s. | Role already running; User running, system start |
  | `non_elevated_system_operations` | Through `restricted`, install and every verb with `--system` → exit 3, `Run as administrator`, the argv, no service. | Non-elevated system install/verbs on Windows; frontend-independent-management Non-elevated Windows system scope; Windows system scope without elevation |

  Commit.
- [ ] **Step 5:** In `e2e.yml`, add a Windows step after "Service lifecycle (Task Scheduler)":
  - `name: Service lifecycle (Windows Service)`
  - `shell: pwsh`
  - env `IROH_TUNNEL_E2E_SERVICE: "1"`
  - `cargo test --all-features --locked --test service_system_windows --test installer_acl_windows -- --nocapture --test-threads=1`

  Remove the Task 1 probe step; `actionlint` is clean. Run `gh workflow run e2e.yml --ref feat/windows-system-service` and record the run URL. Without push approval, record "pending approval". Commit (`ci(e2e): run the Windows Service lifecycle`).

---

## Task 14: Docs, CHANGELOG and final gates — implements 11.1, 11.2, 11.3

**Files:** Modify `docs/services.md`, `docs/troubleshooting.md`, `CHANGELOG.md`, and the module doc in `crates/host/src/service/mod.rs`, which today says user scope only.

- [ ] **Step 1:** In `docs/services.md`, update the scope table so the Windows system row reads "Windows Service (LocalSystem)". In the Windows section, cover:
  - both scopes and the elevated default;
  - the trust rules table, and how to fix a refusal (`icacls <path> /remove *S-1-1-0`);
  - `%ProgramData%\iroh-tunnel`: lock, `logs\<role>.log`, kept on uninstall, and how to delete it;
  - the boot/recovery binary-check gap;
  - the cross-scope refusal;
  - stop drains for `--system`, but is a hard end for `--user` (#185).
- [ ] **Step 2:** In `docs/troubleshooting.md`, replace "Windows has no system services (exit code 5)" with "exit 3 on Windows: open an elevated terminal (Run as administrator)". Add entries for `reparse point` and for `S-1-1-0` refusals.
- [ ] **Step 3:** In `CHANGELOG.md` Unreleased:
  - **Added:** Windows system services (#184; thanks @23rd for #67).
  - **Changed (breaking):** an elevated Windows terminal without `--user`/`--system` now targets system scope for every `service` verb; non-elevated `--system` exits 3 (was 5).
  - **Changed:** the Windows locked-executable update error mentions `service stop --system`.
- [ ] **Step 4:** Gates:
  - `openspec validate windows-system-service --strict`
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
  - `cargo test --workspace --all-features --locked`
  - the Windows CI unit and e2e cells are green, or "pending approval" is recorded.

  Commit (`docs: Windows system services`).

---

## Scenario → test

**windows-service-trust**
- Config in the pinned user's profile → `model::trusted_profile_chain_passes`; `win32::profile_temp_dir_passes_for_current_user`; E2E `system_service_lifecycle`
- Binary writable by the pinned user → `model::binary_set_excludes_pinned_user`; E2E `binary_check_at_install_and_start`
- Junction in the config path → `model::reparse_component_is_refused`; `win32::junction_in_chain_is_refused`; E2E `junction_in_config_path_refused`
- Untrusted owner → `model::untrusted_owner_is_refused`
- NULL DACL → `model::null_dacl_is_refused`; `win32::null_dacl_is_refused`
- Inherit-only entry → `model::inherit_only_full_control_is_ignored`
- Everyone may modify the config → `model::each_file_write_right_is_refused`; `win32::everyone_modify_on_config_is_refused`; E2E `unsafe_config_refused_then_fixed`
- Create-subdirectory on the volume root → `model::create_rights_on_ancestor_are_accepted`
- Delete-child on an ancestor → `model::each_ancestor_replace_right_is_refused`
- Unsafe config at service start → `platform::writable_config_is_permission_error`; E2E `unsafe_config_refused_then_fixed`
- Unsafe binary at CLI start → `verb::verb_start_system_checks_binary_first`; E2E `binary_check_at_install_and_start`
- Unsafe binary at install → E2E `binary_check_at_install_and_start`
- Planted reparse point at the snapshot name → `win32::publish_replaces_a_planted_symlink`; `writer::status_publish_goes_through_verified_dir`
- Fixed condition → E2E `unsafe_config_refused_then_fixed`

**system-service-management**
- Service definition → `windows_scm::service_spec_for_serve`; E2E `system_service_lifecycle`
- Reinstall → `windows_scm::install_replaces_existing`; E2E `system_service_lifecycle`
- Start failure rolls back → `windows_scm::install_rolls_back_when_start_fails`, `install_rolls_back_when_never_running`; E2E `rollback_when_start_fails`
- Role already running → `scm_runtime::exit_to_scm_maps_codes`; E2E `already_running_is_clean_stop`
- Crash recovery → `windows_scm::failure_plan_from_default_policy`; E2E `crash_is_restarted`
- Stop removes the status file → E2E `system_service_lifecycle`
- Stop pending while draining → `scm_runtime::stop_pending_reports_15s_hint_and_increments_checkpoint`; E2E `stop_pending_reported_while_draining`
- Invalid config at start → `platform::invalid_toml_reports_failed_to_parse`; E2E `invalid_config_never_runs`
- Service marker outside the SCM → `scm_runtime::dispatcher_error_1063_is_config_error`; E2E `scm_marker_outside_scm_is_config_error`
- Service without a pin → `platform::service_without_sid_is_refused`; E2E `pinless_service_refused`
- Config without a key → `platform::empty_secret_key_under_service_is_config_error_and_file_unchanged`; E2E `empty_key_never_written`
- Not installed → `windows_scm::start_absent_is_service_error_with_install_hint`; E2E `missing_service_verbs`
- Idempotent stop and uninstall → `windows_scm::stop_of_stopped_is_ok`, `uninstall_absent_is_ok`; E2E `missing_service_verbs`
- Declined retarget on Windows → E2E `retarget_declined_and_approved`
- Approved retarget on Windows → E2E `retarget_declined_and_approved`
- Same user again on Windows → `pin::pin_compares_paths_case_insensitively_on_windows`; E2E `retarget_declined_and_approved`
- Elevated user installs with defaults → `system_windows::defaults_use_token_user_paths`; E2E `system_service_lifecycle`
- Config owned by another user → `system_windows::config_owned_by_other_user_is_config_error`; E2E `config_owned_by_other_user`
- Config directory owned by Administrators → `system_windows::config_dir_owned_by_administrators_defers_to_token`; E2E `config_dir_owned_by_administrators`
- User task exists → `install::system_install_refused_when_user_task_exists`; E2E `cross_scope_refused_both_ways`
- System service exists → `install::user_install_refused_when_system_service_exists`; E2E `cross_scope_refused_both_ways`
- Directory pre-created by a user → `ownership::untrusted_existing_machine_dir_is_refused`; E2E `untrusted_machine_dir_refused`
- Directory made untrusted after install → E2E `untrusted_machine_dir_refused`
- Log available after a refusal → `log_file::writes_append_to_role_log`; E2E `unsafe_config_refused_then_fixed`
- Task execution policy → existing `task_xml` tests (unchanged)
- Task definition → existing `task_xml` tests (unchanged)
- Conflicting explicit scope flags → existing CLI parse test (unchanged)
- Root installs without a scope prompt → existing `decide_install_scope` tests (unchanged)
- Elevated Windows terminal installs without a scope prompt → `install::windows_elevated_install_without_flag_plans_system`; E2E `elevated_without_flag_targets_system`
- Interactive user is offered both scopes → existing `decide_install_scope::install_scope_needs_choice_when_caller_can_answer` (now reachable on Windows after Task 9)
- Non-interactive non-root default → the same existing test
- Unprivileged system install → existing `tests/service_user.rs` (unchanged); `policy::unix_privilege_text_is_unchanged`
- Unprivileged system verbs → existing `tests/service_user.rs` (unchanged)
- Non-elevated system install on Windows → `policy::windows_privilege_text_names_run_as_administrator`; `service_user_windows::non_elevated_system_scope_is_permission_error`; E2E `non_elevated_system_operations`
- Non-elevated system verbs on Windows → `start/stop/restart/uninstall` unit tests (Task 9); E2E `non_elevated_system_operations`
- Root without a flag → existing `default_scope::default_scope_follows_privilege`
- Non-root without a flag → the same
- Elevated Windows terminal without a flag → `default_scope_follows_privilege` + Task 2 Windows `is_elevated` test; E2E `elevated_without_flag_targets_system`
- Container without systemd → existing `tests/service_system.rs` (unchanged)
- Windows scopes → E2E `system_service_lifecycle`, `cross_scope_refused_both_ways`
- Pin in the script → existing `tests/service_system.rs` BusyBox pin case; `pin::legacy_uid_record_still_parses`
- Pin in the Windows service → `pin::sid_record_round_trips`; `windows_scm::service_spec_for_serve`; E2E `system_service_lifecycle`
- Custom state directory → existing `service_user_windows::install_refuses_a_state_directory_the_task_cannot_carry`
- Slow stop → existing task-backend restart (`backend/windows.rs:271`, unchanged)
- System service restart → `windows_scm::restart_waits_for_lock_then_starts`, `restart_times_out_without_starting`; E2E `system_service_lifecycle`

**role-ownership-shutdown**
- Machine lock location → `ownership::windows_machine_dir_is_programdata_child`; existing Unix location tests
- System lock held, user start → `ownership::machine_lock_exclusive_then_shared_is_refused`; E2E `user_task_blocked_while_service_runs`
- User running, system start → `ownership::machine_lock_exclusive_then_shared_is_refused`; E2E `already_running_is_clean_stop`
- Windows user task while the system service runs → E2E `user_task_blocked_while_service_runs`
- SIGTERM → existing `tests/role_ownership.rs` SIGTERM test (unchanged)
- Service stop request → `lifecycle::scm_stop_triggers_the_same_drain`; E2E `system_service_lifecycle`

**frontend-independent-management**
- Windows system scope without elevation → E2E `non_elevated_system_operations`
- Windows system scope uses only a Windows Service → E2E `elevated_without_flag_targets_system`
- Unprivileged system scope → existing host-operation test (unchanged)
- Non-elevated Windows system scope → `policy::windows_privilege_text_names_run_as_administrator`; E2E `non_elevated_system_operations`
- BusyBox user scope unchanged → existing test (unchanged)

**install-release-packaging**
- Fresh default install → `installer_acl_windows::installer_creates_a_directory_that_passes_the_binary_check`
- Existing directory untouched → `installer_acl_windows::installer_leaves_an_existing_directory_untouched`

**self-update**
- Running image → existing `update_zip_windows` locked case (unchanged)
- Guidance for a system service → `replace::windows_locked_hint_mentions_service_stop_system`; the extended `update_zip_windows` assertion
