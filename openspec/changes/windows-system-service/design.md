## Context

`brainstorm.md` (approved) fixes the what and the policy. This document covers how it maps onto the code.

**Current state.**
- `crates/host/src/service/` holds the scope rules (`decide_install_scope`, `default_scope`, `policy`) and the install/verb/status operations, all of which go through `execute` + `Native`.
- The backends are systemd, BusyBox, launchd and Windows Task Scheduler.
- The Unix-only modules are `system` (installing user), `trust` (no-follow descent, `publish`, `load_config`) and `account`.
- `pin` records `config` + `uid` as a comment line in the registration.
- `ownership.rs` has a user tier and a machine tier. `machine_dir()` returns `None` on Windows.
- On Windows, `is_elevated()` is always `false`, `install.rs` forces user scope, and `policy::ensure_supported` refuses system scope with exit 5.
- Runtime: `runtime/skeleton.rs` claims ownership, then calls `service::load_config`. Stop comes from `src/lifecycle.rs`, which waits for Ctrl+C/SIGTERM and runs the shared drain (3 s budget, then a 10 s watchdog).
- Status snapshots go through `status/writer.rs`, which uses `trust::publish` under a pinned system service.

**DDD boundaries touched.**
- **Service management (bounded context).**
  - Use cases: install, uninstall, start, stop, restart, status, and run-as-service.
  - Invariants: one scope per role on Windows; no registration without a trusted pin; no elevation and no fallback.
- **Privileged trust (domain policy).** Invariant: SYSTEM never acts on content an untrusted principal can change. On Windows this becomes a pure evaluation over a security-descriptor model, with a thin Win32 adapter.
- **Role ownership.** Invariant: at most one live process per role per tier. The machine tier extends to Windows.
- **Adapters (platform effects).** Service Control Manager (SCM), Win32 security and file APIs, Task Scheduler, and the log file.

## Goals / Non-Goals

**Goals:** implement the approved brainstorm. That means Windows system scope as a Windows Service, the Windows trust check (config/state and binary), the Windows machine tier and service log, Windows elevation and scope rules, cross-scope refusal, the `install.ps1` directory fix, and update guidance. Each piece reuses the existing seams instead of adding parallel ones.

