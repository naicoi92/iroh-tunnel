## ADDED Requirements

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

## MODIFIED Requirements

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

### Requirement: User service lifecycle
The system SHALL, for a user-tier service, install and start, report status (running/stopped), stop, start, restart and uninstall through the host's per-session service manager (systemd user manager, LaunchAgent, logon task), with the same node identity across stop/start/restart and the registration removed on uninstall. The service SHALL start at the user's login and SHALL NOT be configured to outlive the login session; the system SHALL NOT enable or inspect systemd linger.
Evidence: tests/service_user.rs (user_service_lifecycle_uses_the_pinned_absolute_paths, opt-in via IROH_TUNNEL_E2E_SERVICE=1), src/app.rs:330-352

#### Scenario: Full lifecycle
- **WHEN** a user service is installed, stopped, started, restarted and uninstalled
- **THEN** the status file reports the same node id after each start, is removed after stop, and the registration file no longer exists after uninstall

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

### Requirement: Windows service stop drains
The system SHALL treat the Service Control Manager's stop, shutdown and pre-shutdown requests as a graceful stop under the shared drain contract. While draining it SHALL report stop-pending with a wait hint covering the drain budget and the watchdog, and it SHALL advance the checkpoint until exit.

#### Scenario: Stop removes the status file
- **WHEN** `service stop` runs from an elevated terminal against a running system service
- **THEN** the service reaches stopped and the role's status snapshot is removed

#### Scenario: Stop pending while draining
- **WHEN** the service status is queried while a stop request is draining an open connection
- **THEN** it reports stop-pending with a wait hint of at least 13 seconds

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

### Requirement: Windows user background task registration
The system SHALL, for the Windows user tier, replace and start a per-role logon task with least privilege and restart-on-failure that runs only in the user's interactive session.

#### Scenario: Task execution policy
- **WHEN** `iroh-tunnel-<role>` is registered
- **THEN** it uses InteractiveToken, IgnoreNew, no execution limit, battery-insensitive settings and restart interval 5 seconds with count 999, then runs immediately

#### Scenario: Task definition
- **WHEN** the task XML is rendered
- **THEN** it contains `<LogonTrigger>`, `InteractiveToken`, `LeastPrivilege` and not `HighestAvailable`

## REMOVED Requirements

### Requirement: Privileged service never writes a generated key

**Reason**: The system tier owns its config, so a missing key is generated and persisted; replaced by "System service persists a generated key into its own config".

**Migration**: None needed.

### Requirement: BusyBox backend needs root and ignores user scope

**Reason**: User-scope requests no longer exist; replaced by "BusyBox backend needs root".

**Migration**: Run BusyBox service verbs with `sudo`.

### Requirement: Windows service start requires the service context and a pin

**Reason**: The trusted-SID pin is removed; replaced by "Windows service start requires the service context" and the legacy refusal.

**Migration**: Re-install from an elevated terminal.

### Requirement: Start and conflicting install refused when the role already runs

**Reason**: Cross-tier install refusals are removed; replaced by "Start refused only when the role runs in the same tier".

**Migration**: None needed; tiers may run side by side.

### Requirement: Install scope selection

**Reason**: Replaced by "Service tier follows privilege"; scope flags and the prompt are removed.

**Migration**: Run service verbs without `--user`/`--system`; use `sudo` or an elevated terminal for a system service.

### Requirement: Scope prompt never guesses

**Reason**: There is no scope prompt any more.

**Migration**: None needed.

### Requirement: System scope requires privilege without elevation or fallback

**Reason**: A non-elevated process cannot request the system tier at all; it always acts on its user tier.

**Migration**: Re-run the command with `sudo` or from an elevated terminal to act on the system service.

### Requirement: Default scope for verbs on an existing service

**Reason**: Superseded by "Service tier follows privilege", which applies to every verb.

**Migration**: None needed.

### Requirement: System install resolves the invoking user, never root's home

**Reason**: System services no longer use any user's files, so no user is resolved.

**Migration**: Create the system-tier config with elevation, then re-run the elevated `service install`.

### Requirement: SUDO_USER and --config must agree

**Reason**: `SUDO_USER` is no longer an input.

**Migration**: None needed.

### Requirement: System install uses the user's default config and state locations

**Reason**: Replaced by "System services use machine-wide files".

**Migration**: Move or recreate the config at the system-tier path.

### Requirement: System install verifies files before registering

**Reason**: Replaced by "System-tier files are trusted only when admin-owned", which covers install and start.

**Migration**: None needed.

### Requirement: Registration pins the owner

**Reason**: No user owns a system service's files, so there is nothing to pin.

**Migration**: Re-install; old pinned registrations are refused with guidance.

### Requirement: Retargeting another user's service needs confirmation

**Reason**: Without an owning user there is nothing to retarget; one system service per role belongs to the machine.

**Migration**: None needed.

### Requirement: Privileged service refuses unsafe config paths

**Reason**: Replaced by "System-tier files are trusted only when admin-owned", which trusts only admin principals.

**Migration**: Keep system configs owned by root/Administrators and not writable by others.

### Requirement: Root under a service manager without a pin is refused

**Reason**: Replaced by "Legacy system registrations are refused with guidance", keyed on the new `IROH_TUNNEL_SERVICE` marker.

**Migration**: Re-run the elevated `service install`.

### Requirement: Windows system service never writes a generated key

**Reason**: The system tier owns its config; "System service persists a generated key into its own config" now persists a missing key into it.

**Migration**: None needed.

### Requirement: Windows retarget confirmation

**Reason**: No retargeting exists any more.

**Migration**: None needed.

### Requirement: Windows installing user

**Reason**: Windows system services no longer use the token user's profile.

**Migration**: Create `%ProgramData%\iroh-tunnel\config\<role>.toml` from an elevated terminal and re-install.

### Requirement: Windows token user and --config owner must agree

**Reason**: The token user is no longer an input; system-tier files are checked against admin principals only.

**Migration**: None needed.

### Requirement: Windows cross-scope registration is refused

**Reason**: The user task and the Windows Service of a role may coexist.

**Migration**: None needed; an existing task and service may both stay registered.

