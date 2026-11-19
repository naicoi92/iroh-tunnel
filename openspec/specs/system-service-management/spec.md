# system-service-management Specification

## Purpose

Define how the `service` commands register, start, stop and remove a role as a user or system service on Linux (systemd, BusyBox), macOS (launchd) and Windows, including scope selection, privilege and path-pinning rules. The command surface is owned by `cli-entrypoints`, and Windows path trust checks by `windows-service-trust`.

## Requirements

### Requirement: Service management refused for standalone role binaries
The system SHALL refuse the `service` subcommand of a standalone role binary with a service error directing the user to `iroh-tunnel <role> service ...`.
Evidence: src/app.rs:218-224
Confidence: inferred

#### Scenario: Standalone binary
- **WHEN** `service` is invoked on a standalone serve or access binary
- **THEN** the command fails with a message that service management is not yet supported by the standalone binary and names the `iroh-tunnel <role> service` form

### Requirement: Installed services pin absolute config and state paths
The system SHALL write the absolute config path and absolute state directory into the registration (state via `IROH_TUNNEL_STATE_DIR`), resolving a relative `--config` against the current directory, so the service runs identically regardless of working directory or home. Without overrides the paths SHALL be the defaults of the service's tier.
Evidence: src/app.rs:238-250,306-330,429-440, src/service/systemd.rs:46-90, src/service/launchd.rs:84-140, src/service/busybox.rs:47-120, tests/service_user.rs, tests/service_system.rs

#### Scenario: Install from another working directory
- **WHEN** a user service is installed from `/` with a config path containing a space
- **THEN** the registration contains the absolute config path and state directory and the service writes its status file under that state directory

#### Scenario: State directory override
- **WHEN** `IROH_TUNNEL_STATE_DIR` is set at install time
- **THEN** that directory (made absolute) is the pinned state directory; otherwise the tier's default is pinned

### Requirement: Install refuses a config that could only fail at start
The system SHALL check that the pinned config exists and is valid for the role before registering a service in either tier, and SHALL register nothing and exit with code 2 when it is not.
Evidence: src/app.rs:318-330, src/app.rs:440-470, tests/service_user.rs (install_without_config_fails_with_guidance_and_registers_nothing)

#### Scenario: Missing config
- **WHEN** `serve service install --config <nonexistent path>` is run
- **THEN** the command exits with code 2, stderr names the config path, and no registration exists

### Requirement: Registration write permission failures keep scope and account
The system SHALL, when the OS denies writing a user-tier registration, fail with a permission error (exit code 3) saying that the tier and account were not changed, nothing was registered, and a system service needs the command re-run with elevation; it SHALL NOT retry in another tier or account.
Evidence: src/service/mod.rs:296-340, tests/service_user.rs (denied_registration_fails_with_guidance_and_keeps_scope)

#### Scenario: Read-only home
- **WHEN** the user's home directory is mode 0500 and `service install` runs as that user
- **THEN** the command exits with code 3, stderr mentions permission and that the tier is not changed automatically, and no registration file exists

### Requirement: Registration files are written atomically with fixed permissions
The system SHALL write registrations to a temporary sibling file and rename it into place, creating the parent directory, with mode 0755 for extension-less files (init scripts) and 0644 otherwise, independent of umask.
Evidence: src/service/mod.rs:310-345
Confidence: inferred

#### Scenario: Install writes a unit
- **WHEN** a registration is written
- **THEN** the final file appears by rename with mode 0644 (or 0755 for an init script), and a failed write removes the temporary file

### Requirement: Linux backend chosen at runtime
The system SHALL use the systemd backend on Linux when `/run/systemd/system` exists or PID 1's executable ends in `systemd`, and otherwise the BusyBox/SysV init backend. macOS SHALL use launchd. Windows SHALL use a Task Scheduler task for the user tier and a Windows Service for the system tier. Other platforms SHALL fail with a message that service management is supported only on Linux, macOS and Windows.

#### Scenario: Container without systemd
- **WHEN** a service is installed as root on a Linux host without `/run/systemd/system`
- **THEN** an executable `/etc/init.d/S96iroh-tunnel-<role>` script is installed

#### Scenario: Windows scopes
- **WHEN** a role is installed on Windows from a non-elevated terminal and, separately, from an elevated one
- **THEN** the non-elevated install registers a Task Scheduler task and the elevated install registers a Windows Service

### Requirement: systemd unit content and lifecycle
The system SHALL install a role systemd unit with pinned config/state paths, restart-on-failure after 5 seconds except exit 6, and reload, enable and restart it; user-tier operations SHALL use `systemctl --user`. A re-install SHALL restart the unit even when it is running.
Evidence: crates/host/src/service/backend/systemd.rs (unit tests), crates/host/src/service/types.rs (RestartPolicy tests)

#### Scenario: Unit registration shape
- **WHEN** a unit is generated
- **THEN** `iroh-tunnel-<role>.service` lives under `/etc/systemd/system` or `~/.config/systemd/user`, runs `<binary> <role> run --config <config>`, sets `IROH_TUNNEL_STATE_DIR` and `IROH_TUNNEL_SERVICE=1`, and is wanted by `default.target`

#### Scenario: Unit text
- **WHEN** a serve unit is rendered for `/usr/bin/iroh-tunnel` and config `/etc/cfg`
- **THEN** it contains `ExecStart=/usr/bin/iroh-tunnel serve run --config /etc/cfg`, `Restart=on-failure`, `RestartSec=5` and `RestartPreventExitStatus=6`

#### Scenario: Re-install over a running unit
- **WHEN** the service is installed again in the tier that already holds it
- **THEN** the unit is restarted so the new registration takes effect

### Requirement: systemd quoting and escaping
The system SHALL quote unit-line words that contain spaces or special characters, double `%` everywhere, and double `$` in `ExecStart` but keep it literal in `Environment=`.
Evidence: src/service/systemd.rs:96-125, src/service/systemd.rs tests (unit_pins_state_dir_and_quotes_paths_with_spaces, dollar_is_doubled_in_exec_start_but_literal_in_environment)

