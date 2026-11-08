## ADDED Requirements

### Requirement: Windows system service registration
The system SHALL install `--system` on Windows as an own-process Windows Service named `iroh-tunnel-<role>` (plus the test-only suffix), with automatic start and the LocalSystem account. Its command line SHALL be `"<binary>" <role> run --config "<config>"`, and its service environment SHALL carry the pinned state directory and the pinned SID.

#### Scenario: Service definition
- **WHEN** `serve service install --system` succeeds on Windows
- **THEN** a service `iroh-tunnel-serve` exists with automatic start, LocalSystem account and the quoted `serve run --config` command line, and it is running

### Requirement: Windows system install replaces and rolls back
The system SHALL make a Windows system install replace an existing service of the role. A failure after the service is created SHALL delete it, so nothing stays half-registered.

#### Scenario: Reinstall
- **WHEN** `serve service install --system` runs while `iroh-tunnel-serve` exists for the same owner
- **THEN** one `iroh-tunnel-serve` service exists afterwards, carrying the new definition

#### Scenario: Start failure rolls back
- **WHEN** the newly created service fails to start during install
- **THEN** the command fails and no `iroh-tunnel-serve` service remains

### Requirement: Windows service recovery and exit mapping
The system SHALL map the shared restart policy onto service recovery actions:
- restart after the policy delay, with the failure count reset after one day;
- no actions when the policy is disabled;
- applied to non-crash failures as well.

A run that ends because the role is already running (exit code 6) SHALL be reported to the Service Control Manager as a clean stop, so recovery does not fire. Any other error SHALL be reported as a service-specific failure.

#### Scenario: Role already running
- **WHEN** the system service starts while a user process holds the machine lock shared
- **THEN** the service log says the role is already running, the service is reported stopped without error, and no recovery restart follows

#### Scenario: Crash recovery
- **WHEN** the service process exits with a failure other than exit code 6
- **THEN** the Service Control Manager restarts it after 5 seconds

### Requirement: Windows service stop drains
The system SHALL treat the Service Control Manager's stop, shutdown and pre-shutdown requests as a graceful stop under the shared drain contract. While draining it SHALL report stop-pending with a wait hint covering the drain budget and the watchdog, and it SHALL advance the checkpoint until exit.

#### Scenario: Stop removes the status file
- **WHEN** `service stop --system` runs against a running system service
- **THEN** the service reaches stopped and the role's status snapshot is removed

#### Scenario: Stop pending while draining
- **WHEN** the service status is queried while a stop request is draining an open connection
- **THEN** it reports stop-pending with a wait hint of at least 13 seconds

### Requirement: Windows service reports running only after startup checks
The system SHALL report a Windows system service as running only after the role lock is claimed and the config has passed the trust check and loaded. A failure before that point SHALL end the service with a failure status and SHALL NOT be reported as running.

#### Scenario: Invalid config at start
- **WHEN** the pinned config is invalid TOML and the service starts
- **THEN** the service never reports running, the log contains `failed to parse`, and the config's owner and DACL are unchanged

### Requirement: Windows service start requires the service context and a pin
The system SHALL fail with a configuration error (exit code 2) when `IROH_TUNNEL_SCM=1` is set but the process was not started by the Service Control Manager, and SHALL NOT fall back to a foreground run. A Windows system service started without a parseable `IROH_TUNNEL_TRUSTED_SID` SHALL fail with a permission error telling the operator to re-install with `service install --system`.

#### Scenario: Service marker outside the SCM
- **WHEN** `serve run` is started from a terminal with `IROH_TUNNEL_SCM=1`
- **THEN** it exits with code 2 and does not serve

#### Scenario: Service without a pin
- **WHEN** the system service environment has no `IROH_TUNNEL_TRUSTED_SID`
- **THEN** the service log contains `service install --system` and the service does not serve

### Requirement: Windows system service never writes a generated key
The system SHALL, under a Windows system service, fail with a configuration error telling the user to run `config keygen` as themselves when the config has no `secret_key`, and SHALL NOT write the user's config.

