## 1. Root-only trust mode in the trust walk

Blocked by: none

Owns: `crates/host/src/service/trust.rs`, `crates/host/src/status/writer.rs` (call sites). Linux and macOS (`cfg(unix)` module); Windows untouched.

- [x] 1.1 Introduce `TrustedOwners { RootOnly, RootAnd(u32) }` and a pure `check_entry(owner, mode, kind, owners)` predicate at the `walk`/`check_parent`/`check_final` boundary; existing helpers keep their behaviour through `RootAnd(uid)` (design D1); existing trust tests stay green
- [x] 1.2 Add `verify_binary(path, owners)` (regular file; owner and parents trusted; no group/other write; sticky rule; no symlink; canonical absolute path) with refusal text naming path, owner and mode; covers "Binary in a root-owned directory", "Binary owned by the pinned user", "Binary below a user-owned directory", "Group-writable binary", "Symbolic link in the checked path", "Sticky directory with a root-owned child", "Sticky directory with a user-owned child"
- [x] 1.3 Add `ensure_dir(path, owners, mode)` creating missing components with `mkdirat` + `fchmod` on the new descriptor through verified parent handles, never chmodding an existing one (design D4); covers "Managed directory pre-created by a user"
- [x] 1.4 Extend `publish` (owner set, `Read` source, exact mode via `fchmod`, temp unlinked on any failure before `renameat`) and add `remove_entry(dir, owners, name)` (no-follow, absent role dir and `ENOENT` are absent, `rmdir` of empty dir); covers "Planted link at the copy name", "Copy fails before rename", "Link planted at the copy name before uninstall"

## 2. Effective binary and reference queries per backend

Blocked by: none

Owns: `crates/host/src/service/effective_binary.rs` (new, portable parsers), `crates/host/src/service/backend/systemd.rs`, `crates/host/src/service/backend/launchd.rs`, `crates/host/src/service/backend/busybox.rs`, `crates/host/src/service/platform.rs`, `crates/host/src/service/mod.rs` (module declaration). Parsers tested on every OS; adapters through `Native` + `FakeRunner`.

- [x] 2.1 `EffectiveBinary { NotLoaded, Binary, Unknown }` and pure parsers for `systemctl show -p LoadState -p ExecStart`, `launchctl print` (+ "Could not find service"), and BusyBox `DAEMON` against captured fixtures (design D6); covers "Effective definition per backend", "Unreadable binary in the registration"
- [x] 2.2 Backend adapters `effective_binary(native, role)` for systemd (system manager), launchd (`system/<label>`) and BusyBox; a failing query is `Unknown`; covers "Loaded definition differs from the file"
- [x] 2.3 `platform::effective_binary(native, role)`, `platform::root_run(scope, system_wide_only)`, `RegistrationSources::for_role(role)` and `platform::managed_copy_referenced(native, sources, needle)` (files, drop-ins in `/etc` and `/run`, loaded definition; unreadable/unknown counts as referenced) (design D7); covers "Another registration still names the copy", "Drop-in still references the copy"

## 3. Managed binary use cases

Blocked by: 1, 2

Owns: `crates/host/src/service/service_binary.rs` (new), `crates/host/src/service/mod.rs`, `crates/host/src/service/types.rs`, `crates/host/src/service/native.rs`, `src/render.rs`.

- [x] 3.1 `RunningExecutable` (type only), `ServiceBinary { Direct, Managed { source, copy } }` and `managed_copy_path(role)` with per-OS base and the `test-utils`-only `IROH_TUNNEL_TEST_MANAGED_BIN_DIR` root override that keeps the fixed suffix (design D1, D3, D9)
- [x] 3.2 `choose(resolved, running, copy, verify)`: canonicalize, check, `Direct` or `Managed` (design D2); covers "Install from a root-owned location", "Install from a root-owned symbolic link", "Install from a Homebrew prefix", "Install from a cargo bin directory"
- [x] 3.3 `materialize(binary, owners)`: `ensure_dir`, publish copy 0755 from `read_from`, verify copy; covers "Re-install refreshes the copy", "Copy per role" (content of the other role unchanged)
- [x] 3.4 `release(copy, owners, referenced)`: conservative on reference errors, link-safe removal; covers "Uninstall with a managed copy", "Uninstall without a managed copy"
- [x] 3.5 `AppliedEffect::{ManagedBinaryWritten, ManagedBinaryRemoved}` and `ServiceNote::ManagedBinary { role, source, copy }` with its rendered stdout line; covers "Install note"