#### Scenario: Path with a space and a dollar sign
- **WHEN** the config path is `/home/$u/serve.toml`
- **THEN** `ExecStart` contains `"/home/$$u/serve.toml"` and the state `Environment=` line contains `/home/$u/state` unchanged

### Requirement: systemd uninstall and status semantics
The system SHALL treat stop and disable during uninstall as best-effort, remove the unit file, daemon-reload (failure reported), and print `Uninstalled <unit>`; `status` SHALL print systemctl's output and not fail when the service is stopped.
Evidence: src/service/systemd.rs:172-215
Confidence: inferred

#### Scenario: Uninstall a not-loaded unit
- **WHEN** uninstall runs and the unit is not loaded
- **THEN** the stop/disable failures are ignored and the unit file is removed

### Requirement: BusyBox init script content and lifecycle
The system SHALL install and restart a mode-0755 BusyBox role init script with pinned config/state, start/stop/restart/status support, and a SIGTERM then 5-second SIGKILL stop policy that applies only to a process confirmed as the daemon by "BusyBox init script verifies process identity" or "BusyBox init script recognises legacy PID files". The script's PID file SHALL hold the daemon's PID on its first line and, on the second, the launched `DAEMON` path followed by a single `.` sentinel so trailing characters of the path survive.
Evidence: crates/host/src/service/backend/busybox_script.rs (render and script execution tests), crates/host/src/service/backend/busybox.rs, tests/service_system.rs

#### Scenario: BusyBox paths and stopped status
- **WHEN** a BusyBox role script is generated
- **THEN** it uses `/etc/init.d/S96iroh-tunnel-<role>`, `/var/run/iroh-tunnel-<role>.pid`, `/var/log/iroh-tunnel-<role>.log`, exports `IROH_TUNNEL_STATE_DIR`, runs `<binary> <role> run --config <config>` in background and returns status 3 when stopped

#### Scenario: Script verbs
- **WHEN** the script is rendered
- **THEN** it begins with `#!/bin/sh`, embeds DAEMON, CONFIG, PIDFILE and LOG, and wires `start`, `stop`, `restart` (stop then start) and `status`

#### Scenario: Start records the launched executable
- **WHEN** `start` launches the daemon
- **THEN** the PID file's first line is the daemon's PID and its remaining content is the `DAEMON` path followed by `.`

#### Scenario: Stop of the running daemon
- **WHEN** the PID file names the daemon this script launched and `stop` runs
- **THEN** the daemon receives SIGTERM, is force-killed only if it is still that daemon after 5 seconds, and the PID file is removed

#### Scenario: PID reused during the stop grace window
- **WHEN** the daemon ignores SIGTERM and, before the 5-second check, its PID no longer runs the recorded executable
- **THEN** no SIGKILL is sent to that PID

### Requirement: launchd registration and domains
The system SHALL install a labelled launchd role job in the domain of its tier, with pinned command and state paths, `IROH_TUNNEL_SERVICE=1`, RunAtLoad and restart on unsuccessful exit; the plist SHALL declare Apple's property-list document type. Loading SHALL issue `launchctl enable` for the job label in the target domain, then boot out the old job, then bootstrap the plist; a failure to enable SHALL fail the operation, while a failure booting out an old job SHALL be ignored.
Evidence: crates/host/src/service/backend/launchd.rs (format_plist_with, load)

#### Scenario: launchd registration shape
- **WHEN** a launchd job is registered
- **THEN** `dev.iroh-tunnel.<role>` uses `~/Library/LaunchAgents` with `gui/<uid>` in the user tier or `/Library/LaunchDaemons` with `system` in the system tier, XML-escaped arguments, `IROH_TUNNEL_STATE_DIR` and `IROH_TUNNEL_SERVICE=1`

#### Scenario: Plist document type
- **WHEN** a launchd plist is rendered
- **THEN** its document type declaration references `PropertyList-1.0.dtd` and does not reference `PropertyManifest-1.0.dtd`

#### Scenario: Load command order
- **WHEN** a launchd job is loaded
- **THEN** the commands issued are `enable <service-target>`, then `bootout <service-target>`, then `bootstrap <domain> <plist>`, in that order

#### Scenario: Re-install over a loaded job
- **WHEN** the service is installed while already loaded
- **THEN** the job label is enabled, the existing job is booted out, and the new plist is bootstrapped

#### Scenario: Enabling the label fails
- **WHEN** `launchctl enable` fails for the target
- **THEN** the operation fails reporting that command, and no bootstrap is attempted

#### Scenario: Booting out a job that is not loaded
- **WHEN** `launchctl bootout` fails because no job is registered
- **THEN** the failure is ignored, its output is not shown, and the load continues to bootstrap

### Requirement: launchd verbs and idempotent stop
The system SHALL start with `kickstart`, restart with `kickstart -k`, stop with `launchctl kill SIGTERM` treating "no process to signal" (exit 3) as success and printing that it is already stopped, and when the job is not loaded SHALL report that no service is loaded in the current tier's domain with an install hint that, for the other tier, says to run the command with or without elevation. A failed bootstrap SHALL print diagnostic guidance naming the domain's disable list among the things to inspect.
Evidence: crates/host/src/service/backend/launchd.rs, src/render.rs
Confidence: inferred

#### Scenario: Stop an already stopped job
- **WHEN** `service stop` runs and launchd reports no process to signal
- **THEN** the command succeeds and prints `<label> is already stopped.`

#### Scenario: Bootstrap fails
- **WHEN** a bootstrap fails during install
- **THEN** the guidance printed to stderr contains `launchctl print-disabled` with the target domain

