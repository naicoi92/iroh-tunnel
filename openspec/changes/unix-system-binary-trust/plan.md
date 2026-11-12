# Unix System Service Binary Trust Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` (`unix-service-binary-trust`, `system-service-management` delta) and `design.md`.

**Goal:** A root-run Linux/macOS service never runs a binary a non-root principal can change: installs register a root-only-writable binary or a managed root-owned copy, CLI start/restart refuse an unsafe effective binary, and uninstall releases the copy.

**Architecture:** `trust.rs` gains an explicit owner set, plus handle-based `ensure_dir`, `publish` and `remove_entry`. A portable `effective_binary.rs` parses what the service manager will run; the backends query it. `service_binary.rs` holds the domain decision (`ServiceBinary`) and the copy lifecycle. `install.rs` plans the binary without effects and writes it first at execution. `verb.rs` checks before any lifecycle command and releases the copy after uninstall.

**Tech Stack:** Rust 2021 (MSRV 1.91), `libc` (`openat`, `mkdirat`, `fchmod`, `renameat`, `unlinkat`), `anyhow` + `CliError`, `tempfile` in tests. No new dependency.

## Global Constraints

- Behaviour applies on Linux and macOS only; other targets plan `Direct(resolved)`. Windows code and behaviour are unchanged and must still compile.
- Trusted principal for the binary: root only. The pinned uid is never trusted for it. Production call sites pass `TrustedOwners::RootOnly`.
- Managed copy: `/var/lib/iroh-tunnel/bin/<role>/iroh-tunnel` (Linux), `/Library/Application Support/iroh-tunnel/bin/<role>/iroh-tunnel` (macOS). File mode is exactly `0755` and new directories are `0755`, whatever the umask. No pathname `chmod`/`chown`, ever.
- `IROH_TUNNEL_TEST_MANAGED_BIN_DIR` is honoured only with `test-utils`, and the fixed suffix `iroh-tunnel/bin/<role>/iroh-tunnel` is always appended to it.
- Refusals are `CliError::Permission` (exit 3), naming path, owner uid and mode where known. At start/restart they add `sudo iroh-tunnel <role> service install --system`.
- Only `service install` writes a copy. Start and restart never do; on refusal they issue no start, stop or restart command.
- RS-01–RS-16; `// SAFETY:` on every `unsafe` block; `~/.agents/skills/coding-standards/SKILL.md`. Repo content in English.

## Review Focus