**Non-Goals:**
- `--user` changes (#185) and the Unix binary check (#186).
- Accounts other than LocalSystem.
- The Event Log.
- Refactoring Unix code beyond widening shared types: `Pin` and the privilege guidance.

## Decisions

### D1. Spike first, recorded in this change
Before the trust code is written, a test-only measurement runs on `windows-latest` and `windows-11-arm`. It dumps owner, DACL (SDDL) and reparse attributes for the following:
- the user's `%LOCALAPPDATA%` chain;
- `C:\`;
- `C:\Program Files` and a directory created by `install.ps1` under UAC;
- the elevated token's user SID compared with the non-elevated one.

The results are appended to this design under "Measurements". A result that contradicts brainstorm assumptions 1-4 stops implementation and returns to the owner. *Why:* the whole trust rule rests on these four facts.

### D2. Trust evaluation is a pure model plus a Win32 adapter
New module `service/trust_windows/`:
- **`model`** (all platforms, unit-tested on every CI host). It holds `Sid`, `Ace { sid, mask, flags, allow }` and `Descriptor { owner, dacl: Option<Vec<Ace>>, reparse: bool, is_dir }`, plus `evaluate(chain, trusted: &TrustedSet, target: Target) -> Result<(), Refusal>`.
- **`win32`** (cfg windows). It opens each component, from the volume root down, with `CreateFileW(READ_CONTROL | FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE, FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)`. Leaving out `FILE_SHARE_DELETE` means no component can be renamed or deleted while we hold the chain. It reads attributes with `GetFileInformationByHandle` and owner/DACL with `GetSecurityInfo`, then builds the model.

Rules in `evaluate`:
- **Reparse points.** Any reparse point on any component is refused: symlinks, junctions, mount points.
- **Unreadable DACL.** A NULL DACL, or one we cannot read, is refused.
- **Owner.** The owner must be in the trusted set, because an owner has implicit `WRITE_DAC`.
- **Which ACEs count.** Only allow-ACEs that apply to the object itself count; inherit-only ACEs (including CREATOR OWNER templates) do not. Deny-ACEs are ignored, which is conservative: a deny can only remove access.
- **Final object.** An untrusted SID with any of `FILE_WRITE_DATA`, `FILE_APPEND_DATA`, `FILE_WRITE_EA`, `DELETE`, `WRITE_DAC`, `WRITE_OWNER`, `FILE_DELETE_CHILD`, `GENERIC_WRITE` or `GENERIC_ALL` is refused.
- **Ancestor directory.** An untrusted SID with `FILE_DELETE_CHILD`, `DELETE`, `WRITE_DAC`, `WRITE_OWNER` or `GENERIC_ALL` is refused. `FILE_ADD_FILE` and `FILE_ADD_SUBDIRECTORY` are allowed: they cannot replace a trusted child, which is the Windows analogue of the sticky `/tmp` rule. The child is still checked on its own.
- **Trusted sets.** For config/state: SYSTEM (`S-1-5-18`), Administrators (`S-1-5-32-544`), TrustedInstaller (`S-1-5-80-956008885-...`) and the pinned SID. For the binary: the same set without the pinned SID.
- **Refusal message.** A `Refusal` names the path, the owner as `DOMAIN\name (SID)` and the offending SID with its rights. It is reported as `CliError::Permission` (exit 3).

*Alternative:* `AccessCheck` with an impersonated or synthesized token per principal. Rejected: it needs a token for every principal on the DACL (group expansion), and it hides which ACE failed. The explicit model is testable and explains its refusals.

### D3. Reading and writing through verified handles
- **Config.** It is read from the verified final handle (`ReadFile` on that handle), never reopened by path.
- **Status snapshot under a system service (link-safe write).** The writer verifies the state directory chain and keeps the directory handle. It creates the temp file with `NtCreateFile`-equivalent semantics relative to that handle (`CreateFileW` on the verified path, plus `FILE_FLAG_OPEN_REPARSE_POINT` and `CREATE_NEW`, followed by a check that the new file's final path is under the held directory). It then renames with `SetFileInformationByHandle(FileRenameInfoEx, RootDirectory = dir handle, REPLACE_IF_EXISTS | POSIX_SEMANTICS)`. This mirrors `openat`/`renameat` in `trust::publish`, and `status/writer.rs` gains a cfg-windows branch beside the Unix one.
- **Key.** As on Unix, a system service never writes the user's config. An empty `secret_key` is the same configuration error with `config keygen` guidance.

### D4. Pin, installing user and account names
- **`Pin`.** `Pin.uid: u32` becomes `Pin.owner: PinOwner { Uid(u32), Sid(String) }`.
  - The record text gains `sid=<S-1-...>`. Unix records keep `uid=`, and records already written stay parseable (`find` accepts either).
  - `account_name` takes `&PinOwner`; on Windows it uses `LookupAccountSidW` to give `DOMAIN\name`.
  - The host crate is `publish = false`, so this only changes the workspace-internal API (`src/cli_service.rs`).
- **Installing user.** The token user comes from `GetTokenInformation(TokenUser)`. The `--config` directory owner is a cross-check:
  - a user SID that differs from the token user is exit 2, naming both accounts;
  - Administrators, SYSTEM or TrustedInstaller as owner names no person, so the token user stands (brainstorm decision 11).
  - Without `--config`, the paths are the token user's `%LOCALAPPDATA%` defaults, resolved through `SHGetKnownFolderPath(FOLDERID_LocalAppData, token)` and never through the process environment.
- **Where the pin lives.** It is stored in the service's registry key, `HKLM\SYSTEM\CurrentControlSet\Services\iroh-tunnel-<role>`:
  - **`Description`** holds `iroh-tunnel-pin config=... sid=...`, set through `windows-service` and read back as a registry string, so no text is localized.
  - **`Environment`** (REG_MULTI_SZ, honoured by the SCM for the service process) holds `IROH_TUNNEL_STATE_DIR=<pinned>`, `IROH_TUNNEL_TRUSTED_SID=<sid>` and `IROH_TUNNEL_SCM=1`.
  - *Why the registry:* the SCM API has no environment field, and the key is SYSTEM/Administrators-writable only, like a root-owned unit file.
- **Service name and command line.** The name is `iroh-tunnel-<role>` (plus the test suffix). The command line is `"<binary>" <role> run --config "<config>"`, the same documented `run` CLI with no hidden flag.

### D5. Running under the SCM
- **Detecting the SCM.** `src/app.rs` checks `IROH_TUNNEL_SCM=1` before building the runtime. If it is set, `service_dispatcher::start` runs the role in `service_main`. A dispatcher failure with `ERROR_FAILED_SERVICE_CONTROLLER_CONNECT` means the variable was set outside the SCM: that is a configuration error, never a silent foreground run. Without the variable, behaviour is unchanged.
- **Stop path.** `lifecycle.rs` gains a stop source, `StopSource::Scm`. The control handler maps `Stop`, `Shutdown` and `Preshutdown` onto the same trigger that SIGINT/SIGTERM use (RO-03, an OS signal), so the drain budget, the repeated-signal and watchdog rules all apply unchanged.
- **Reported states.** The handler reports `StopPending` with `wait_hint` 15 s (3 s drain + 10 s watchdog + margin) and increments `checkpoint` every second until exit. It reports `Running` only after the ownership claim and the config load succeed.
- **Exit mapping to the SCM.**
  - Normal exit → `Stopped`, `Win32(0)`.
  - Role already running (exit 6) → `Stopped`, `Win32(0)`, logged, so recovery does not fire (the analogue of `RestartPreventExitStatus=6`).
  - Any other error → `ServiceSpecific(exit code)`, which the SCM treats as a failure and which triggers recovery because `failure_actions_on_non_crash_failures` is enabled.
- **Panic.** A panic in the role is caught at the `service_main` boundary and reported as a failure exit before the process ends.

### D6. Registration and recovery
- **Install.** `CreateService` uses `OWN_PROCESS`, `AutoStart`, `ErrorControl::Normal` and LocalSystem. The shared `RestartPolicy` maps to `ServiceFailureActions`: restart after `RestartSec` (5 s), reset period 1 day; when the policy is disabled, no actions.
- **Order of steps.** Install is a replace: it stops and deletes any existing service, waits until it is gone, then creates, configures and starts the new one. A failure after `CreateService` rolls back by deleting the service, so nothing half-registered remains.
- **Testing seam.** The pure parts take a plain `ServiceSpec` and are unit-tested everywhere: the command-line rendering and quoting, the environment block, the pin record and the failure-action mapping. Only `backend/windows_service.rs` touches `windows-service`.
- **Verbs.** Start, stop, restart, status and uninstall open the service with the least access each needs. Status reads `ServiceState` (structured) plus the lock and owner record, as `ServiceStatusReport` already models. `restart` stops, waits for the lock to release (15 s, as for the task backend), then starts.

### D7. Windows machine tier and machine directory
- **`machine_dir()`.** On Windows it returns `%ProgramData%\iroh-tunnel`, from `FOLDERID_ProgramData` rather than the environment.
- **Elevated create.** Creation uses an explicit protected security descriptor: `O:BAG:SYD:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)`. That gives SYSTEM and Administrators full access and Users read/execute, with no inheritance from ProgramData. If the directory already exists, it goes through the D2 evaluation with trusted set {SYSTEM, Administrators, TrustedInstaller}; an untrusted one is fail closed and is never repaired.
- **Lock semantics.** `ownership.rs` keeps its exclusive/shared semantics through `std::fs::File::try_lock`/`try_lock_shared`, which are stable on MSRV 1.91 and use `LockFileEx` on Windows.
  - A non-elevated process opens the lock read-only and shared. Users have read access, and `PermissionDenied` on create already counts as "not elevated".
  - The lock files are never deleted and the owner record stays a separate file, which already holds for mandatory locks.
- **Service log.** `logs\<role>.log` lives in the machine directory. Tracing in a service process writes to it through a small size-capped writer (10 MiB, one rotated `.1`), with no new crate. Foreground and CLI processes keep stderr.

### D8. Elevation, scope selection and guidance on Windows
- **Elevation and defaults.** `is_elevated()` on Windows uses `GetTokenInformation(TokenElevation)`. `install.rs` drops the forced-user line, and `policy::ensure_supported` no longer refuses Windows system scope. Elevated without a flag therefore means system, for every verb (breaking change, CHANGELOG).
- **Privilege guidance.** `policy::ensure_privilege` stays one function but takes the rerun line from a platform hook:
  - Unix: `sudo <argv>`;
  - Windows: "run from an elevated terminal (Run as administrator): `<argv>`".
  - The exit code (3) and the absolute config path in the guidance stay the same.
- **Cross-scope refusal (install only).** It uses existence probes that need no admin rights:
  - `--system` checks the role's user task with the existing `schtasks /Query` exit status;
  - `--user` checks the service with `OpenService(SERVICE_QUERY_STATUS)` under `SC_MANAGER_CONNECT`.
  - The refusal is a service error (exit 5) naming the uninstall command for the other scope.
- **Binary check.** It runs in `plan_system_install` on the resolved binary, and in `verb_via` for system `Start`/`Restart` on the binary read back from the registration (`QueryServiceConfig`, structured).

### D9. `install.ps1` and update
- **`install.ps1`.** Changed only if D1 shows that the directory it creates grants the creating user write access. In that case, after creating `$BinDir`, it sets the owner to Administrators and replaces the DACL with Program Files' inherited entries minus any entry for the current user SID (protected). It never touches an existing directory it did not create. A PowerShell test in CI checks that the result passes the binary rule.
- **Update.** The Windows locked-executable failure (`self-update`) appends: "if this executable runs as a system service, stop it first: `iroh-tunnel <role> service stop --system` (elevated)". The hint is unconditional, so update never queries the SCM, and DIST-02 is unchanged.

### D10. Dependencies
- `windows-service` (latest compatible with MSRV 1.91) and `windows-sys`, both under `[target.'cfg(windows)'.dependencies]` of `crates/host` (and of the root crate for the dispatcher), with only the `Win32_Security*`, `Win32_Storage_FileSystem`, `Win32_System_Threading`, `Win32_UI_Shell` and `Win32_System_Registry` features.
- `windows-sys` is pinned to a version already in `Cargo.lock` if possible, to avoid a second copy.
- Every `unsafe` block lives in the `win32` adapters, each with a `// SAFETY:` comment (RS-12).