## 4. Install integration

Blocked by: 3

Owns: `crates/host/src/service/install.rs`, `crates/host/src/service/types.rs` (`InstallRequest` field), `crates/host/src/service/fixtures.rs`, `src/cli_service.rs`, `crates/host/examples/manage_service.rs`.

- [x] 4.1 `InstallRequest.running_executable: RunningExecutable` filled by the CLI (`/proc/self/exe` read source on Linux, `current_exe()` on macOS and elsewhere) and the example
- [x] 4.2 Pure `plan_binary(req, scope, system_wide_only, verify)` called last in planning, after every check and prompt; nothing written (design D5); covers "Declined retarget writes no copy", "BusyBox user-scope install from a user-writable binary", "Not on PATH" (user scope unchanged)
- [x] 4.3 `install_steps` ordering: materialize → `ManagedBinaryWritten` → note → register `ServiceBinary::path()`; failure keeps the copy in `applied`; covers "Registration write fails after the copy", "Root-run registration from a user-writable binary" (delta)
- [x] 4.4 After a successful `Direct` root-run install, `release` the role's copy; covers "Re-install from a trusted location"

## 5. Start, restart and uninstall

Blocked by: 3

Owns: `crates/host/src/service/verb.rs`.

- [x] 5.1 For Linux/macOS root-run `Start`/`Restart`: query the effective binary, `verify_binary(RootOnly)`, refuse with exit 3 + `service install --system` guidance before any lifecycle command; `NotLoaded` falls through; `Unknown` refuses; never copies (design D6); covers "Legacy registration with a user-writable binary", "Directory loosened after install", "Trusted registered binary", "Restart after an upgrade of a managed install", "Unreadable binary in the registration"
- [x] 5.2 `verb_steps` seam (guard → backend; backend → release on uninstall) and, after a successful root-run `Uninstall`, `release` the copy inside the same `execute` step; covers "Cleanup fails after the registration is removed"

## 6. Docs and changelog

Blocked by: 4, 5

Owns: `docs/services.md`, `docs/troubleshooting.md`, `CHANGELOG.md`.

- [x] 6.1 `docs/services.md` System services: binary check, managed copy location, re-install after upgrade, start/restart refusal, boot starts not checked
- [x] 6.2 `docs/troubleshooting.md`: exit 3 on start/restart with an unsafe binary → re-install
- [x] 6.3 `CHANGELOG.md` Unreleased: **Breaking** CLI start/restart refusal for legacy unsafe binaries; Added managed copy for Homebrew/cargo system installs

## 7. End-to-end verification

Blocked by: 4, 5

Owns: `tests/service_system.rs` (Linux privileged, BusyBox backend, opt-in `IROH_TUNNEL_E2E_PRIVILEGED=1`).

- [x] 7.1 Privileged E2E: install from a user-owned bin dir with a controlled `PATH` → `DAEMON` is the managed copy (root, 0755, same bytes), original unchanged; serve runs
- [x] 7.2 Privileged E2E: stop, wait for release, point `DAEMON` at a user-writable binary → `service start` exits 3 naming the path, no pidfile; the init script started directly still runs the service (no self-check, "Boot start of a legacy registration"); re-install fixes start
- [x] 7.3 Privileged E2E: uninstall removes the copy and its directory; install from a root-owned path registers it directly
- [x] 7.4 Privileged E2E: a managed role directory pre-created by a user refuses install (exit 3) unchanged; a declined retarget writes no copy
- [ ] 7.5 macOS manual check with the real binary under `sudo` (launchd, user-owned path); record results or the blocked reason in `verify.md`
- [ ] 7.6 Full gates: fmt, clippy, rustdoc, tests (`--all-features --locked`), MSRV 1.91, plus a Windows target check (`cargo check --target x86_64-pc-windows-msvc --all-targets --all-features --locked` or CI)