1. Merged-usr and Homebrew symlinks (`/bin -> usr/bin`, `/opt/homebrew/bin/iroh-tunnel -> ../Caskroom/…`) must be canonicalized before the walk. Pinned in Task 3 `choose_resolves_a_link_before_verifying`.
2. A path with a space (`/Library/Application Support/…`) must stay one argument in systemd `ExecStart`, the launchd plist and the BusyBox `DAEMON`, and must parse back. Pinned in Task 2 (parser fixtures with spaces) and Task 4 `managed_path_renders_as_one_argument_in_each_backend`.
3. Umask `077` must not change the copy mode or the mode of new directories. Pinned in Task 1 `publish_and_ensure_dir_ignore_umask` (child process).
4. A not-loaded unit or job must fall through to today's error, and an unparseable loaded one must be refused. Pinned in Task 2 (`NotLoaded` vs `Unknown` fixtures) and Task 5.
5. Re-running uninstall when the copy, the role directory or the link target is already gone must succeed. Pinned in Task 1 `remove_entry_absent_and_dangling_link`.

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `trust::TrustedOwners` | new | `pub(crate) enum TrustedOwners { RootOnly, RootAnd(u32) }` with `fn trusts(self, owner: u32) -> bool` | no |
| `trust::verify_binary` | new | `pub(crate) fn verify_binary(path: &Path, owners: TrustedOwners) -> anyhow::Result<()>` | no |
| `trust::ensure_dir` | new | `pub(crate) fn ensure_dir(path: &Path, owners: TrustedOwners, mode: u32) -> anyhow::Result<()>` | no |
| `trust::publish` | modified | `pub(crate) fn publish(dir: &Path, owners: TrustedOwners, name: &str, temp: &str, source: &mut dyn std::io::Read, mode: u32) -> anyhow::Result<()>`; status writer passes `RootAnd(uid)`, `&mut bytes`, `0o644` | no |
| `trust::remove_entry` | new | `pub(crate) fn remove_entry(dir: &Path, owners: TrustedOwners, name: &str) -> anyhow::Result<bool>` (true = removed; also `rmdir`s `dir` if then empty) | no |
| `trust::remove_in`, `trust::read_in` | modified | take `TrustedOwners` instead of `u32` | no |
| `effective_binary::EffectiveBinary` | new | `pub(crate) enum EffectiveBinary { NotLoaded, Binary(PathBuf), Unknown(String) }` | no |
| `effective_binary::parse_systemd_show` | new | `pub(crate) fn parse_systemd_show(stdout: &str) -> EffectiveBinary` (input: `LoadState=…\nExecStart=…`) | no |
| `effective_binary::parse_launchctl_print` | new | `pub(crate) fn parse_launchctl_print(success: bool, stdout: &str, stderr: &str) -> EffectiveBinary` | no |
| `effective_binary::parse_busybox_script` | new | `pub(crate) fn parse_busybox_script(script: Option<&str>) -> EffectiveBinary` | no |
| backend `effective_binary` | new | `systemd::effective_binary(native: &mut Native, role: &str) -> EffectiveBinary`; same for `launchd`; `busybox::effective_binary(role: &str) -> EffectiveBinary` | no |
| `platform::effective_binary` | new | `pub(crate) fn effective_binary(native: &mut Native, role: &str) -> EffectiveBinary` (root-run backend of this host) | no |
| `platform::root_run` | new | `pub(crate) fn root_run(scope: ServiceScope, system_wide_only: bool) -> bool` | no |
| `platform::RegistrationSources` | new | `pub(crate) struct RegistrationSources { pub files: Vec<PathBuf>, pub drop_in_dirs: Vec<PathBuf> }` + `fn for_role(role: &str) -> Self` | no |
| `platform::managed_copy_referenced` | new | `pub(crate) fn managed_copy_referenced(native: &mut Native, role: &str, sources: &RegistrationSources, needle: &str) -> bool` (true on any match or any read/query failure) | no |
| `RunningExecutable` | new | `pub struct RunningExecutable { pub display: PathBuf, pub read_from: PathBuf }` (`crates/host/src/service/types.rs`) | yes (crate API) |
| `service_binary::ServiceBinary` | new | `pub(crate) enum ServiceBinary { Direct(PathBuf), Managed { source: RunningExecutable, copy: PathBuf } }` + `fn path(&self) -> &Path` | no |
| `service_binary::managed_copy_path` | new | `pub(crate) fn managed_copy_path(role: &str) -> PathBuf`; `pub(crate) fn managed_needle(role: &str) -> String` = `"iroh-tunnel/bin/<role>/iroh-tunnel"` | no |
| `service_binary::choose` | new | `pub(crate) fn choose(resolved: &Path, running: &RunningExecutable, copy: PathBuf, verify: &dyn Fn(&Path) -> anyhow::Result<()>) -> anyhow::Result<ServiceBinary>` | no |
| `service_binary::materialize` | new | `pub(crate) fn materialize(binary: &ServiceBinary, owners: TrustedOwners) -> anyhow::Result<Option<AppliedEffect>>` | no |
| `service_binary::release` | new | `pub(crate) fn release(copy: &Path, owners: TrustedOwners, referenced: impl FnOnce() -> bool) -> anyhow::Result<Option<AppliedEffect>>` | no |
| `AppliedEffect::ManagedBinaryWritten(PathBuf)`, `::ManagedBinaryRemoved(PathBuf)` | new | variants of the existing public enum | yes (crate API) |
| `ServiceNote::ManagedBinary { role: String, source: PathBuf, copy: PathBuf }` | new | rendered on stdout as ``Registered a root-owned copy of <source> at <copy> for the <role> service; re-run `sudo iroh-tunnel <role> service install --system` after upgrading to update it\n`` | yes (crate API, CLI output) |
| `InstallRequest.running_executable` | new | `pub running_executable: RunningExecutable` | yes (crate API) |
| `install::plan_binary` | new | `fn plan_binary(req: &InstallRequest, scope: ServiceScope, system_wide_only: bool, verify: &dyn Fn(&Path) -> Result<()>) -> Result<ServiceBinary>` | no |
| `install::install_steps` | new | `fn install_steps(native: &mut Native, notes: &mut dyn FnMut(&ServiceNote), role: &str, binary: &ServiceBinary, materialize: impl FnOnce(&ServiceBinary) -> Result<Option<AppliedEffect>>, register: impl FnOnce(&mut Native, &mut dyn FnMut(&ServiceNote), &Path) -> Result<()>, release_after_direct: impl FnOnce(&mut Native) -> Result<Option<AppliedEffect>>) -> Result<()>`; `release_after_direct(native)` runs only for `Direct`, after `register` succeeds | no |
| `IROH_TUNNEL_TEST_MANAGED_BIN_DIR` | new | env var, `test-utils` only: a root under which the fixed suffix `iroh-tunnel/bin/<role>/iroh-tunnel` is appended, so every managed path contains `managed_needle(role)` | no |
| `install::existing_registration` seam | new | `plan_system_install` reads the existing registration through `existing: &dyn Fn(&str) -> Option<String>` (production: `platform::existing_registration(role, System)`) | no |
| `verb::verb_steps` | new | `fn verb_steps(native: &mut Native, notes: &mut dyn FnMut(&ServiceNote), verb: Verb, guard: impl FnOnce(&mut Native) -> Result<()>, backend: impl FnOnce(&mut Native, &mut dyn FnMut(&ServiceNote)) -> Result<()>, release_after_uninstall: impl FnOnce(&mut Native) -> Result<Option<AppliedEffect>>) -> Result<()>`: `guard` runs before `backend` for Start/Restart; `release_after_uninstall` runs after a successful Uninstall backend. Deviation (Task 5): the planned `role: &str` parameter was dropped because the closures capture the role and the orchestration never reads it | no |
| `trust::check_entry` | new | `pub(crate) fn check_entry(owner: u32, mode: u32, kind: EntryKind, owners: TrustedOwners) -> Result<(), String>` pure metadata predicate used by `check_parent`/`check_final`. Deviation (Task 1): `EntryKind` became `pub(crate)` because the predicate takes it | no |