### Requirement: Restart policy is shared across platforms
The system SHALL apply one default restart policy (restart on failure, 5 second delay) to every backend; launchd ignores the delay. The systemd unit SHALL exempt the role's "already running" exit status from restart. The launchd job SHALL restart on unsuccessful exit only and SHALL NOT carry an exit-status exemption, because launchd's restart condition cannot express one; a macOS job that exits because the role is already running in its tier is therefore respawned until it is booted out.
Evidence: crates/host/src/service/types.rs (RestartPolicy), crates/host/src/service/backend/systemd.rs (format_unit), crates/host/src/service/backend/launchd.rs (format_plist_with)

#### Scenario: Policy disabled
- **WHEN** a unit is rendered with restart-on-failure disabled
- **THEN** it contains `Restart=no`

#### Scenario: systemd exempts the refused-start status
- **WHEN** a systemd unit is rendered with the default policy
- **THEN** it contains `RestartPreventExitStatus=6`

#### Scenario: launchd carries no exit-status exemption
- **WHEN** a launchd plist is rendered with the default policy
- **THEN** its `KeepAlive` dictionary sets `SuccessfulExit` to false and contains no exit-status exemption, there being no launchd key that expresses one

### Requirement: Binary resolution for registrations
The system SHALL resolve the binary as the `iroh-tunnel` found on `PATH` and otherwise the absolute path of the current executable. A user-tier registration on systemd, launchd or Windows SHALL register that resolved path. A root-run registration on Linux or macOS SHALL register the binary chosen by `unix-service-binary-trust` ("System install registers a trusted binary or a managed copy"): the canonical resolved path when it passes the root-only binary check, otherwise the role's managed copy. A Windows system service SHALL register the resolved path, checked as `windows-service-trust` specifies.
Evidence: src/cli_service.rs (resolve_tunnel_executable_with tests), crates/host/src/service/install.rs

#### Scenario: Not on PATH
- **WHEN** `iroh-tunnel` is not on `PATH` (for example `cargo run`) and a user service is installed
- **THEN** the current executable's absolute path is registered

#### Scenario: Root-run registration from a user-writable binary
- **WHEN** a system service is installed on Linux or macOS and the resolved binary fails the root-only binary check
- **THEN** the registration names the role's managed copy instead of the resolved path

### Requirement: Test-only instance suffix
The system SHALL, only in builds with the `test-utils` feature, append the value of `IROH_TUNNEL_TEST_SERVICE_SUFFIX` to service, label and task names so test registrations do not collide with real services; production builds use no suffix.
Evidence: src/service/mod.rs:290-300, tests/service_user.rs (Registered::registration uses suffix)

#### Scenario: Suffix set in a test build
- **WHEN** the variable is `-e2e123` in a test-utils build
- **THEN** the registration is named `iroh-tunnel-serve-e2e123.service` (or the label/task equivalent)

### Requirement: User service lifecycle
The system SHALL, for a user-tier service, install and start, report status (running/stopped), stop, start, restart and uninstall through the host's per-session service manager (systemd user manager, LaunchAgent, logon task), with the same node identity across stop/start/restart and the registration removed on uninstall. The service SHALL start at the user's login and SHALL NOT be configured to outlive the login session; the system SHALL NOT enable or inspect systemd linger. Removal of the status file after stop SHALL hold for the systemd user manager and LaunchAgent; the Windows logon task follows "Windows user-tier stop terminates without draining" instead.
Evidence: tests/service_user.rs (user_service_lifecycle_uses_the_pinned_absolute_paths, opt-in via IROH_TUNNEL_E2E_SERVICE=1), src/app.rs:330-352

#### Scenario: Full lifecycle
- **WHEN** a systemd or launchd user service is installed, stopped, started, restarted and uninstalled
- **THEN** the status file reports the same node id after each start, is removed after stop, and the registration file no longer exists after uninstall

#### Scenario: Windows logon task lifecycle
- **WHEN** a Windows logon task is installed, stopped, started, restarted and uninstalled
- **THEN** the status file reports the same node id after each start, and the task no longer exists after uninstall

#### Scenario: No linger
- **WHEN** a user service is installed on a systemd host
- **THEN** no `loginctl` command is run

### Requirement: Status snapshot publishing under a system service is link-safe
The system SHALL, under a system service, write the status snapshot by re-verifying the state directory on every write against the system-tier owners, creating the temp file exclusively without following links and renaming it atomically, so a planted symlink is replaced rather than written through, and stale temp files or a snapshot left by `kill -9` do not block restart.
Evidence: src/service/trust.rs:260-340 (unit tests), tests/service_system.rs

#### Scenario: Planted symlink
- **WHEN** `serve-status.json` in the state directory is replaced by a symlink to another root-owned file and the service restarts
- **THEN** the symlink is replaced by a regular file and the target file content is unchanged

### Requirement: Windows install refuses a non-default state directory
The system SHALL refuse a Windows user-tier install when `IROH_TUNNEL_STATE_DIR` selects a directory other than the default (compared case- and separator-insensitively). The refusal SHALL be a configuration error (exit code 2) naming the variable, and SHALL register nothing.

#### Scenario: Custom state directory
- **WHEN** `IROH_TUNNEL_STATE_DIR` points elsewhere and `service install` runs from a non-elevated terminal
- **THEN** exit code is 2, stderr mentions `IROH_TUNNEL_STATE_DIR`, and no task exists

### Requirement: Windows verbs on a missing task
The system SHALL fail Windows user-tier start/stop/status for missing tasks with service exit 5 and install guidance, treat already-stopped stop and absent uninstall as success, and use ownership locks rather than localized task text to detect a running role.
Evidence: src/service/windows.rs:60-100, src/service/windows.rs:170-279, tests/service_user_windows.rs (start_without_an_installed_task_names_the_install_command)

#### Scenario: Not installed
- **WHEN** `serve service start|stop|status` runs from a non-elevated terminal with no task
- **THEN** each exits with code 5 and stderr contains `service install`

### Requirement: Windows restart waits for the old process
The system SHALL, on Windows, restart in either tier by stopping the task or service, waiting up to 15 seconds for the role's lock in that tier to be released, and then starting it. If the old process has not exited by then, it SHALL fail with a service error and SHALL NOT start a second instance.