#### Scenario: Config without a key
- **WHEN** the pinned config has an empty `secret_key` and the service starts
- **THEN** the service log contains `config keygen`, the service does not serve, and the config file is unchanged

### Requirement: Windows system verbs on a missing service
The system SHALL fail Windows `service start|stop|status --system` for a role without a system service with service exit code 5 and `service install --system` guidance. It SHALL treat stop of an already-stopped service and uninstall of an absent service as success, and SHALL read service state from structured Service Control Manager status, never from localized text.

#### Scenario: Not installed
- **WHEN** `serve service start|stop|status --system` runs from an elevated terminal with no `iroh-tunnel-serve` service
- **THEN** each exits with code 5 and stderr contains `service install --system`

#### Scenario: Idempotent stop and uninstall
- **WHEN** `serve service stop --system` runs against a stopped service, and `serve service uninstall --system` runs with no service
- **THEN** both succeed

### Requirement: Windows retarget confirmation
The system SHALL apply the retarget confirmation rule to Windows system services by comparing the pinned SID. A service whose pin cannot be read SHALL count as an unknown owner.

#### Scenario: Declined retarget on Windows
- **WHEN** bob installs over alice's system service from an elevated terminal and answers `n`
- **THEN** exit code is 5, stderr contains `Retarget` and alice's account name, and the service definition and pin are unchanged

#### Scenario: Approved retarget on Windows
- **WHEN** bob answers `y`
- **THEN** the service pins bob's SID and config, and both users' configs are kept

#### Scenario: Same user again on Windows
- **WHEN** alice installs again over her own system service
- **THEN** install succeeds without a prompt

### Requirement: Windows installing user
The system SHALL identify the installing user of a Windows system install from the elevated token's user SID. Default paths SHALL be that user's known `%LOCALAPPDATA%` folders, resolved from the token and not from the environment.

#### Scenario: Elevated user installs with defaults
- **WHEN** alice runs `serve service install` from an elevated terminal without `--config`
- **THEN** the registration pins alice's `%LOCALAPPDATA%` config path and her SID

### Requirement: Windows token user and --config owner must agree
The system SHALL fail a Windows system install with a configuration error (exit code 2) naming both accounts when the `--config` directory is owned by a user SID other than the token user. An owner that is the Administrators group, SYSTEM or TrustedInstaller SHALL defer to the token user.

#### Scenario: Config owned by another user
- **WHEN** alice installs with `--config` in a directory owned by bob
- **THEN** exit code is 2, the message names both accounts, and nothing is registered

#### Scenario: Config directory owned by Administrators
- **WHEN** the `--config` directory is owned by the Administrators group
- **THEN** the token user is pinned and no mismatch is reported

### Requirement: Windows cross-scope registration is refused
The system SHALL, on Windows, refuse `service install --system` when the role has a user task, and refuse `service install --user` when the role has a system service. The refusal SHALL be a service error (exit code 5) naming the `service uninstall` command for the other scope, and SHALL register nothing.

#### Scenario: User task exists
- **WHEN** `serve service install --system` runs while the `iroh-tunnel-serve` user task exists
- **THEN** exit code is 5, stderr contains `service uninstall --user`, and no service is created

#### Scenario: System service exists
- **WHEN** `serve service install --user` runs while the `iroh-tunnel-serve` service exists
- **THEN** exit code is 5, stderr contains `service uninstall --system`, and no task is created

### Requirement: Windows machine directory
The system SHALL resolve the Windows machine directory `%ProgramData%\iroh-tunnel` from the known folder. An elevated process SHALL create it with a protected DACL granting SYSTEM and Administrators full access and Users read access. An existing directory that fails the trust evaluation for SYSTEM, Administrators and TrustedInstaller SHALL be refused and never repaired. Uninstall SHALL leave the directory in place.

#### Scenario: Directory pre-created by a user
- **WHEN** a non-administrator created `%ProgramData%\iroh-tunnel` before the first system install
- **THEN** install exits with code 3 naming the directory and its owner, no service is registered, and the directory's DACL is unchanged