**Contract deviations recorded during apply (all internal):**

- `install::existing_registration` seam: implemented as `plan_system_install_with(req, env, lock_dirs, existing, verify)` behind the `plan_system_install` wrapper, which also takes the `verify` seam so tests avoid the real root-only walk.
- `service_binary::release_unreferenced_copy(native, role, root_run) -> Result<Option<AppliedEffect>>` (new, not in the table): the one cleanup policy shared by install (`Direct`) and uninstall, moved out of `install.rs` in Task 5.
- `verb::check_effective(loaded, role, verify)` (new, private): pure refusal formatting behind `guard_effective_binary`; prefixes the registered binary path when the trust message does not already name it.
- `install::plan_user_install(req, env, probe, lock_dirs, system_wide_only)` (new, private) and `policy::ensure_root_run_privilege(root_run, elevated)` (new): the final review found an unprivileged BusyBox `--user` install reaching `materialize` (exit 1); the privilege check now runs during planning with the existing BusyBox text.
- `Native::output` is compiled on Linux too (previously macOS, Windows and tests only), for the systemd `effective_binary` query.
- `launchd` private helpers `UnloadPoll`/`UNLOAD_POLL`, `bootout_job`, `wait_until_unloaded` and `print_definition` (new, task 7.5 fix): after a successful `bootout`, uninstall polls `launchctl print` (up to 50 probes, 100 ms apart) until launchd drops the job, so the release step does not see a `SIGTERMed` job still naming the copy. Timeout or an unexpected probe failure is not an error; the conservative reference check then keeps the copy. `uninstall` keeps its signature.
- `platform::effective_binary`, `RegistrationSources` and `managed_copy_referenced` exist only on Linux and macOS, as Global Constraints scope the behaviour.

---

## Task 1: Root-only trust mode — implements 1

**Covers:** 1.1, 1.2, 1.3, 1.4
**Difficulty:** Hard
**Blocked by:** none

**Files:**
- Modify: `crates/host/src/service/trust.rs`, `crates/host/src/status/writer.rs`

**Interfaces:**
- Produces: `TrustedOwners`, `verify_binary`, `ensure_dir`, `publish`, `remove_entry`, `remove_in`, `read_in` as in Contracts.

- [ ] **Step 1: Write failing tests** in `trust.rs` `mod tests` (`me()` is the test uid; `other = me() + 1`):
  - `root_only_accepts_a_system_binary`: `verify_binary(&canonicalize("/bin/sh"), RootOnly)` is `Ok`.
  - `root_only_refuses_a_file_owned_by_the_test_user` (skipped when `me() == 0`): a fixture file → `Permission` naming the path.
  - `binary_refuses_untrusted_parent_naming_it`: `verify_binary(file, RootAnd(other))` → the message names a fixture directory and its mode.
  - `binary_refuses_0775_naming_the_mode`, `binary_refuses_a_symlink_and_a_directory`.
  - `binary_sticky_parent_trusted_child_passes` and `binary_sticky_parent_untrusted_child_fails` (a 1777 dir with `RootAnd(me())` vs `RootAnd(other)`).
  - `check_entry_refuses_other_owner`: pure predicate, owner `other`, mode `0o040755`, `RootAnd(me())` → `Err` naming `other`; `RootOnly` with owner `0` → `Ok`.
  - `ensure_dir_creates_missing_components_0755` and `ensure_dir_refuses_existing_untrusted_and_leaves_mode`: an existing 0777 dir is refused, its mode is still 0777, and no child was created.
  - `ensure_dir_never_follows_a_planted_link`: a symlink at a missing component's name → refused, link target unchanged.
  - `publish_and_ensure_dir_ignore_umask` (through `fixtures::in_isolated_home` child, `libc::umask(0o077)` inside): file `0755`, new dir `0755`.
  - `publish_failure_before_rename_keeps_old_file_and_no_temp`: a `Read` that errors midway; the old content stays and no temp file remains.
  - `remove_entry_absent_and_dangling_link`: a missing dir → `Ok(false)`; a dangling link at the name → unlinked, `Ok(true)`, empty dir removed; a live link → link removed, target intact.