#### Scenario: Slow stop
- **WHEN** the role is still running 15 seconds after `schtasks /End`
- **THEN** restart fails saying the task did not stop and does not start again

#### Scenario: System service restart
- **WHEN** `service restart` runs from an elevated terminal against a running service
- **THEN** the old process exits after draining and a new process starts

### Requirement: Windows access-denied guidance
The system SHALL turn an English "access is denied" failure while registering the Windows task into a permission error stating the scope and account were not changed and nothing was registered.
Evidence: src/service/windows.rs:170-195
Confidence: inferred

#### Scenario: Access denied
- **WHEN** `schtasks /Create` fails with "Access is denied"
- **THEN** a permission error is returned instead of the raw schtasks failure

### Requirement: Windows system service registration
The system SHALL install a system-tier service on Windows as an own-process Windows Service named `iroh-tunnel-<role>` (plus the test-only suffix), with automatic start and the LocalSystem account. Its command line SHALL be `"<binary>" <role> run --config "<config>"`, and its service environment SHALL carry the pinned state directory and `IROH_TUNNEL_SCM=1`, and no trusted SID.

#### Scenario: Service definition
- **WHEN** `serve service install` succeeds from an elevated Windows terminal
- **THEN** a service `iroh-tunnel-serve` exists with automatic start, LocalSystem account and the quoted `serve run --config` command line naming `%ProgramData%\iroh-tunnel\config\serve.toml`, and it is running

### Requirement: Windows system install replaces and rolls back
The system SHALL make a Windows system install replace an existing service of the role. A failure after the service is created SHALL delete it, so nothing stays half-registered.

#### Scenario: Reinstall
- **WHEN** `serve service install` runs from an elevated terminal while `iroh-tunnel-serve` exists
- **THEN** one `iroh-tunnel-serve` service exists afterwards, carrying the new definition, without any confirmation prompt

#### Scenario: Start failure rolls back
- **WHEN** the newly created service fails to start during install
- **THEN** the command fails and no `iroh-tunnel-serve` service remains

### Requirement: Windows service recovery and exit mapping
The system SHALL map the shared restart policy onto service recovery actions:
- restart after the policy delay, with the failure count reset after one day;
- no actions when the policy is disabled;
- applied to non-crash failures as well.

A run that ends because the role is already running in the system tier (exit code 6) SHALL be reported to the Service Control Manager as a clean stop, so recovery does not fire. Any other error SHALL be reported as a service-specific failure.

#### Scenario: Role already running
- **WHEN** the system service starts while another system-tier process of the role holds `<role>.lock` in the machine directory
- **THEN** the service log says the role is already running, the service is reported stopped without error, and no recovery restart follows

#### Scenario: User instance does not stop the service
- **WHEN** the system service starts while only user-tier processes of the role run and hold the presence file shared
- **THEN** the service reaches running

#### Scenario: Crash recovery
- **WHEN** the service process exits with a failure other than exit code 6
- **THEN** the Service Control Manager restarts it after 5 seconds

### Requirement: Windows service stop drains
The system SHALL treat the Service Control Manager's stop, shutdown and pre-shutdown requests as a graceful stop under the shared drain contract. While draining it SHALL report stop-pending with a wait hint covering the drain budget and the watchdog, and it SHALL advance the checkpoint until exit.

#### Scenario: Stop removes the status file
- **WHEN** `service stop` runs from an elevated terminal against a running system service
- **THEN** the service reaches stopped and the role's status snapshot is removed

#### Scenario: Stop pending while draining
- **WHEN** the service status is queried while a stop request is draining an open connection
- **THEN** it reports stop-pending with a wait hint of at least 13 seconds

### Requirement: Windows service reports running only after startup checks
The system SHALL report a Windows system service as running only after the role lock is claimed and the config has passed the trust check and loaded. A failure before that point SHALL end the service with a failure status and SHALL NOT be reported as running.

#### Scenario: Invalid config at start
- **WHEN** the pinned config is invalid TOML and the service starts
- **THEN** the service never reports running, the log contains `failed to parse`, and the config's owner and DACL are unchanged

### Requirement: Windows system verbs on a missing service
The system SHALL fail Windows system-tier `service start|stop|status` for a role without a system service with service exit code 5 and elevated `service install` guidance. It SHALL treat stop of an already-stopped service and uninstall of an absent service as success, and SHALL read service state from structured Service Control Manager status, never from localized text.

#### Scenario: Not installed
- **WHEN** `serve service start|stop|status` runs from an elevated terminal with no `iroh-tunnel-serve` service
- **THEN** each exits with code 5 and stderr contains `service install`

#### Scenario: Idempotent stop and uninstall
- **WHEN** `serve service stop` runs from an elevated terminal against a stopped service, and `serve service uninstall` runs with no service
- **THEN** both succeed

### Requirement: Windows machine directory
The system SHALL resolve the Windows machine directory `%ProgramData%\iroh-tunnel` from the known folder. An elevated process SHALL create it with a protected DACL granting SYSTEM and Administrators full access and Users read access, and SHALL create its `config` subdirectory with a protected DACL granting only SYSTEM and Administrators. An existing directory that fails the trust evaluation for SYSTEM, Administrators and TrustedInstaller SHALL be refused and never repaired. An existing `config` subdirectory or config file whose DACL grants read access to any principal other than SYSTEM, Administrators and TrustedInstaller SHALL be refused with a permission error before any secret is written or served, and SHALL NOT be repaired. Uninstall SHALL leave the directories in place.

#### Scenario: Directory pre-created by a user
- **WHEN** a non-administrator created `%ProgramData%\iroh-tunnel` before the first system install
- **THEN** install exits with code 3 naming the directory and its owner, no service is registered, and the directory's DACL is unchanged