## Risks / Trade-offs

- [D1 contradicts an assumption, for example a CREATOR OWNER entry or the default `%LOCALAPPDATA%` ACL including another principal] → Implementation stops at the spike and the owner decides. `install.ps1` is prepared for the expected case.
- [The ACE evaluation misses a right that permits replacement] → Conservative masks: a final object refuses any write-family right, and ancestor checks refuse delete-family rights. Unit tests cover each right, including generic rights and inherit-only flags. E2E grants Everyone each right on a config and on an ancestor, and expects a refusal.
- [A binary replaced after a CLI check runs at boot or recovery] → Accepted and documented (owner decision). The install directory is protected by D9.
- [Hard links: a user creates a hard link inside their profile to a file they cannot otherwise write] → Creating a hard link needs write access to the target, so the content is already user-controllable and gains nothing. Hard links are not refused.
- [The SCM kills a slow stop] → The checkpoint and wait hint cover the drain plus the watchdog; the watchdog exits first.
- [The breaking elevated default surprises scripted admins] → CHANGELOG entry, docs, and an explicit scope flag in the printed guidance.
- [Log growth] → Size cap with one rotation.
- [Trust for status writes adds latency on each snapshot write] → One chain verification per write, which matches the Unix behaviour already accepted.
- [The second `windows-sys` version grows the build] → Prefer the version already locked; accept a second one if `windows-service` requires it.

## Migration Plan

- There are no existing Windows system services, so nothing needs migrating.
- Existing `--user` tasks are unchanged. The only behaviour change for them is the elevated default (CHANGELOG, `docs/services.md`).
- Rollback is `service uninstall --system` for any installed service; downgrading the binary restores the exit-5 refusal.
- `ProgramData\iroh-tunnel` is left in place on uninstall, like `/run` and `/var/log`. Docs say how to remove it.

## Open Questions

- The D1 measurements. Implementation of D2, D7 and D9 waits on them.
- Whether `FileRenameInfoEx` with `POSIX_SEMANTICS` is available on every supported Windows build. The fallback is `FileRenameInfo` with `ReplaceIfExists`; this is confirmed in D1 on the runners.