- [ ] **Step 2:** `cargo test -p iroh-tunnel-host --all-features --locked service::trust` → compile FAIL.
- [ ] **Step 3:** Implement. Thread `TrustedOwners` through `walk`/`check_parent`/`check_final`. `ensure_dir` walks existing components, then per missing name does `mkdirat(fd, name, mode)` → `open_child(dir)` → `fchmod(fd, mode)` → `Entry::new` + `check_parent`. On `EEXIST` it opens and verifies only. `publish` streams with `std::io::copy`, then `fchmod`, `sync_all`, `renameat`, and runs `unlinkat` of the temp on every error before the rename. `remove_entry` uses `symlink_metadata(dir)` for absence, `walk(dir, owners, true)`, `unlinkat(fd, name, 0)` (`ENOENT` → false), then `unlinkat(parentfd, dirname, AT_REMOVEDIR)` ignoring `ENOTEMPTY`. Update the `writer.rs` call sites.
- [ ] **Step 4:** Tests PASS; `cargo clippy -p iroh-tunnel-host --all-targets --all-features --locked -- -D warnings` clean.
- [ ] **Step 5:** Commit `feat(service): root-only trust mode and handle-based dir creation`.

---

## Task 2: Effective binary and reference queries — implements 2

**Covers:** 2.1, 2.2, 2.3
**Difficulty:** Normal
**Blocked by:** none

**Files:**
- Create: `crates/host/src/service/effective_binary.rs` (portable, no `cfg`)
- Fixtures: inline `const` strings in the test module, each with a comment giving its source and version
- Modify: `crates/host/src/service/mod.rs` (`mod effective_binary;`), `crates/host/src/service/backend/systemd.rs`, `crates/host/src/service/backend/launchd.rs`, `crates/host/src/service/backend/busybox.rs`, `crates/host/src/service/platform.rs`

**Interfaces:**
- Produces: `EffectiveBinary` and parsers, backend `effective_binary`, `platform::{effective_binary, root_run, RegistrationSources, managed_copy_referenced}` as in Contracts.

- [ ] **Step 1: Write failing parser tests** in `effective_binary.rs`:
  - systemd: `"LoadState=loaded\nExecStart={ path=/usr/local/bin/iroh-tunnel ; argv[]=/usr/local/bin/iroh-tunnel serve run --config /c ; ignore_errors=no ; start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; code=(null) ; status=0/0 }\n"` → `Binary("/usr/local/bin/iroh-tunnel")`. A `path=` containing `Application Support` → that path with the space. `LoadState=not-found\nExecStart=\n` → `NotLoaded`. `LoadState=loaded\nExecStart=\n` → `Unknown`. Two `{ path=… }` blocks → `Unknown`.
  - launchd: success with a block containing `\tprogram = /Library/PrivilegedHelperTools/x` → `Binary`. No `program` line but `arguments = {\n\t\t/opt/x\n` → `Binary("/opt/x")`. `success=false`, stderr `Could not find service "dev.iroh-tunnel.serve" in domain for system` → `NotLoaded` (captured on macOS 15, exit 113). `success=false` with other stderr → `Unknown`.
  - BusyBox: `parse_busybox_script(Some(&format_script(...)))` with `/a b/iroh-tunnel` → `Binary("/a b/iroh-tunnel")` (round trip in `busybox.rs` tests); `None` → `NotLoaded`; a script without `DAEMON=` → `Unknown`.
- [ ] **Step 2: Write failing adapter tests** with `FakeRunner`:
  - systemd runs exactly `systemctl show -p LoadState -p ExecStart iroh-tunnel-serve<suffix>.service` (no `--user`); a spawn error → `Unknown`.
  - launchd runs exactly `launchctl print system/dev.iroh-tunnel.serve<suffix>`.
  - `root_run`: `(System, false)` true, `(User, false)` false, `(User, true)` true.
  - `managed_copy_referenced_*` over a temp `RegistrationSources`: match in a file → true; match only in `<dir>/override.conf` → true; an unreadable file (mode 000, skipped as root) → true; nothing, with the loaded query `NotLoaded` → false; loaded query `Unknown` → true.