#### Scenario: Directory made untrusted after install
- **WHEN** an untrusted SID gains write access to `%ProgramData%\iroh-tunnel` and the service starts
- **THEN** the service ends with a failure status without writing a log file there, and the directory's DACL is unchanged

### Requirement: Windows system service log
The system SHALL, when running as a Windows system service, write its log to `%ProgramData%\iroh-tunnel\logs\<role>.log`, capped in size with one rotated file.

#### Scenario: Log available after a refusal
- **WHEN** the system service refuses an unsafe config
- **THEN** `%ProgramData%\iroh-tunnel\logs\serve.log` contains the permission error

### Requirement: Windows user background task registration
The system SHALL, for Windows user scope, replace and start a per-role logon task with least privilege and restart-on-failure.

#### Scenario: Task execution policy
- **WHEN** `iroh-tunnel-<role>` is registered
- **THEN** it uses InteractiveToken, IgnoreNew, no execution limit, battery-insensitive settings and restart interval 5 seconds with count 999, then runs immediately

#### Scenario: Task definition
- **WHEN** the task XML is rendered
- **THEN** it contains `<LogonTrigger>`, `InteractiveToken`, `LeastPrivilege` and not `HighestAvailable`

## MODIFIED Requirements

### Requirement: Install scope selection
The system SHALL honor an explicit install scope. Otherwise an elevated process (root, or an elevated Administrator token on Windows) defaults to system, interactive non-elevated users choose a scope, and non-interactive non-elevated users default to user scope. The same rules apply on every supported platform.

#### Scenario: Conflicting explicit scope flags
- **WHEN** both `--user` and `--system` are passed to the CLI
- **THEN** argument parsing rejects the combination

#### Scenario: Root installs without a scope prompt
- **WHEN** root runs `serve service install` with no scope flag
- **THEN** the service is installed with system scope and no scope question is printed

#### Scenario: Elevated Windows terminal installs without a scope prompt
- **WHEN** `serve service install` runs with no scope flag from an elevated Windows terminal
- **THEN** a system service is installed and no scope question is printed

#### Scenario: Interactive user is offered both scopes
- **WHEN** a non-elevated user runs `service install` with no scope flag on a terminal
- **THEN** a prompt offering `user` and `system` is written to stderr, and an empty answer, `u` or `user` selects user scope while `s` or `system` selects system scope

#### Scenario: Non-interactive non-root default
- **WHEN** a non-elevated process with no terminal runs `service install` with no scope flag
- **THEN** a user service is installed without any prompt

### Requirement: System scope requires privilege without elevation or fallback
The system SHALL refuse non-elevated system-scope operations with permission exit code 3 and an explicit rerun command, without self-elevation, user-scope fallback or mutation. The rerun guidance SHALL be `sudo <command>` on Unix, and SHALL tell the user to run `<command>` from an elevated terminal (Run as administrator) on Windows. Install guidance SHALL include the absolute config path.

#### Scenario: Unprivileged system install
- **WHEN** a non-root user runs `serve service install --system --config <path>`
- **THEN** the command exits with code 3, stderr contains `sudo` and `serve service install --system` and the absolute config path, and no registration file exists

#### Scenario: Unprivileged system verbs
- **WHEN** a non-root user runs `serve service uninstall|start|stop|restart --system`
- **THEN** each exits with code 3 and stderr contains `sudo iroh-tunnel serve service <verb> --system`

#### Scenario: Non-elevated system install on Windows
- **WHEN** `serve service install --system --config <path>` runs from a non-elevated Windows terminal
- **THEN** the command exits with code 3, stderr contains `Run as administrator`, `serve service install --system` and the absolute config path, and no service exists

#### Scenario: Non-elevated system verbs on Windows
- **WHEN** `serve service uninstall|start|stop|restart --system` runs from a non-elevated Windows terminal
- **THEN** each exits with code 3, stderr contains `Run as administrator` and `iroh-tunnel serve service <verb> --system`, and nothing is changed

