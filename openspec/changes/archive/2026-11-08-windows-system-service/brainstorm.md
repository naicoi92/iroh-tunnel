## Design Summary

### Intent

Let a Windows operator run `iroh-tunnel <role> service install --system` and get what Linux and macOS operators already get: a role that runs in the background from boot without anyone logged in, survives logoff, restarts on failure, and drains gracefully when stopped. Today `--system` on Windows exits with code 5 because SM-04 (#79) deferred Windows system scope. The reason was the missing trust model, not any objection to Windows Services. Community PR #67 (closed in favour of #184) prompted the owner to revisit that deferral.

### Goals

- Windows system scope follows the **shared user/system scope model** already confirmed for Unix (SM-01..SM-05, #77, #79, #134). It adds no Windows-only policy.
- The service runs as **LocalSystem** (#77: a system service runs as root/Administrator, and whoever installs it owns it).
- It reads the **installing user's own config and state**, pinned at install, so the user can keep editing their config (#79 constraint).
- It trusts **SYSTEM, Administrators and the one pinned user** for config and state, checks **at install and at every start**, fails closed with a message naming the path, owner and offending access, and **never changes ACLs**.
- It trusts **only SYSTEM, Administrators and TrustedInstaller** for the **service binary** and every directory above it, and fails closed otherwise (owner decision in this brainstorm). The pinned user is trusted for config but not for the binary, because a user-writable binary lets any non-elevated process of that user become SYSTEM. The binary is checked at install and by the CLI `start`/`restart` verbs. A start by the Service Control Manager at boot or through recovery actions is not checked, because a check inside the already-running binary protects nothing. This gap is documented.
- A **machine-wide location**, `%ProgramData%\iroh-tunnel`, plays the role `/run/iroh-tunnel` and `/var/log` play on Unix. It holds the machine-tier role lock, which keeps a role's user task and system service from running together in either start order, and the system service's log file. The service creates it with its own ACL: SYSTEM and Administrators write, Users read. If it already exists with an owner or write access outside SYSTEM/Administrators, that is fail closed. This ACL is set only on our own directory, never on the user's files.
- `install.ps1` sets the owner of the install directory it creates to Administrators and drops any inherited ACE that would let the creating user write it, but only if measurement shows such an ACE exists. It touches only that directory.
- On Windows, installing one scope while the same role is registered in the other scope is refused, with guidance to run `service uninstall --user` or `--system` first. Task Scheduler cannot exempt exit 6 from its restart policy, so a losing user task would otherwise retry up to 999 times after every logon.
- Stop drains through the OS service stop request. That is an OS signal of the same kind as SIGTERM, not a new stop command or control channel, so RO-03 holds.
- Scope selection on Windows follows the shared rules. Windows gets a real elevation check. An elevated caller without a scope flag defaults to `--system`, for install and for the other verbs (**breaking change**, noted in the CHANGELOG). A non-elevated interactive install asks user/system. `--system` without elevation exits 3 with "rerun from an elevated terminal" guidance instead of exit 5.
- System-scope semantics shared with Unix stay the same: retarget confirmation for a different or unknown owner, `--config` owner and caller must agree, no self-elevation and no fallback to user scope.

### Non-goals

- Changing the `--user` mechanism (Task Scheduler task, `schtasks /End`). Graceful stop for `--user` is spike #185.
- Adding the binary trust rule to Linux/macOS system scope: tracked in #186.
- Running the service under any account other than LocalSystem (user account with a stored password, virtual or managed service accounts).
- Moving the user's config or state into ProgramData, or adjusting any ACL on the user's files. Only our own machine directory and install directory get an ACL we set.
- A Windows Event Log source, GUI or tray integration.
- Support for the standalone `serve`/`access` binaries' `service` subcommand (still refused, as on Unix).

### Anti-goals (a failure even if it works)

- Any path where a non-elevated process can make SYSTEM read attacker-chosen config or run an attacker-chosen binary: replacement, reparse point, junction, hard-link games, or a writable parent directory.
- Behaviour that depends on localized Windows text (the spec already forbids this for the user-scope backend).
- A system service and the same role's user task running at once. The machine-tier lock must refuse the second instance in either start order.
- Silently installing to a different scope than the shared rules choose.

### Constraints

- MSRV 1.91; `unsafe` only where justified, each block with `// SAFETY:` (RS-12); existing CI gates, including Windows x64 and ARM64 runners.
- Exit classification stays the same (permission 3, configuration 2, service 5, ...).
- Owner decision: the elevated default is `--system` (breaking), accepted with a CHANGELOG note.
- Owner decision: implementation uses the `windows-service` crate for service management and runtime, and `windows-sys` for the trust check (approach A below).
- New dependency: `windows-service` (cfg windows only). `windows-sys` is already in `Cargo.lock` transitively and becomes a direct cfg-windows dependency.
- PR #67's SCM dispatcher, stop handling and failure actions are the reference. Commits that reuse them carry `Co-authored-by: 23rd <23rd@vivaldi.net>`.

### Known facts and assumptions to verify on real Windows

These are inferred from Microsoft documentation, not measured. If one is false, the trust design changes before implementation proceeds:

1. The default ACL of a user's `%LOCALAPPDATA%` tree grants write only to SYSTEM, Administrators and that user, so a default install passes the config/state check.
2. A UAC-elevated token keeps the user's own SID as its user. Installing from "Run as administrator" therefore identifies the user. A different admin account (over-the-shoulder credentials) is detected because it disagrees with the `--config` owner.
3. Authenticated Users' create-subdirectory right on `C:\` does not let them replace or rename another principal's directory, so it is not a disqualifying write on an ancestor.
4. `ProgramFiles\iroh-tunnel` as created by `install.ps1` passes the binary check by default. The suspected exception is a CREATOR OWNER-derived ACE for the creating admin user; if it is present, `install.ps1` removes it, as decided above.

## Alternatives Considered

### Alternative A: `windows-service` crate for management and runtime, `windows-sys` for trust (chosen)
- **Approach**: create, configure, start, stop, query and delete through the crate's Service Control Manager API; run the role under its service dispatcher; implement the DACL and reparse-point checks directly on `windows-sys`.
- **Pros**: structured states and Win32 error codes, never localized text; no `unsafe` in our management code; reuses #67; widely used crate (Mullvad).
- **Cons**: one new dependency; a thin seam is needed so management logic can be unit-tested off Windows, unlike the user backend, which goes through `Native` command runs.
- **Why chosen**: the most reliable state handling for the least `unsafe`; the seam is small.

### Alternative B: `sc.exe` through the existing `Native` command seam
- **Approach**: mirror the `schtasks` backend and drive `sc create/config/failure/start/stop/query/delete`; the crate is used only for the runtime dispatcher.
- **Pros**: reuses the existing test seam; looks like the other backends.
- **Cons**: `sc` output is localized and must be parsed; `binPath=` quoting is error-prone; failure actions and stop timing are hard to read back.
- **Why not chosen**: parsing localized text is a long-term defect source the spec already avoids.

### Alternative C: raw `windows-sys` for everything
- **Approach**: call `CreateServiceW`, `StartServiceCtrlDispatcherW` and the rest directly.
- **Pros**: one dependency.
- **Cons**: a large `unsafe` surface for no capability A lacks.
- **Why not chosen**: cost and risk without benefit.

### Also rejected earlier (scope-level)
- **Keep deferring** (`--system` unsupported): no boot-time start and no drain for Windows operators.
- **Service under the user's account**: needs a stored password or gMSA.
- **Virtual account with config in ProgramData**: breaks SM-02 path pinning and "never chown/chmod".

## Agreed Approach

Alternative A, applied as a one-to-one mapping of the Unix system scope onto Windows:

| Shared rule | Unix today | Windows |
|---|---|---|
| Registration | systemd / launchd / BusyBox | Windows Service, automatic start |
| Runtime account | root | LocalSystem |
| Missing privilege | exit 3, `sudo` rerun line | exit 3, elevated-terminal rerun line |
| Installing user | `SUDO_USER`, then `--config` owner, then prompt | elevated token's user, cross-checked with the `--config` owner; mismatch is exit 2 naming both |
| Pinned paths | user's canonical config and state | user's canonical `%LOCALAPPDATA%` config and state |
| Pin | `iroh-tunnel-pin` + `IROH_TUNNEL_TRUSTED_UID` | equivalent pin carrying the user's SID, readable back for retarget decisions |
| Trusted for config/state | root + pinned UID | SYSTEM + Administrators + pinned SID |
| Trusted for binary | (not checked; #186) | SYSTEM + Administrators only |
| Path check | no-follow descent, owner/mode per handle | descent refusing every reparse point, owner and DACL per handle |
| Config read | from the verified handle | from the verified handle |
| Stop | SIGTERM, then drain | OS service stop request, then drain |
| Restart on failure | shared `RestartPolicy` | shared `RestartPolicy` as service recovery actions |
| Retarget | y/yes confirmation | same |
| Role lock (user/system exclusion) | machine tier `/run/iroh-tunnel/<role>.lock` (system exclusive, users shared) | machine tier in `%ProgramData%\iroh-tunnel` with the same exclusive/shared semantics |
| Service log | journald / launchd log / `/var/log` | log file in `%ProgramData%\iroh-tunnel` |
| Restart loop on "already running" (exit 6) | `RestartPreventExitStatus=6` | exit 6 reported to the Service Control Manager as a clean stop, so no recovery action fires |
| Other scope already registered | (allowed; the lock refuses at start) | install refused, uninstall-first guidance |

The user-visible CLI is unchanged apart from Windows now accepting `--system` and the new elevated default.

## Key Decisions

1. Windows system scope is implemented now, mirroring the Unix model (owner, this thread).
2. Elevated without a flag means `--system` on Windows too; breaking change noted in the CHANGELOG (owner).
3. The service binary must be writable only by SYSTEM/Administrators, fail closed, Windows only in this change; Unix in #186 (owner).
4. Implementation approach A (owner).
5. `--user` is unchanged; its drain is #185 (owner).
6. Assumptions 1-4 above are verified on real Windows runners before or during implementation; a false one returns to design rather than being worked around.
7. Machine directory `%ProgramData%\iroh-tunnel` for the machine lock and the service log, created with our own ACL, fail closed on an untrusted existing one (owner).
8. Binary checked at install and on CLI `start`/`restart`; boot and recovery starts unchecked and documented (owner).
9. `install.ps1` fixes the owner and inherited ACE of its own install directory if measurement shows the creating user can write it (owner).
10. Cross-scope install on Windows is refused with uninstall-first guidance (owner).
11. Agent's call: if a config or its directory is owned by a group (Administrators) or SYSTEM, it names no person; the installing user is then the elevated token's user, and this is not a mismatch.
12. Agent's call: `update` keeps DIST-02 (never stops a running tunnel); when a Windows system service holds the binary, the message adds `service stop --system` guidance.
13. Agent's call: E2E for the system service runs on the existing Windows CI runners, which execute elevated. A non-elevated refusal path is covered where a restricted token can be obtained, otherwise by unit tests of the decision logic.

## Open Questions

- How the running process learns it was started by the Service Control Manager (a hidden `run` argument as in #67, or detection) is left to design; it must not change the documented `run` CLI.
- Where the pin lives (service description, environment block or a registry value) is left to design, provided it can be read back without localized text.
- Log file size bound and rotation for the service log in `%ProgramData%\iroh-tunnel` are left to design; it must be bounded.
- How long the stop request may take before the Service Control Manager is told the service has stopped: it must cover the shared 3 s drain budget plus the 10 s watchdog. Exact wait hints are left to design.