- [ ] **Step 3:** `cargo test -p iroh-tunnel-host --all-features --locked effective_binary backend platform` → FAIL.
- [ ] **Step 4:** Implement. Adapters use `native.output` (always captured). `RegistrationSources::for_role` lists `/etc/systemd/system/<unit>`, `/etc/init.d/S96iroh-tunnel-<role>`, `/Library/LaunchDaemons/dev.iroh-tunnel.<role>.plist` (with the test suffix) and the drop-in dirs `/etc/systemd/system/<unit>.d`, `/run/systemd/system/<unit>.d`. Missing files and dirs count as no reference; any other I/O error counts as a reference.
- [ ] **Step 5:** Tests PASS on macOS and Linux; clippy clean.
- [ ] **Step 6:** Commit `feat(service): read the effective service binary per backend`.

---

## Task 3: Managed binary use cases — implements 3

**Covers:** 3.1, 3.2, 3.3, 3.4, 3.5
**Difficulty:** Hard
**Blocked by:** Task 1, Task 2

**Files:**
- Create: `crates/host/src/service/service_binary.rs`
- Modify: `crates/host/src/service/mod.rs`, `crates/host/src/service/types.rs`, `crates/host/src/service/native.rs`, `src/render.rs`

**Interfaces:**
- Consumes: `TrustedOwners`, `verify_binary`, `ensure_dir`, `publish`, `remove_entry` (Task 1).
- Produces: `RunningExecutable` (type only; the `InstallRequest` field is added in Task 4), `ServiceBinary`, `managed_copy_path`, `managed_needle`, `choose`, `materialize`, `release`, the new `AppliedEffect` variants and `ServiceNote::ManagedBinary`.

- [ ] **Step 1: Write failing tests** in `service_binary.rs` (base = tempdir, owners = `RootAnd(me())`):
  - `managed_copy_path_is_per_role_under_base` (asserts the path ends with `managed_needle(role)` for any override root, e.g. `/tmp/x`); `production_base_per_os` (`#[cfg(not(feature = "test-utils"))]`-independent: test the pure `fn base_for(os) -> &'static str`).
  - `choose_direct_when_verified`; `choose_managed_when_refused` (the source is `running` and the copy is the given path); `choose_resolves_a_link_before_verifying` (verify is called with the canonical target).
  - `materialize_writes_copy_0755_with_running_bytes` (`read_from` is a fixture file whose bytes differ from `display`).
  - `materialize_refreshes_and_leaves_other_role_bytes`: serve and access copies exist; refresh serve from new bytes; the access bytes are unchanged.
  - `materialize_refuses_unsafe_role_dir`: the role dir pre-created with mode 0777 under a safe tempdir (`RootAnd(me())`) → `Permission` naming the role dir and `0777`, mode unchanged, no file written. The "owned by another user" variant is covered by the pure `trust::check_entry` test (owner `other`, mode 0755 → refused naming owner) and by privileged E2E `sm_bin_04`.
  - `materialize_replaces_planted_link` (target bytes unchanged).
  - `release_removes_copy_and_empty_dir_when_unreferenced`; `release_keeps_copy_when_referenced`; `release_is_idempotent_when_absent`.
  - `render`: the exact `ManagedBinary` line from Contracts.
- [ ] **Step 2:** `cargo test -p iroh-tunnel-host --all-features --locked service_binary` and `cargo test --all-features --locked render` → FAIL.
- [ ] **Step 3:** Implement per design D1–D4 and D7; `release` calls `remove_entry(parent, owners, "iroh-tunnel")`. Gate the OS-touching functions with `#[cfg(any(target_os = "linux", target_os = "macos"))]`; `ServiceBinary` and `RunningExecutable` are portable.
- [ ] **Step 4:** Tests PASS; clippy clean.
- [ ] **Step 5:** Commit `feat(service): managed root-owned copy of the service binary`.

---

## Task 4: Install integration — implements 4

**Covers:** 4.1, 4.2, 4.3, 4.4
**Difficulty:** Hard
**Blocked by:** Task 3

**Files:**
- Modify: `crates/host/src/service/install.rs`, `crates/host/src/service/types.rs` (`InstallRequest.running_executable`), `crates/host/src/service/fixtures.rs`, `src/cli_service.rs`, `crates/host/examples/manage_service.rs`

**Interfaces:**
- Consumes: Task 3 items, `platform::root_run`, `platform::system_wide_only`, `platform::managed_copy_referenced`, `RegistrationSources::for_role`.
- Produces: `plan_binary`, `install_steps`; `InstallPlan.binary: ServiceBinary`.

