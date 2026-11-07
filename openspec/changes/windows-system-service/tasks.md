<!--
File ownership and order (one writer per worktree; groups run in order unless noted):
- Group 1 owns tests/windows_trust_probe.rs (test-only) and appends "Measurements" to design.md. It gates groups 3, 6 and 9: a contradicted assumption stops work and returns to the owner.
- Group 2 owns crates/host/src/service/{is_elevated,policy,install,verb,default_scope,decide_install_scope}.rs.
- Group 3 owns crates/host/src/service/trust_windows/** and Cargo.toml dependency entries.
- Group 4 owns crates/host/src/service/{pin,account_name,system}.rs and src/cli_service.rs (Pin API). It depends on 2.
- Group 5 owns crates/host/src/ownership.rs, crates/host/src/paths/** (machine dir) and the service log writer. It depends on 3.
- Group 6 owns crates/host/src/service/backend/windows_service.rs, platform.rs and registration.rs. It depends on 3, 4 and 5.
- Group 7 owns src/app.rs, src/lifecycle.rs, crates/host/src/runtime/** and crates/host/src/status/writer.rs. It depends on 3, 5 and 6.
- Group 8 owns the cross-scope and update-guidance code. It depends on 6.
- Group 9 owns install.ps1 and its PSScriptAnalyzer and Windows checks. It depends on 1 and 3.
- Group 10 owns tests/service_system_windows.rs and .github/workflows/e2e.yml. It depends on 6 to 9.
- Group 11 owns docs/**, CHANGELOG.md and the openspec bookkeeping.

Platforms:
- Pure logic (trust model, scope rules, pin, failure-action mapping, command line, environment block) is unit-tested on Linux, macOS and Windows CI.
- Win32 adapters and lifecycle are tested only on windows-latest (x64), plus windows-11-arm where the runner exists. The E2E runners are elevated.

Traceability:
- WST = windows-service-trust
- SSM = system-service-management
- ROS = role-ownership-shutdown
- FIM = frontend-independent-management
- IRP = install-release-packaging
- SU = self-update
-->

## 1. Spike: measure the Windows assumptions (design D1)

- [x] 1.1 Add the ignored test `tests/windows_trust_probe.rs` (cfg windows, `IROH_TUNNEL_E2E_SERVICE=1`). It prints SDDL, owner and reparse attributes for:
  - every component of the user's `%LOCALAPPDATA%\iroh-tunnel` chain;
  - `C:\` and `C:\Program Files`;
  - a directory created the way `install.ps1` creates `$BinDir`;
  - `%ProgramData%`;
  - the elevated and filtered token user SIDs.

  It also checks whether `SetFileInformationByHandle(FileRenameInfoEx, POSIX_SEMANTICS)` succeeds. Run it via `gh workflow run e2e.yml --ref feat/windows-system-service` on x64 (and ARM64 if available).
- [x] 1.2 Append a "Measurements" section to `design.md` with the raw results and a verdict for brainstorm assumptions 1-4 and the rename open question. If an assumption is contradicted, stop and send an Inbox question to the owner before groups 3, 6 and 9 continue.

## 2. Scope rules and elevation on Windows

- [x] 2.1 Implement `is_elevated()` on Windows with `GetTokenInformation(TokenElevation)`, keeping the existing test shape. (SSM: Install scope selection; Default scope for verbs)
- [x] 2.2 Remove the forced-user line in `install.rs`, and remove the Windows branch of `policy::ensure_supported`. Unit-test `decide_install_scope`/`default_scope` on Windows for: elevated without a flag means system; a non-elevated terminal prompts; non-elevated without a terminal means user. (SSM: Elevated Windows terminal installs without a scope prompt; Elevated Windows terminal without a flag; Interactive user is offered both scopes)
- [x] 2.3 Make the `ensure_privilege` rerun guidance platform-specific: `sudo <argv>` on Unix, "Run as administrator" plus `<argv>` on Windows. Exit code 3 and the absolute config path stay. Unit tests for both texts. (SSM: Non-elevated system install on Windows; Non-elevated system verbs on Windows; FIM: Non-elevated Windows system scope; Windows system scope without elevation)

## 3. Windows trust model (design D2, D3)

- [x] 3.1 Add `windows-sys` as a direct `cfg(windows)` dependency of `crates/host`, reusing the locked version and only the features named in D10. Add `windows-service` likewise to `crates/host` and to the root crate. Confirm `cargo tree` resolves on MSRV 1.91.
- [x] 3.2 Add the pure `trust_windows::model` (all platforms): `Sid`, `Ace`, `Descriptor`, `TrustedSet` (config/state set with the pinned SID; binary set without it), and `evaluate`. Write a TDD test per rule:
  - reparse point refused;
  - NULL or unreadable DACL refused;
  - untrusted owner refused;
  - inherit-only ACE ignored;
  - each final-object write right refused (data, append, EA, delete, `WRITE_DAC`, `WRITE_OWNER`, delete-child, generic write/all);
  - each ancestor replacement right refused;
  - create-file/subdirectory on an ancestor accepted;
  - deny ACEs ignored;
  - the refusal names the path, owner and offending SID with its rights.

  (WST: Trusted principals; Path components are held and never followed; Owner and DACL of every component; Write rights refused on the final object; Replacement rights refused on ancestors; Never repair access on user files)
- [x] 3.3 Add the `trust_windows::win32` adapter (cfg windows): descend from the volume root with `CreateFileW` (`FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS`, no `FILE_SHARE_DELETE`), then `GetFileInformationByHandle` and `GetSecurityInfo`, feeding `evaluate`. Each `unsafe` block gets a `// SAFETY:` comment. Windows unit tests:
  - a temp tree with an Everyone-modify ACE is refused;
  - a junction is refused;
  - a NULL DACL is refused;
  - the profile-default ACL passes.

  (WST: Junction in the config path; Everyone may modify the config; NULL DACL; Config in the pinned user's profile; Binary writable by the pinned user)
- [x] 3.4 Add `read_verified` (read the config from the verified handle) and `publish` / `remove_in` for the status snapshot: create the temp file with `CREATE_NEW` + no-follow under the held directory, then rename relative to the directory handle, using the D1 fallback if needed. Windows unit test: a planted symbolic link at the snapshot name is replaced and its target is unchanged. (WST: Verified handles only)

## 4. Pin, installing user and accounts (design D4)

- [x] 4.1 Change `Pin.uid` to `Pin.owner: PinOwner { Uid, Sid }`. The record text emits `uid=` or `sid=`; `find` parses both, and existing Unix records still parse. Update `account_name` to take `&PinOwner` (`LookupAccountSidW` on Windows) and port `src/cli_service.rs`. Unit tests cover round trip, legacy record and malformed record. (SSM: Registration pins the owner — Pin in the script; Pin in the Windows service)
- [x] 4.2 Windows installing user: take the token user SID, resolve `%LOCALAPPDATA%` with `SHGetKnownFolderPath(FOLDERID_LocalAppData, token)`, and cross-check the `--config` directory owner:
  - a different user SID gives exit 2 naming both accounts;
  - Administrators, SYSTEM or TrustedInstaller defers to the token user.

  The pure decision gets unit tests; the Win32 lookups go in the adapter. (SSM: Windows installing user — Elevated user installs with defaults; Windows token user and --config owner must agree — Config owned by another user; Config directory owned by Administrators)

## 5. Machine directory, machine-tier lock and service log (design D7)

- [x] 5.1 Make `machine_dir()` on Windows return `%ProgramData%\iroh-tunnel` from `FOLDERID_ProgramData`; the test-utils override is kept. Elevated create uses the protected SDDL from D7. An existing directory is evaluated with the {SYSTEM, Administrators, TrustedInstaller} set and refused, never repaired. Non-elevated: open read-only and shared, never create. Windows unit tests cover create, a pre-existing untrusted directory and the non-elevated path. (SSM: Windows machine directory — Directory pre-created by a user; Directory made untrusted after install; ROS: Machine-tier lock between system service and user processes)
- [x] 5.2 Extend the existing exclusive/shared role-lock tests to Windows (`File::try_lock`/`try_lock_shared`): a system holder refuses a user start with exit 6 "system service"; a user holder blocks the exclusive claim. (ROS: Machine lock location; System lock held, user start; User running, system start; Windows user task while the system service runs)
- [x] 5.3 Add a size-capped log writer (10 MiB plus one `.1` rotation, no new crate) at `logs\<role>.log` in the machine directory. Use it only when running as a system service, and never write when the machine directory failed trust. Unit tests cover rotation and the cap. (SSM: Windows system service log — Log available after a refusal)

## 6. Windows Service backend (design D4, D6)

- [x] 6.1 Add pure rendering in `backend/windows_service.rs`, unit-tested on all platforms:
  - `ServiceSpec`: name with test suffix, quoted command line `"<binary>" <role> run --config "<config>"`;
  - environment block `IROH_TUNNEL_STATE_DIR`, `IROH_TUNNEL_TRUSTED_SID`, `IROH_TUNNEL_SCM=1`;
  - pin description;
  - `RestartPolicy` mapped to failure actions (5 s, reset after 1 day, none when disabled, applied to non-crash failures).

  Credit #67 with `Co-authored-by: 23rd <23rd@vivaldi.net>` where its code is reused. (SSM: Windows system service registration; Windows service recovery and exit mapping)
- [x] 6.2 Add the SCM adapter (cfg windows) through `windows-service`:
  - install = stop, delete, wait-gone, create (LocalSystem, AutoStart), set description, environment and failure actions, then start; any failure after create deletes the service;
  - start/stop/restart/status/uninstall use least-access opens and structured `ServiceState`;
  - restart waits up to 15 s for the role lock;
  - a missing service gives exit 5 with `service install --system`;
  - stop of an already-stopped service and uninstall of an absent one succeed.

  (SSM: Windows system install replaces and rolls back; Windows system verbs on a missing service; Windows restart waits for the old process — System service restart)
- [x] 6.3 Wire system scope on Windows into `install.rs` (`plan_system_install`), `verb.rs`, `status.rs` and `platform::existing_registration`:
  - verify state and config through the trust check, and the binary with the binary set, before registering;
  - read the pin back for the retarget decision;
  - CLI `start`/`restart --system` re-check the binary read back with `QueryServiceConfig`.

  (WST: Trust check timing — Unsafe binary at install; Unsafe binary at CLI start; SSM: Windows retarget confirmation — all three; Linux backend chosen at runtime — Windows scopes)

## 7. Running under the Service Control Manager (design D5)

- [x] 7.1 In `src/app.rs`, when `IROH_TUNNEL_SCM=1`, start `service_dispatcher` and run the role in `service_main`. `ERROR_FAILED_SERVICE_CONTROLLER_CONNECT` gives a configuration error (exit 2) with no foreground fallback. A panic is caught at the `service_main` boundary and reported as a failure. (SSM: Windows service start requires the service context and a pin — Service marker outside the SCM)
- [x] 7.2 Add `StopSource::Scm` to `lifecycle.rs`. Stop, Shutdown and Preshutdown feed the same trigger as SIGINT/SIGTERM; the drain budget, repeated-signal and watchdog rules are unchanged. Report StopPending with a 15 s wait hint and advance the checkpoint every second. Unit-test the trigger mapping on all platforms. (ROS: Stop signals begin a graceful drain — Service stop request; SSM: Windows service stop drains — Stop removes the status file; Stop pending while draining)
- [x] 7.3 Report Running only after the ownership claim and the trust-checked config load. Exit-status mapping:
  - exit 6 → Stopped with `Win32(0)`;
  - other errors → `ServiceSpecific(code)`;
  - normal exit → Stopped with `Win32(0)`.

  (SSM: Windows service reports running only after startup checks — Invalid config at start; Windows service recovery and exit mapping — Role already running; Crash recovery)
- [x] 7.4 Windows branches of `service::load_config` and `status/writer.rs`:
  - with `IROH_TUNNEL_TRUSTED_SID`: verify the state directory, read the config from the verified handle, refuse an empty `secret_key` with `config keygen` guidance, and publish snapshots link-safely;
  - running as a service without a parseable SID: permission error naming `service install --system`.

  (WST: Unsafe config at service start; Verified handles only; SSM: Windows system service never writes a generated key — Config without a key; Service without a pin)

## 8. Cross-scope refusal and update guidance

- [x] 8.1 Windows install refuses the other scope:
  - `--system` probes the user task (`schtasks /Query` exit status);
  - `--user` probes the service (`OpenService(SERVICE_QUERY_STATUS)`);
  - exit 5 naming `service uninstall --user|--system`, nothing registered.

  Unit-test the decision with fake probes. (SSM: Windows cross-scope registration is refused — User task exists; System service exists)
- [x] 8.2 Append the unconditional `iroh-tunnel <role> service stop --system` (elevated) hint to the Windows locked-executable failure in `update/`. Nothing is stopped. Unit test on the message; extend `tests/update_zip_windows.rs`. (SU: Locked executable on Windows fails cleanly — Running image; Guidance for a system service)

## 9. Installer directory protection (design D9)

- [x] 9.1 If the D1 measurement shows a creator-writable ACE: after `install.ps1` creates `$BinDir` (default or `-BinDir`), set the owner to Administrators and a protected DACL derived from the parent without entries for the current user SID. Never touch a pre-existing directory. Keep PSScriptAnalyzer clean. If D1 shows nothing is needed, record that in `design.md` and mark this task done without changing code. (IRP: Windows installer protects the install directory it creates)
- [x] 9.2 Add a Windows CI check: run `install.ps1` against a local release fixture into a fresh directory, then assert the binary trust check passes; a pre-existing directory keeps its SDDL. (IRP: Fresh default install; Existing directory untouched)

## 10. End-to-end on Windows

- [ ] 10.1 Add `tests/service_system_windows.rs` (cfg windows, `IROH_TUNNEL_E2E_SERVICE=1`, unique test suffix, cleanup on drop) covering:
  - full lifecycle: install, running, status, stop (drain removes the snapshot), start, restart, uninstall;
  - stop-pending during a drain;
  - a refused unsafe config with the log line;
  - the fixed condition (the service runs again);
  - a junction swap;
  - a pre-created untrusted machine directory;
  - a missing service;
  - retarget declined;
  - cross-scope refusal both ways;
  - user task blocked while the service runs.

  (traces the SSM, WST and ROS scenarios listed in groups 3-8)
- [ ] 10.2 Cover the non-elevated paths:
  - a restricted-token child process (`CreateRestrictedToken`) asserts exit 3 and "Run as administrator" for install and verbs;
  - if a restricted token is not obtainable on the runner, the decision logic is covered by group 2 unit tests and the gap is recorded in `verify.md`.

  (SSM: Non-elevated system install on Windows; Non-elevated system verbs on Windows; FIM scenarios)
- [ ] 10.3 Add the step "Service lifecycle (Windows Service)" to the Windows cell of `.github/workflows/e2e.yml`, running `service_system_windows` with `--test-threads=1`. Run the workflow on the branch and record the run URL.

## 11. Documentation and release notes

- [ ] 11.1 `docs/services.md` Windows section, covering:
  - system scope;
  - the elevated default;
  - the trust rules and how to fix a refusal;
  - `%ProgramData%\iroh-tunnel` (lock, log, leaving it on uninstall);
  - the documented boot/recovery binary-check gap;
  - cross-scope refusal.

  Update `docs/troubleshooting.md`: replace "Windows has no system services (exit 5)" with the exit-3 elevated guidance, and add the trust refusal messages.
- [ ] 11.2 CHANGELOG under Unreleased:
  - Added: Windows system services;
  - Changed (breaking): an elevated Windows terminal without a scope flag now targets system scope; non-elevated `--system` exits 3 instead of 5;
  - credit to @23rd for #67.
- [ ] 11.3 Run `openspec validate windows-system-service --strict`; run `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, and `cargo test --workspace --all-features --locked` on the local host. Confirm that the Windows CI unit and E2E cells are green.