#### Scenario: Directory made untrusted after install
- **WHEN** an untrusted SID gains write access to `%ProgramData%\iroh-tunnel` and the service starts
- **THEN** the service ends with a failure status without writing a log file there, and the directory's DACL is unchanged

#### Scenario: Readable config directory is refused
- **WHEN** an Administrators-owned `%ProgramData%\iroh-tunnel\config` grants Users read access and an elevated `serve config keygen` or `serve service install` runs
- **THEN** the command exits with code 3 naming the directory and `Users`, no config is written, and the DACL is unchanged

#### Scenario: Config directory hides secrets from users
- **WHEN** an elevated `serve config keygen` creates `%ProgramData%\iroh-tunnel\config\serve.toml`
- **THEN** a non-elevated user cannot read the file

### Requirement: Windows system service log
The system SHALL, when running as a Windows system service, write its log to `%ProgramData%\iroh-tunnel\logs\<role>.log`, capped in size with one rotated file.

#### Scenario: Log available after a refusal
- **WHEN** the system service refuses an unsafe config
- **THEN** `%ProgramData%\iroh-tunnel\logs\serve.log` contains the permission error

### Requirement: Windows user background task registration
The system SHALL, for the Windows user tier, replace and start a per-role logon task with least privilege and restart-on-failure that runs only in the user's interactive session.

#### Scenario: Task execution policy
- **WHEN** `iroh-tunnel-<role>` is registered
- **THEN** it uses InteractiveToken, IgnoreNew, no execution limit, battery-insensitive settings and restart interval 5 seconds with count 999, then runs immediately

#### Scenario: Task definition
- **WHEN** the task XML is rendered
- **THEN** it contains `<LogonTrigger>`, `InteractiveToken`, `LeastPrivilege` and not `HighestAvailable`

### Requirement: BusyBox init script values are shell literals
The system SHALL render every value interpolated into the BusyBox init script (binary, role, config, state directory, PID file, log and proc paths) as a POSIX shell literal, so that no character is interpreted by the shell and the daemon receives the exact configured paths. The script SHALL export `IROH_TUNNEL_SERVICE=1` and SHALL NOT contain an owner pin or a trusted uid.