- [ ] **Step 1: Write failing tests** in `install.rs`:
  - `plan_binary_system_scope_managed_when_refused`, `plan_binary_user_scope_is_resolved_path` (`system_wide_only = false`), `plan_binary_busybox_user_scope_is_root_run` (`system_wide_only = true`), `plan_binary_non_unix_is_direct` (`cfg(not(any(linux, macos)))`).
  - `declined_retarget_writes_no_copy`: `plan_system_install` with the `existing` seam returning a body pinned to another uid returns `Needs(Retarget)`; the test managed dir (`IROH_TUNNEL_TEST_MANAGED_BIN_DIR`, child process) is still empty. Runs where the system plan's account/trust prerequisites can be met by the test user (Linux/macOS non-root: the pin target is the test user and its config fixture); otherwise covered by `sm_bin_05`.
  - `install_steps_writes_copy_then_note_then_registers_copy`: recorded order is `ManagedBinaryWritten`, `ManagedBinary` note, then `register` called with the copy path.
  - `install_steps_register_failure_keeps_copy_in_applied`.
  - `install_steps_direct_releases_copy` (the release closure receives `native` and is called once, after register; never for `Managed`).
  - `managed_path_renders_as_one_argument_in_each_backend`: render systemd, launchd and BusyBox with `/Library/Application Support/iroh-tunnel/bin/serve/iroh-tunnel` and parse back through Task 2 parsers or text checks.
- [ ] **Step 2:** `cargo test -p iroh-tunnel-host --all-features --locked service::install` → FAIL.
- [ ] **Step 3:** Implement. The CLI fills `running_executable`: on Linux `RunningExecutable { display: current_exe()?, read_from: "/proc/self/exe".into() }` (never `read_link`); elsewhere both fields are `current_exe()?`. Call `plan_binary` last in `plan_install`/`plan_system_install`. `install_via` runs `install_steps` inside `execute`, with `materialize(…, RootOnly)`, `backend_install` and `release(…, RootOnly, || managed_copy_referenced(…))`. Update `fixtures::request` and the example.
- [ ] **Step 4:** Workspace `cargo test --all-features --locked` PASS; clippy clean.
- [ ] **Step 5:** Commit `feat(service): register a root-owned binary for system services`.

---

## Task 5: Start, restart and uninstall — implements 5

**Covers:** 5.1, 5.2
**Difficulty:** Normal
**Blocked by:** Task 3

**Files:**
- Modify: `crates/host/src/service/verb.rs`

**Interfaces:**
- Produces: `verb_steps` (Contracts).
- Consumes: `platform::effective_binary`, `platform::root_run`, `verify_binary`, `release`, `managed_copy_path`, `managed_copy_referenced`, `RegistrationSources::for_role`.
- Produces: `fn guard_effective_binary(native: &mut Native, role: &str, verify: &dyn Fn(&Path) -> Result<()>) -> Result<()>` (private).

- [ ] **Step 1: Write failing tests** against `guard_effective_binary` (with `FakeRunner` responses for the query) and `verb_steps` (with closure stubs recording order):
  - `start_refuses_unsafe_effective_binary_before_lifecycle`: `effective` → `Binary(p)`, `verify` → `Permission`. The result is `Permission` whose text contains `p` and `service install --system`; the runner recorded no `start`, `restart`, `kickstart`, `stop` or init-script invocation.
  - `restart_refusal_does_not_stop_the_running_service` (same, `Restart`).
  - `directory_loosened_names_the_directory`: real fixture tree with `RootAnd(me())`; chmod a parent to 0775 after "install"; the guard error names that parent and `0775`.
  - `start_checks_then_starts`: `verify` → `Ok`; the start command runs once after the guard.
  - `restart_never_writes_a_copy`: `applied` has no `ManagedBinary*`, and the managed dir mtime/bytes are unchanged.
  - `not_loaded_falls_through_to_backend`; `unknown_definition_is_permission`.
  - `uninstall_releases_copy_after_registration_removed`: `backend` stub records `RegistrationRemoved`; `release_after_uninstall` runs after it and its effect is appended. `uninstall_cleanup_failure_keeps_registration_removed_in_applied`: the release stub errors; through `execute`, `ServiceFailure.applied` contains `RegistrationRemoved`. `guard_runs_before_backend_and_blocks_it`: a guard error means the backend stub is never called.
  - `user_scope_start_skips_guard` (`root_run` false).
- [ ] **Step 2:** `cargo test -p iroh-tunnel-host --all-features --locked service::verb` → FAIL.
- [ ] **Step 3:** Implement `verb_steps` and call it inside the `execute` closure of `verb_via` (Linux/macOS, `root_run(scope, system_wide_only())`). For `Start`/`Restart`, call `guard_effective_binary(native, role, &|p| verify_binary(p, RootOnly))` before `backend_verb`. For `Uninstall`, after `backend_verb(...)?`, call `release(…)` and push its effect.
- [ ] **Step 4:** Tests PASS; clippy clean.
- [ ] **Step 5:** Commit `feat(service): check the effective binary before system start/restart`.

