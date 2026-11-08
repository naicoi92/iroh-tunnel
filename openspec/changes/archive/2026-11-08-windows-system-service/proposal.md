## Why

On Windows, `service install --system` exits with code 5 because SM-04 (#79) deferred Windows system scope until a trust model existed. Windows operators therefore cannot run a role from boot without a logged-in user. The user-scope task is also hard-terminated on stop, so it never drains. Community PR #67 showed demand. The Unix system scope (#134) has since established the model to mirror: LocalSystem/root runtime, pinned user paths, a fail-closed trust check and no ACL changes. Issue #184 tracks this change.

## What Changes

**Windows system scope**
- From: `--system` on Windows exits 5 with `--user` guidance; no Windows Service is ever created.
- To: `--system` installs and manages a per-role Windows Service that starts automatically, runs as LocalSystem, reads the installing user's pinned config and state, recovers on failure per the shared restart policy, and drains on stop.
- Reason: parity with Linux/macOS system scope.
- Impact: non-breaking for explicit `--user` callers; the exit-5 refusal disappears.

**Windows scope selection and privilege**
- From: Windows is never elevated for scope purposes; install always selects user scope without a prompt.
- To: Windows uses a real elevation check and the shared rules. Elevated without a flag selects system scope for every verb. A non-elevated interactive install offers user/system. `--system` without elevation exits 3 with elevated-terminal rerun guidance.
- Reason: one scope model across platforms (owner decision).
- Impact: **breaking** for anyone who runs `service` verbs in an elevated Windows terminal without a scope flag; noted in the CHANGELOG.

**Windows trust check**
- New: at install and at each service start, the config, the state directory and every directory above them must be writable only by SYSTEM, Administrators or the pinned user. Every reparse point is refused. The config is read only from the verified handle. A failure is fail closed and names the path, owner and offending access. The tool never changes the user's ACLs.
- New: the service binary and every directory above it must be writable only by SYSTEM, Administrators or TrustedInstaller. This is checked at install and by the CLI `start`/`restart`. Starts by the Service Control Manager at boot or through recovery actions are not checked; this gap is documented.

**Windows machine directory and machine-tier lock**
- From: the machine-tier role lock exists only on Linux and macOS.
- To: Windows gets `%ProgramData%\iroh-tunnel`, created by the service with its own ACL (SYSTEM/Administrators write, Users read) and refused if it already exists with an untrusted owner or writer. It holds the machine-tier lock, so a role's user task and system service exclude each other in either start order. It also holds the bounded service log.

**Windows cross-scope registration**
- New: on Windows, installing one scope while the same role is registered in the other scope is refused, with uninstall-first guidance. Task Scheduler cannot exempt the "already running" exit from its restart policy.

**Windows installer**
- New: `install.ps1` gives the install directory it creates an Administrators owner and drops any inherited access entry that lets the creating user write it. This is applied only if measurement shows such an entry exists.

**Update guidance**
- New: when a Windows system service holds the executable, the locked-executable message adds `service stop --system` guidance. Update still never stops a running tunnel.

### Non-goals

- The `--user` mechanism and its graceful stop (#185).
- The binary trust rule for Linux/macOS (#186).
- Any runtime account other than LocalSystem.
- Moving the user's config or state, or changing ACLs on the user's files.
- Event Log integration.
- `service` on the standalone role binaries.

## Capabilities

### New Capabilities
- `windows-service-trust`: the Windows trust policy for system scope. It defines the trusted principals for config/state and for the binary, the per-component reparse-point and DACL checks, the timing of the checks (install, service start, CLI start/restart), the failure reporting and the no-ACL-change rule for user files.

### Modified Capabilities
- `system-service-management`:
  - Windows system scope replaces "Windows user background task only" and the exit-5 scenario.
  - Adds the Windows Service registration (auto start, LocalSystem, recovery actions, the "already running" exit treated as a clean stop), the SID pin and retarget, installing-user resolution from the elevated token and the `--config` owner, the elevated default and the Windows privilege guidance, the cross-scope refusal, the machine directory and the service log.
  - Updates the backend-selection requirement.
- `role-ownership-shutdown`:
  - The machine-tier lock extends to Windows (`%ProgramData%\iroh-tunnel`).
  - The service stop request is a graceful stop signal like SIGINT/SIGTERM.
- `frontend-independent-management`: the "Windows remains user-only" scenario is replaced by Windows system scope under the same no-elevation and no-fallback rule.
- `install-release-packaging`: the Windows installer sets a safe owner and access on the install directory it creates.
- `self-update`: the Windows locked-executable message adds system-service stop guidance.

## Impact

- **Code:**
  - `crates/host/src/service/`: a new Windows system backend; `is_elevated`, `policy`, `install`, `platform`, `system` (installing user), `pin` and the trust module gain Windows variants.
  - `crates/host/src/ownership.rs`: the Windows machine tier.
  - The runtime entry point: running under the Service Control Manager with stop mapped to the shared shutdown.
  - `install.ps1`, plus the update locked-executable message.
- **Dependencies:** `windows-service` (new, Windows only); `windows-sys` becomes a direct Windows-only dependency (already in `Cargo.lock`).
- **CLI/API:** `--system` works on Windows. The elevated default changes (breaking). The exit code for unprivileged `--system` on Windows changes from 5 to 3.
- **Security:** a new SYSTEM-privileged surface. Mitigations are the trust check on config, state and binary, the protected machine directory, and fail-closed behaviour. A documented residual gap: a binary replaced between CLI checks is not caught on a boot or recovery start.
- **Testing and CI:** E2E on the existing elevated Windows x64/ARM64 runners. Before the trust code is finalised, these runners must confirm four facts: the default `%LOCALAPPDATA%` ACL, the token user under UAC, the effect of create-subdirectory on `C:\`, and the `Program Files` install directory ACL.
- **Docs:** `docs/services.md` and `docs/troubleshooting.md` (Windows system scope, the residual gap), and the CHANGELOG (breaking elevated default).
- **Credit:** reused SCM dispatcher and stop handling from #67 carry `Co-authored-by: 23rd`.