#### Scenario: Config path with shell metacharacters
- **WHEN** the config path contains `$(touch <marker>)`, a backtick command, `"`, `'`, `\` and a space, and the rendered script's `start` runs
- **THEN** no marker file is created and the daemon receives the config path byte-for-byte

#### Scenario: State and binary paths with shell metacharacters
- **WHEN** the binary and state paths contain `$name`, a backtick and a space
- **THEN** the daemon is the binary at the literal path and sees `IROH_TUNNEL_STATE_DIR` equal to the literal state path

#### Scenario: Service marker without a pin
- **WHEN** an init script is rendered
- **THEN** the daemon sees `IROH_TUNNEL_SERVICE=1`, and the script contains neither `iroh-tunnel-pin` nor `IROH_TUNNEL_TRUSTED_UID`

### Requirement: BusyBox install refuses non-UTF-8 paths
The system SHALL refuse to install a BusyBox script whose binary, config or state path is not valid UTF-8, with a config error (exit code 2) naming the path, before the init script is written or any service command runs; a managed binary copy prepared earlier in the same install may remain, as after any other failed registration.

#### Scenario: Non-UTF-8 path refused
- **WHEN** a BusyBox install is rendered with a config path that is not valid UTF-8
- **THEN** install fails with exit code 2 and an error naming the path, and no script is written

### Requirement: BusyBox init script verifies process identity
The BusyBox init script SHALL treat the PID in its PID file as the running daemon only when that process is alive and its executable (`/proc/<pid>/exe`) is the executable recorded in the PID file, with or without the kernel's ` (deleted)` suffix. `status` SHALL NOT modify the PID file.

#### Scenario: Running daemon replaced on disk
- **WHEN** the daemon's executable shows as `<recorded path> (deleted)` because a re-install replaced the managed copy
- **THEN** `start` reports it already running and `stop` stops it

### Requirement: BusyBox init script discards stale PID files
A PID file whose PID is not a positive decimal number without leading zeros, is not a live process, or whose process runs another executable SHALL be stale: `start` and `stop` SHALL remove it without signalling the process, `start` SHALL launch the daemon, and `status` SHALL report not running with exit status 3.

#### Scenario: Stale PID file names an unrelated live process
- **WHEN** the PID file names a live process whose executable is not the recorded daemon, and `stop` runs
- **THEN** the process receives no signal, keeps running, and the PID file is removed

#### Scenario: Start with a stale PID file
- **WHEN** the PID file names a live unrelated process and `start` runs
- **THEN** the daemon is launched and the PID file is rewritten with the daemon's PID and executable

#### Scenario: Status with a stale PID file
- **WHEN** the PID file names a live unrelated process and `status` runs
- **THEN** it reports not running, exits with status 3 and leaves the PID file in place

#### Scenario: Dead or malformed PID file
- **WHEN** the PID file names a PID with no live process, or holds no decimal PID, and `start` runs
- **THEN** the PID file is replaced and the daemon is launched; `status` on such a file reports not running with exit status 3

### Requirement: BusyBox init script refuses an unverifiable identity
When the PID may be live but its executable cannot be confirmed (no `<proc>/<pid>` entry, `readlink` unavailable, or the caller may not signal a process whose `<proc>/<pid>` entry exists), the BusyBox init script SHALL treat the identity as unverifiable: `start` and `stop` SHALL exit with status 1 without signalling and keep the PID file, and `status` SHALL report unknown with exit status 4.

#### Scenario: Unverifiable identity
- **WHEN** the PID file names a live process but no `<proc>/<pid>` entry exists
- **THEN** `stop` and `start` exit with status 1 without signalling it and keep the PID file, and `status` exits with status 4

#### Scenario: Status of a root daemon by a non-root user
- **WHEN** a non-root user runs `status` while the PID file names a live root-owned daemon
- **THEN** `status` reports unknown with exit status 4 and does not claim the daemon is stopped

### Requirement: BusyBox init script recognises legacy PID files
For a PID file with no recorded executable (written by an earlier script), the BusyBox init script SHALL count the process as the daemon when its executable is the current `DAEMON` (with or without ` (deleted)`), or when its executable name starts with `iroh-tunnel` and its command line ends with this role's `run --config <config>` for this script's config; other processes SHALL be stale and an unreadable identity unverifiable.

#### Scenario: Legacy PID file of the same binary
- **WHEN** the PID file holds only a PID whose process runs the current `DAEMON`, and `restart` runs
- **THEN** the old process is stopped and the daemon is started with a PID file in the new format

#### Scenario: Legacy PID file after a binary path change
- **WHEN** the PID file holds only a PID whose process is an `iroh-tunnel` executable at another path running this role with `run --config <config>`, in either the combined (`iroh-tunnel <role> run`) or the standalone (`iroh-tunnel-<role> run`) form, and `restart` runs
- **THEN** the old process is stopped before the daemon is started

#### Scenario: Legacy PID file of an unrelated process
- **WHEN** the PID file holds only a PID whose process is neither the current `DAEMON` nor an `iroh-tunnel` executable running this role and config (for example the other role, or another config)
- **THEN** `stop` does not signal it and `start` launches the daemon

#### Scenario: Re-install migrates an existing registration
- **WHEN** `service install` runs over a BusyBox registration written by an earlier version
- **THEN** the script is rewritten with literal values and identity checks, and its `restart` replaces the old daemon

### Requirement: Windows service logs refused status reports
Once the service log is open, the system SHALL log every status report the Service Control Manager refuses at warning level or above, naming the reported state and, for a pending report, its checkpoint. These lines SHALL follow the service log's existing rules: a quiet run keeps only errors, and an untrusted machine directory gets no log.

#### Scenario: Refusal is logged
- **WHEN** the Service Control Manager refuses a stop-pending report after the service log is open
- **THEN** the service log names the stop-pending state and its checkpoint

### Requirement: Windows service retries running and stopped reports
The system SHALL retry a refused running or stopped report a bounded number of times. When the running report is still refused, it SHALL request the graceful stop and end the service with service-specific code 101, and MUST NOT leave it in start-pending. When the stopped report is still refused, the process SHALL exit with code 101 without sending it again. A refused pending report SHALL NOT be retried on its own; the next pending report carries a fresh checkpoint.

#### Scenario: Running report refused once
- **WHEN** the Service Control Manager refuses the first running report of a starting service
- **THEN** the report is retried, the service is reported running, and the refusal is in the service log

#### Scenario: Running report always refused
- **WHEN** the Service Control Manager refuses every attempt of the running report
- **THEN** the role drains, the service is reported stopped with service-specific code 101, and no start-pending report follows

#### Scenario: Stopped report refused once
- **WHEN** the Service Control Manager refuses the first stopped report of a finished run
- **THEN** the stopped report is retried and accepted with the run's exit mapping

#### Scenario: Stopped report always refused
- **WHEN** the Service Control Manager refuses every attempt of the stopped report
- **THEN** the refusals are logged, the stopped report is not sent again, and the process exits with code 101

#### Scenario: Pending report refused
- **WHEN** the Service Control Manager refuses a start-pending report
- **THEN** that report is not sent again and the next tick reports a higher checkpoint

#### Scenario: Stop request while the running report is retried
- **WHEN** a stop request arrives while the running report is being retried
- **THEN** the stop is handled after the retry settles: a clean stop when the running report was accepted, or the stop already in progress with code 101 when it was not

### Requirement: Windows service internal faults end the service with fixed codes
The system SHALL end the Windows system service with a service-specific failure when its own bookkeeping fails: code 100 when the internal service state is poisoned by a panic, which MUST NOT be continued on, and code 102 when the pending-checkpoint ticker cannot start, which MUST NOT abort the process. Codes 100, 101 and 102 MUST NOT be used for any other exit.

#### Scenario: Poisoned service state
- **WHEN** the internal service state is poisoned by a panic while the service runs
- **THEN** the service is reported stopped with service-specific code 100 and the process exits

#### Scenario: Status ticker cannot start
- **WHEN** the pending-checkpoint ticker thread cannot be created at service start
- **THEN** the role does not run and the service is reported stopped with service-specific code 102

### Requirement: Windows service fault codes take precedence over the exit mapping
A recorded fault code (100, 101 or 102) SHALL replace the run's own exit mapping from "Windows service recovery and exit mapping", including a clean exit after the drain. A fault SHALL be recorded only before the role runs or after it reported running, so it never meets an already-running exit (code 6). Fault codes SHALL trigger the shared recovery policy like any other service-specific failure.

#### Scenario: Fault overrides a clean drain
- **WHEN** a fault is recorded and the drained run then exits with code 0
- **THEN** the service is reported stopped with the fault's code, not as a clean stop

#### Scenario: Fault triggers recovery
- **WHEN** the service ends with code 101 and the restart policy is enabled
- **THEN** the Service Control Manager restarts it after the policy delay

### Requirement: Windows user task refused for always-elevated accounts
The system SHALL refuse a user-tier `service install` on Windows with a permission error (exit code 3), registering nothing, when the logon task would run with an elevated token: the account is the built-in Administrator, or it is a member of Administrators while UAC is disabled (`EnableLUA` is 0). The message SHALL say the account always runs elevated and tell the operator to install from an elevated terminal to get a system service.

#### Scenario: Administrator with UAC disabled
- **WHEN** a non-elevated process of an Administrators member runs `serve service install` on a host with `EnableLUA=0`
- **THEN** exit code is 3, stderr says the account always runs elevated and names the elevated `service install`, and no task exists

#### Scenario: Standard user with UAC disabled
- **WHEN** a non-admin account runs `serve service install` on a host with `EnableLUA=0`
- **THEN** the user logon task is registered

#### Scenario: Administrator with UAC enabled
- **WHEN** an Administrators member runs `serve service install` from a non-elevated terminal with `EnableLUA=1`
- **THEN** the user logon task is registered

### Requirement: System service persists a generated key into its own config
The system SHALL, under a system service, generate and persist a missing `secret_key` into the system-tier config through the normal atomic, locked store, keeping the file owned by root (or Administrators) with owner-only access, and SHALL report an invalid edited config without serving it and without changing its owner or mode.
Evidence: src/service/trust.rs:392-420, tests/service_system.rs (SM-EDIT-01)

#### Scenario: Administrator edits config
- **WHEN** the administrator edits `/etc/iroh-tunnel/serve.toml` with `sudo iroh-tunnel serve config set ...` and restarts the service
- **THEN** the new services are served and the file keeps owner root and mode 0600

#### Scenario: Invalid edit
- **WHEN** the config is invalid TOML at restart
- **THEN** the log contains `failed to parse`, the service does not run, and the file's owner and mode are unchanged

#### Scenario: Missing key
- **WHEN** the system config has an empty `secret_key` and the service starts
- **THEN** a key is written into the file, the file stays owner root with mode 0600, and the service serves

### Requirement: BusyBox backend needs root
The system SHALL require root for every BusyBox backend operation except `status`, failing a non-root operation with a permission error that says the host has no per-user services and to re-run with sudo, and SHALL change nothing in that case.
Evidence: src/service/busybox.rs:155-215, src/service/busybox.rs:222-262
Confidence: inferred

#### Scenario: Non-root BusyBox operation
- **WHEN** a non-root user installs, uninstalls, starts, stops or restarts on a BusyBox host
- **THEN** the command fails with a permission error mentioning sudo and nothing is changed

#### Scenario: Root BusyBox install
- **WHEN** root installs on a BusyBox host
- **THEN** the system-wide `/etc/init.d` script is installed with no user-scope note

### Requirement: Windows service start requires the service context
The system SHALL fail with a configuration error (exit code 2) when `IROH_TUNNEL_SCM=1` is set but the process was not started by the Service Control Manager, and SHALL NOT fall back to a foreground run. A Windows system service started with `IROH_TUNNEL_TRUSTED_SID` SHALL be refused as a legacy registration ("Legacy system registrations are refused with guidance").

#### Scenario: Service marker outside the SCM
- **WHEN** `serve run` is started from a terminal with `IROH_TUNNEL_SCM=1`
- **THEN** it exits with code 2 and does not serve

#### Scenario: Legacy pinned service
- **WHEN** the system service environment has `IROH_TUNNEL_TRUSTED_SID`
- **THEN** the service log contains `service install` and the system config path, and the service does not serve

### Requirement: Start refused only when the role runs in the same tier
The system SHALL refuse `service start`, before invoking the service manager, only when the role already runs in the same tier. A role running only in the other tier SHALL NOT refuse `service start` or `service install`, which print the other-tier notice and proceed; re-installing over the same tier's running service SHALL stay supported.
Evidence: crates/host/src/service/install.rs (plan_install pre-checks), crates/host/src/service/verb.rs, crates/host/src/ownership.rs (Tier, probe)

#### Scenario: Role already running
- **WHEN** `service start` is invoked while the role holds its lock in the same tier
- **THEN** the command fails from the ownership check rather than starting a second instance

#### Scenario: User install while the system service runs
- **WHEN** a non-root user with a valid user-tier config runs `service install` on a systemd, launchd or Windows host while the system-tier serve runs
- **THEN** the user service is registered and started, stderr contains the other-tier notice, and the system service keeps running

#### Scenario: System install while a user service runs
- **WHEN** `sudo iroh-tunnel serve service install` runs while a user-tier serve runs and holds `serve.user.lock` shared
- **THEN** the system service is registered and started, stderr contains the other-tier notice, and the user service keeps running

#### Scenario: Re-install over the same tier's own running service
- **WHEN** `service install` is invoked while the role is held only in the invoking tier
- **THEN** the install proceeds and the registration is replaced, as for an install on a host where the role does not run

### Requirement: Service tier follows privilege
The system SHALL run every `service` verb (`install`, `uninstall`, `start`, `stop`, `restart`, `status`) in the tier of the invoking process: a system service when the process is root (including under `sudo`) or an elevated Windows Administrator, and a user service otherwise. The verbs SHALL accept no scope flag and SHALL NOT prompt for a scope, a user name or a retarget confirmation.

#### Scenario: Root installs a system service
- **WHEN** `sudo iroh-tunnel serve service install` runs on a systemd host
- **THEN** `/etc/systemd/system/iroh-tunnel-serve.service` is written and no question is printed

#### Scenario: User installs a user service
- **WHEN** a non-root user with a valid user-tier config runs `iroh-tunnel serve service install` on a terminal of a systemd, launchd or Windows host
- **THEN** a user service is registered and no question is printed

#### Scenario: Elevated Windows terminal
- **WHEN** `iroh-tunnel serve service stop` runs from an elevated Windows terminal
- **THEN** the Windows Service is targeted, not the user task

#### Scenario: Removed flags
- **WHEN** `iroh-tunnel serve service install --system` or `--user` is run
- **THEN** argument parsing fails with an unexpected-argument error and nothing is registered

### Requirement: System services use machine-wide files
The system SHALL, for a system install without `--config`, use the system-tier default config path and, without `IROH_TUNNEL_STATE_DIR`, the system-tier default state directory, pin both in canonical form (resolving root-owned system links such as macOS `/var`), and SHALL fail with a configuration error (exit code 2) naming the missing path and the elevated `config keygen` command when the config does not exist. It SHALL NOT read `SUDO_USER`, `HOME` or any user profile to choose these paths.

#### Scenario: Default system install
- **WHEN** `/etc/iroh-tunnel/serve.toml` exists and `sudo iroh-tunnel serve service install` runs on Linux with `SUDO_USER=alice`
- **THEN** the registration contains `--config /etc/iroh-tunnel/serve.toml` and `IROH_TUNNEL_STATE_DIR=/run/iroh-tunnel`, and nothing under `/home/alice`

#### Scenario: macOS default paths pass the trust walk
- **WHEN** `sudo iroh-tunnel serve service install` runs on macOS with default paths
- **THEN** the plist pins `/Library/Application Support/iroh-tunnel/config/serve.toml` and the state directory `/Library/Application Support/iroh-tunnel/state`, the trust walk accepts both, and the service reaches running

#### Scenario: Missing system config
- **WHEN** `sudo iroh-tunnel serve service install` runs and `/etc/iroh-tunnel/serve.toml` does not exist
- **THEN** exit code is 2, stderr names `/etc/iroh-tunnel/serve.toml` and `sudo iroh-tunnel serve config keygen`, and nothing is registered

### Requirement: System-tier files are trusted only when admin-owned
The system SHALL verify the config file, the state directory and their ancestors at system install and at every service-manager start, accepting only admin owners (root; SYSTEM, Administrators, TrustedInstaller) and refusing entries writable by others, symlinks and non-canonical paths with a permission error naming path, owner and mode. It SHALL parse only the verified handle, never repair access, and skip foreground elevated runs.

#### Scenario: User-owned config at install
- **WHEN** `sudo iroh-tunnel serve service install --config /home/alice/serve.toml` runs and the file is owned by alice
- **THEN** exit code is 3, stderr names the path and alice's uid, and nothing is registered

#### Scenario: Config loosened after install
- **WHEN** the system config is made mode 0666 and the service is restarted by its manager
- **THEN** the service log contains a `permission error` naming the config path and `0666`, and the service never reaches running

#### Scenario: Symlinked config directory
- **WHEN** `/etc/iroh-tunnel` is replaced by a symlink and the service starts
- **THEN** the service log contains `symlink` and the service does not serve

#### Scenario: Link below the canonical root
- **WHEN** a non-root user replaces a directory below the pinned canonical config path with a symlink and the service starts
- **THEN** the service log contains `symlink` and the service does not serve

#### Scenario: Foreground root run in a container
- **WHEN** `iroh-tunnel serve run --config /etc/iroh-tunnel/serve.toml` runs as root in a container where the file is a read-only bind mount owned by uid 1000
- **THEN** the role starts

### Requirement: Registrations mark the service context
The system SHALL set `IROH_TUNNEL_SERVICE=1` in the environment of every Unix registration (systemd unit, launchd plist, BusyBox init script) and keep `IROH_TUNNEL_SCM=1` for the Windows Service, and SHALL NOT write any owner pin or trusted-principal variable.

#### Scenario: systemd unit environment
- **WHEN** a system unit is rendered
- **THEN** it contains `Environment=IROH_TUNNEL_SERVICE=1` and `IROH_TUNNEL_STATE_DIR`, and contains neither `iroh-tunnel-pin` nor `IROH_TUNNEL_TRUSTED_UID`

### Requirement: Legacy system registrations are refused with guidance
The system SHALL refuse `run` with a configuration error (exit code 2), before reading any config, when `IROH_TUNNEL_TRUSTED_UID` or `IROH_TUNNEL_TRUSTED_SID` is set, or when a root process runs under systemd (`INVOCATION_ID`) or launchd (`XPC_SERVICE_NAME`) without `IROH_TUNNEL_SERVICE=1`. The message SHALL name the system-tier config path of the role and tell the operator to create it and re-run `service install` with elevation. Nothing SHALL be copied or migrated.

#### Scenario: Pinned registration after upgrade
- **WHEN** a system unit written by an earlier version starts `serve run --config /home/alice/.config/iroh-tunnel/serve.toml` with `IROH_TUNNEL_TRUSTED_UID=1000`
- **THEN** the service log contains `/etc/iroh-tunnel/serve.toml` and `sudo iroh-tunnel serve service install`, the service does not serve, and alice's file is unchanged

#### Scenario: Windows pinned service after upgrade
- **WHEN** a Windows system service written by an earlier version starts with `IROH_TUNNEL_TRUSTED_SID` set
- **THEN** the service log names `%ProgramData%\iroh-tunnel\config\serve.toml` and the elevated `service install` command, and the service does not serve

### Requirement: Windows user-tier stop terminates without draining
The system SHALL stop a Windows user-tier role with `schtasks /End` only, with no control channel of its own. Task Scheduler ends the process without the drain, after `stop` returns; the status snapshot and owner record MAY remain and SHALL be handled as crash leftovers. The stop SHALL NOT trigger restart-on-failure.
Evidence: user_background_task_lifecycle (exit), status_cmd.rs (stale), ownership.rs (start), #185 (no restart)

#### Scenario: Stop leaves a stale snapshot
- **WHEN** `serve service stop` runs from a non-elevated terminal against a running logon task
- **THEN** the role process exits without draining, and once it has exited `serve service status` reports `serve is not running`

#### Scenario: Leftover snapshot reported stale
- **WHEN** `serve status` runs after the stopped process has exited and its snapshot is still present
- **THEN** stderr warns the snapshot is stale and stdout still shows it

#### Scenario: Next start is not blocked
- **WHEN** `serve service start` runs after the stopped process has exited, with its snapshot and owner record still present
- **THEN** a new process starts and publishes a new snapshot, without any leftover being removed by hand

#### Scenario: No restart after stop
- **WHEN** a running logon task with restart-on-failure is stopped with `serve service stop`
- **THEN** no new role process is started by Task Scheduler

### Requirement: Windows graceful stop requires the system tier
The user documentation SHALL state that a Windows user-tier stop does not drain and SHALL name the system tier ("Windows service stop drains") as the way to get a graceful stop on Windows.
Evidence: docs/services.md (Windows, User tier)

#### Scenario: Documented alternative
- **WHEN** a user reads the Windows user-tier section of `docs/services.md`
- **THEN** it says `stop` terminates the role without draining and points to the system tier for a graceful stop