---

## Task 6: Docs and changelog — implements 6

**Covers:** 6.1, 6.2, 6.3
**Difficulty:** Easy
**Blocked by:** Task 4, Task 5

**Files:**
- Modify: `docs/services.md`, `docs/troubleshooting.md`, `CHANGELOG.md`

- [ ] **Step 1:** `docs/services.md` System services: the binary and every directory above it must be root-owned and not group- or world-writable, or install registers a root-owned copy at the per-OS path. Re-run `sudo iroh-tunnel <role> service install --system` after upgrading. CLI `start`/`restart` exit `3` on an unsafe binary. Starts by systemd/launchd/init at boot are not binary-checked. Uninstall removes the copy.
- [ ] **Step 2:** `docs/troubleshooting.md`: an entry for the exit-3 binary refusal, with the reinstall command.
- [ ] **Step 3:** `CHANGELOG.md` Unreleased: BREAKING note for Linux/macOS `service start/restart --system`; Added note for root-owned copies on Homebrew/cargo system installs.
- [ ] **Step 4:** Commit `docs(service): document system service binary trust`.

---

## Task 7: End-to-end verification — implements 7

**Covers:** 7.1, 7.2, 7.3, 7.4, 7.5, 7.6
**Difficulty:** Hard
**Blocked by:** Task 4, Task 5

**Files:**
- Modify: `tests/service_system.rs`

- [ ] **Step 1: Add privileged tests** (opt-in `IROH_TUNNEL_E2E_PRIVILEGED=1`, root, BusyBox container). Each test sets `PATH` explicitly so the resolved binary is the one under test, uninstalls in a drop guard, and waits for the pidfile and role lock to clear between stop and start.
  - `sm_bin_01_user_bin_dir_registers_managed_copy`: put `BIN` at `/home/it91a/bin/iroh-tunnel` (owner it91a, 0755) and set `PATH=/home/it91a/bin:/usr/bin:/bin`. After install, `DAEMON` is `/var/lib/iroh-tunnel/bin/serve/iroh-tunnel`, the copy is root-owned with mode 0755 and the same bytes as `BIN`, the original's owner and mode are unchanged, stdout has the managed-copy note, and serve reaches running.
  - `sm_bin_02_legacy_unsafe_daemon_refuses_start`: install as in 01, `service stop`, wait for release, then rewrite `DAEMON` to the user path. `service start` exits 3, stderr names the path and `service install --system`, and no pidfile appears. Then run `/etc/init.d/S96iroh-tunnel-serve start` directly: serve runs, since there is no self-check. Stop it, re-install, stop, then `service start` succeeds.
  - `sm_bin_04_role_dir_owned_by_user_refuses`: pre-create `/var/lib/iroh-tunnel/bin/serve` owned by it91a with mode 0755. Install from the user path exits 3 naming that directory and `it91a`'s uid; no script is written; the directory's owner and mode are unchanged. Clean up.
  - `sm_bin_05_declined_retarget_writes_no_copy`: install it91a's serve from root `/usr/local/bin` (direct). Then, as `SUDO_USER=it91b` from a user path, answer `n` to the retarget prompt: exit 5, and `/var/lib/iroh-tunnel/bin/serve` does not exist.
  - `sm_bin_03_root_path_direct_and_uninstall_cleans`: after a managed install, uninstall: the copy and `/var/lib/iroh-tunnel/bin/serve` no longer exist. Then install with `BIN` copied to `/usr/local/bin/iroh-tunnel` (root, 0755) and `PATH=/usr/local/bin:/usr/bin:/bin`: `DAEMON` is that path and no managed copy exists.