### Requirement: Default scope for verbs on an existing service
The system SHALL, for uninstall, start, stop, restart and status without a scope flag, use system scope when the process is elevated and user scope otherwise, on every supported platform, and SHALL NOT prompt for these verbs.

#### Scenario: Root without a flag
- **WHEN** root runs `serve service restart` with no scope flag
- **THEN** the system-scope service is restarted

#### Scenario: Non-root without a flag
- **WHEN** a non-root user runs `serve service stop` with no scope flag
- **THEN** the user-scope service is targeted and no privilege is required

#### Scenario: Elevated Windows terminal without a flag
- **WHEN** `serve service stop` runs with no scope flag from an elevated Windows terminal
- **THEN** the system service is targeted, not the user task

### Requirement: Linux backend chosen at runtime
The system SHALL use the systemd backend on Linux when `/run/systemd/system` exists or PID 1's executable ends in `systemd`, and otherwise the BusyBox/SysV init backend. macOS SHALL use launchd. Windows SHALL use a Task Scheduler task for user scope and a Windows Service for system scope. Other platforms SHALL fail with a message that service management is supported only on Linux, macOS and Windows.

#### Scenario: Container without systemd
- **WHEN** a service is installed on a Linux host without `/run/systemd/system`
- **THEN** an executable `/etc/init.d/S96iroh-tunnel-<role>` script is installed

#### Scenario: Windows scopes
- **WHEN** a role is installed on Windows with `--user` and, separately, with `--system`
- **THEN** user scope registers a Task Scheduler task and system scope registers a Windows Service

### Requirement: Registration pins the owner
The system SHALL record in every system registration a single-line pin record:
- on Unix, the comment `iroh-tunnel-pin config=<escaped path> uid=<n>` and the environment `IROH_TUNNEL_TRUSTED_UID=<uid>`;
- on Windows, `iroh-tunnel-pin config=<escaped path> sid=<SID>` in the service registration and `IROH_TUNNEL_TRUSTED_SID=<SID>` in the service environment.

It SHALL treat a registration without a parseable pin as an unknown owner.

#### Scenario: Pin in the script
- **WHEN** alice's system service is installed on BusyBox
- **THEN** the script contains her canonical config path and `IROH_TUNNEL_TRUSTED_UID=<her uid>`

#### Scenario: Pin in the Windows service
- **WHEN** alice's system service is installed on Windows
- **THEN** the registration carries her canonical config path and SID, and the service environment contains `IROH_TUNNEL_TRUSTED_SID=<her SID>`

### Requirement: Windows install refuses a non-default state directory
The system SHALL refuse a Windows user-scope install when `IROH_TUNNEL_STATE_DIR` selects a directory other than the default (compared case- and separator-insensitively). The refusal SHALL be a configuration error (exit code 2) naming the variable, and SHALL register nothing.

#### Scenario: Custom state directory
- **WHEN** `IROH_TUNNEL_STATE_DIR` points elsewhere and `service install --user` runs
- **THEN** exit code is 2, stderr mentions `IROH_TUNNEL_STATE_DIR`, and no task exists

### Requirement: Windows restart waits for the old process
The system SHALL, on Windows, restart in either scope by stopping the task or service, waiting up to 15 seconds for the role's lock to be released, and then starting it. If the old process has not exited by then, it SHALL fail with a service error and SHALL NOT start a second instance.

#### Scenario: Slow stop
- **WHEN** the role is still running 15 seconds after `schtasks /End`
- **THEN** restart fails saying the task did not stop and does not start again

#### Scenario: System service restart
- **WHEN** `service restart --system` runs against a running service
- **THEN** the old process exits after draining and a new process starts

## REMOVED Requirements

### Requirement: Windows user background task

**Reason**: It stated that Windows supports user tasks only and that system scope fails with exit code 5. This change adds Windows system scope.

**Migration**: The user-task behaviour carries over unchanged as "Windows user background task registration". System scope now follows "Windows system service registration". Callers that relied on exit code 5 for `--system` on Windows now get a system service when elevated, or exit code 3 when not.