- [ ] **Step 2:** In a disposable container: `IROH_TUNNEL_E2E_PRIVILEGED=1 cargo test --all-features --locked --test service_system -- --test-threads=1` → PASS.
- [ ] **Step 3:** macOS manual check: `sudo ./target/debug/iroh-tunnel serve service install --system` run from a user-owned directory with that directory first on `PATH`. Check `launchctl print system/dev.iroh-tunnel.serve` (`program` is the managed copy), `ls -l` of the copy (`root`, `-rwxr-xr-x`), restart (copy bytes unchanged), uninstall (copy gone). Record the results, or why the check was blocked, in `verify.md`.
- [ ] **Step 4:** Gates: `cargo fmt --all -- --check`; `cargo clippy --all-targets --all-features --locked -- -D warnings`; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`; `cargo test --all-features --locked`; `cargo +1.91 check --all-targets --all-features --locked`. For Windows: `cargo check --target x86_64-pc-windows-msvc --all-targets --all-features --locked` if the target is installed, otherwise the CI Windows job, recorded in `verify.md`.
- [ ] **Step 5:** Commit `test(service): privileged e2e for system service binary trust`.

---

## Scenario → test

- Binary in a root-owned directory → `trust::tests::root_only_accepts_a_system_binary`
- Binary owned by the pinned user → `trust::tests::root_only_refuses_a_file_owned_by_the_test_user`
- Binary below a user-owned directory → `trust::tests::binary_refuses_untrusted_parent_naming_it`
- Group-writable binary → `trust::tests::binary_refuses_0775_naming_the_mode`
- Symbolic link in the checked path → `trust::tests::binary_refuses_a_symlink_and_a_directory`
- Sticky directory with a root-owned child → `trust::tests::binary_sticky_parent_trusted_child_passes`
- Sticky directory with a user-owned child → `trust::tests::binary_sticky_parent_untrusted_child_fails`
- Install from a root-owned location → `service_binary::tests::choose_direct_when_verified` + `sm_bin_03`
- Install from a root-owned symbolic link → `service_binary::tests::choose_resolves_a_link_before_verifying`
- Install from a Homebrew prefix → `service_binary::tests::choose_managed_when_refused` + `materialize_writes_copy_0755_with_running_bytes` + Task 7 macOS check
- Install from a cargo bin directory → `sm_bin_01`
- Install note → `render` ManagedBinary test + `install::tests::install_steps_writes_copy_then_note_then_registers_copy` + `sm_bin_01`
- Copy per role → `service_binary::tests::materialize_refreshes_and_leaves_other_role_bytes`
- Re-install refreshes the copy → `service_binary::tests::materialize_refreshes_and_leaves_other_role_bytes`
- Re-install from a trusted location → `install::tests::install_steps_direct_releases_copy` + `sm_bin_03`
- Managed directory pre-created by a user → `trust::tests::check_entry_refuses_other_owner` + `service_binary::tests::materialize_refuses_unsafe_role_dir` + `sm_bin_04`
- Planted link at the copy name → `service_binary::tests::materialize_replaces_planted_link`
- Copy fails before rename → `trust::tests::publish_failure_before_rename_keeps_old_file_and_no_temp`
- Declined retarget writes no copy → `install::tests::declined_retarget_writes_no_copy` + `sm_bin_05`
- Registration write fails after the copy → `install::tests::install_steps_register_failure_keeps_copy_in_applied`
- Effective definition per backend → `effective_binary` parser tests (systemd, launchd, BusyBox) + adapter command tests
- Loaded definition differs from the file → `effective_binary` launchd/systemd `Binary` fixtures + `verb::tests::start_refuses_unsafe_effective_binary_before_lifecycle` (the guard uses the query result, never the file)
- Restart after an upgrade of a managed install → `verb::tests::restart_never_writes_a_copy`
- Trusted registered binary → `verb::tests::start_checks_then_starts`
- Legacy registration with a user-writable binary → `verb::tests::start_refuses_unsafe_effective_binary_before_lifecycle` + `sm_bin_02`
- Directory loosened after install → `verb::tests::directory_loosened_names_the_directory` + `verb::tests::restart_refusal_does_not_stop_the_running_service`
- Unreadable binary in the registration → `verb::tests::unknown_definition_is_permission` + `effective_binary` `Unknown` fixtures
- Boot start of a legacy registration → `sm_bin_02` (init script started directly runs the service)
- Uninstall with a managed copy → `verb::tests::uninstall_releases_copy_after_registration_removed` + `sm_bin_03`
- Another registration still names the copy → `platform` `managed_copy_referenced_*` (file match) + `service_binary::tests::release_keeps_copy_when_referenced`
- Drop-in still references the copy → `platform` `managed_copy_referenced_*` (drop-in match)
- Uninstall without a managed copy → `service_binary::tests::release_is_idempotent_when_absent`
- Link planted at the copy name before uninstall → `trust::tests::remove_entry_absent_and_dangling_link`
- Cleanup fails after the registration is removed → `verb::tests::uninstall_cleanup_failure_keeps_registration_removed_in_applied`
- BusyBox user-scope install from a user-writable binary → `install::tests::plan_binary_busybox_user_scope_is_root_run`
- Not on PATH (delta) → existing `src/cli_service.rs` `resolve_tunnel_executable_with` test + `install::tests::plan_binary_user_scope_is_resolved_path`
- Root-run registration from a user-writable binary (delta) → `install::tests::install_steps_writes_copy_then_note_then_registers_copy`
