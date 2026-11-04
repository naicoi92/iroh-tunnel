## ADDED Requirements

### Requirement: Install scope selection
The system SHALL honor an explicit install scope; otherwise root defaults to system, interactive unprivileged users choose a scope, and non-interactive unprivileged users default to user scope.
Evidence: src/service/scope.rs:86-150 (unit tests), src/app.rs:273-300, tests/service_user.rs, tests/service_system.rs

#### Scenario: Conflicting explicit scope flags
- **WHEN** both `--user` and `--system` are passed to the CLI
- **THEN** argument parsing rejects the combination

#### Scenario: Root installs without a scope prompt
- **WHEN** root runs `serve service install` with no scope flag
- **THEN** the service is installed with system scope and no scope question is printed

#### Scenario: Interactive user is offered both scopes
- **WHEN** a non-elevated user runs `service install` with no scope flag on a terminal
- **THEN** a prompt offering `user` and `system` is written to stderr, and an empty answer, `u` or `user` selects user scope while `s` or `system` selects system scope

#### Scenario: Non-interactive non-root default
- **WHEN** a non-elevated process with no terminal runs `service install` with no scope flag
- **THEN** a user service is installed without any prompt

### Requirement: Scope prompt never guesses
The system SHALL fail with a configuration error (exit code 2) when the scope prompt receives end-of-input or an unrecognised answer, and SHALL register nothing in that case.
Evidence: src/service/scope.rs:104-150, src/service/scope.rs tests (unknown_or_missing_answer_is_an_error_not_a_guess), tests/service_user.rs (interactive_install_offers_user_or_system_and_system_is_refused_unprivileged)

#### Scenario: Unrecognised scope answer
- **WHEN** the answer to the scope prompt is `maybe`
- **THEN** the command exits with code 2, the message tells the user to answer `user` or `system` or pass `--user`/`--system`, and nothing is registered

#### Scenario: No answer
- **WHEN** stdin reaches end-of-file at the scope prompt
- **THEN** the command fails with a message asking for `--user` or `--system`

### Requirement: System scope requires privilege without elevation or fallback
The system SHALL refuse unprivileged system-scope operations with permission exit code 3 and an explicit sudo rerun command, without self-elevation, user-scope fallback or mutation; install guidance SHALL include the absolute config path.
Evidence: src/service/scope.rs:150-185 (unit tests), src/app.rs:300-380, tests/service_user.rs

#### Scenario: Unprivileged system install
- **WHEN** a non-root user runs `serve service install --system --config <path>`
- **THEN** the command exits with code 3, stderr contains `sudo` and `serve service install --system` and the absolute config path, and no registration file exists

#### Scenario: Unprivileged system verbs
- **WHEN** a non-root user runs `serve service uninstall|start|stop|restart --system`
- **THEN** each exits with code 3 and stderr contains `sudo iroh-tunnel serve service <verb> --system`

### Requirement: Default scope for verbs on an existing service
The system SHALL, for uninstall, start, stop, restart and status without a scope flag, use system scope when the process is elevated and user scope otherwise, and SHALL NOT prompt for these verbs.
Evidence: src/service/scope.rs:84-92, src/app.rs:283-292, src/service/scope.rs tests (default_scope_follows_privilege), tests/service_system.rs (restart/stop run with `--system`)

#### Scenario: Root without a flag
- **WHEN** root runs `serve service restart` with no scope flag
- **THEN** the system-scope service is restarted

#### Scenario: Non-root without a flag
- **WHEN** a non-root user runs `serve service stop` with no scope flag
- **THEN** the user-scope service is targeted and no privilege is required

### Requirement: Service management refused for standalone role binaries
The system SHALL refuse the `service` subcommand of a standalone role binary with a service error directing the user to `iroh-tunnel <role> service ...`.
Evidence: src/app.rs:218-224
Confidence: inferred

#### Scenario: Standalone binary
- **WHEN** `service` is invoked on a standalone serve or access binary
- **THEN** the command fails with a message that service management is not yet supported by the standalone binary and names the `iroh-tunnel <role> service` form

### Requirement: Installed services pin absolute config and state paths
The system SHALL write the absolute config path and absolute state directory into the registration (state via `IROH_TUNNEL_STATE_DIR`), resolving a relative `--config` against the current directory, so the service runs identically regardless of working directory or home.
Evidence: src/app.rs:238-250,306-330,429-440, src/service/systemd.rs:46-90, src/service/launchd.rs:84-140, src/service/busybox.rs:47-120, tests/service_user.rs, tests/service_system.rs

#### Scenario: Install from another working directory
- **WHEN** a user service is installed from `/` with a config path containing a space
- **THEN** the registration contains the absolute config path and state directory and the service writes its status file under that state directory

#### Scenario: State directory override
- **WHEN** `IROH_TUNNEL_STATE_DIR` is set at install time
- **THEN** that directory (made absolute) is the pinned state directory; otherwise the platform default is pinned

### Requirement: Install refuses a config that could only fail at start
The system SHALL check that the pinned config exists and is valid for the role before registering a user-scope service, and SHALL register nothing and exit with code 2 when it is not.
Evidence: src/app.rs:318-330, src/app.rs:440-470, tests/service_user.rs (install_without_config_fails_with_guidance_and_registers_nothing)

#### Scenario: Missing config
- **WHEN** `serve service install --user --config <nonexistent path>` is run
- **THEN** the command exits with code 2, stderr names the config path, and no registration exists

### Requirement: Registration write permission failures keep scope and account
The system SHALL, when the OS denies writing a registration, fail with a permission error (exit code 3) saying that the scope and account were not changed, nothing was registered, and `--system` should be used (as root) only for a system service; it SHALL NOT retry under another scope or account.
Evidence: src/service/mod.rs:296-340, tests/service_user.rs (denied_registration_fails_with_guidance_and_keeps_scope)

#### Scenario: Read-only home
- **WHEN** the user's home directory is mode 0500 and `service install --user` runs
- **THEN** the command exits with code 3, stderr mentions permission and that scope is not changed automatically (`--system`), and no registration file exists

### Requirement: Registration files are written atomically with fixed permissions
The system SHALL write registrations to a temporary sibling file and rename it into place, creating the parent directory, with mode 0755 for extension-less files (init scripts) and 0644 otherwise, independent of umask.
Evidence: src/service/mod.rs:310-345
Confidence: inferred

#### Scenario: Install writes a unit
- **WHEN** a registration is written
- **THEN** the final file appears by rename with mode 0644 (or 0755 for an init script), and a failed write removes the temporary file

### Requirement: Linux backend chosen at runtime
The system SHALL use the systemd backend on Linux when `/run/systemd/system` exists or PID 1's executable ends in `systemd`, and otherwise the BusyBox/SysV init backend; macOS uses launchd; Windows uses a Task Scheduler task; other platforms fail with a message that service management is supported only on Linux, macOS and Windows.
Evidence: src/service/mod.rs:1-12, src/service/mod.rs:365-395, tests/service_system.rs (header: BusyBox backend in a systemd-less container)

#### Scenario: Container without systemd
- **WHEN** a service is installed on a Linux host without `/run/systemd/system`
- **THEN** an executable `/etc/init.d/S96iroh-tunnel-<role>` script is installed

### Requirement: systemd unit content and lifecycle
The system SHALL install a role systemd unit with pinned config/state paths, restart-on-failure after 5 seconds except exit 6, and reload, enable and restart it; user operations SHALL use `systemctl --user`.
Evidence: src/service/systemd.rs:16-170 (unit tests), src/service/mod.rs (restart-policy tests)

#### Scenario: Unit registration shape
- **WHEN** a unit is generated
- **THEN** `iroh-tunnel-<role>.service` lives under `/etc/systemd/system` or `~/.config/systemd/user`, runs `<binary> <role> run --config <config>`, sets `IROH_TUNNEL_STATE_DIR` and is wanted by `default.target`

#### Scenario: Unit text
- **WHEN** a serve unit is rendered for `/usr/bin/iroh-tunnel` and config `/etc/cfg`
- **THEN** it contains `ExecStart=/usr/bin/iroh-tunnel serve run --config /etc/cfg`, `Restart=on-failure`, `RestartSec=5` and `RestartPreventExitStatus=6`

#### Scenario: Re-install over a running unit
- **WHEN** the service is installed again
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
The system SHALL install and restart a mode-0755 BusyBox role init script with pinned config/state, start/stop/restart/status support, and a SIGTERM then 5-second SIGKILL stop policy.
Evidence: src/service/busybox.rs:30-150,215-250 (unit tests), tests/service_system.rs

#### Scenario: BusyBox paths and stopped status
- **WHEN** a BusyBox role script is generated
- **THEN** it uses `/etc/init.d/S96iroh-tunnel-<role>`, `/var/run/iroh-tunnel-<role>.pid`, `/var/log/iroh-tunnel-<role>.log`, exports `IROH_TUNNEL_STATE_DIR`, runs `<binary> <role> run --config <config>` in background and returns status 3 when stopped

#### Scenario: Script verbs
- **WHEN** the script is rendered
- **THEN** it begins with `#!/bin/sh`, embeds DAEMON, CONFIG, PIDFILE and LOG, and wires `start`, `stop`, `restart` (stop then start) and `status`

### Requirement: BusyBox backend needs root and ignores user scope
The system SHALL require root for every BusyBox backend operation except `status`, failing with a permission error that says the host has no per-user services and to re-run with sudo, and SHALL treat a user-scope request as the same system-wide script with an explanatory note rather than a user install.
Evidence: src/service/busybox.rs:155-215, src/service/busybox.rs:222-262
Confidence: inferred

#### Scenario: User-scope install on BusyBox as root
- **WHEN** root requests user scope on a BusyBox host
- **THEN** a note that the service is a system-wide `/etc/init.d` script is printed and the script is installed

#### Scenario: Non-root BusyBox operation
- **WHEN** a non-root user installs, uninstalls, starts, stops or restarts on a BusyBox host
- **THEN** the command fails with a permission error and nothing is changed

### Requirement: launchd registration and domains
The system SHALL install a labelled launchd role job in the appropriate user/system domain, with pinned command and state paths, RunAtLoad and restart on unsuccessful exit; install SHALL boot out the old job before bootstrap.
Evidence: src/service/launchd.rs:36-215 (unit tests), tests/service_user.rs

#### Scenario: launchd registration shape
- **WHEN** a launchd job is registered
- **THEN** `dev.iroh-tunnel.<role>` uses `~/Library/LaunchAgents` with `gui/<uid>` or `/Library/LaunchDaemons` with `system`, XML-escaped arguments and `IROH_TUNNEL_STATE_DIR`; failure booting out an old job is ignored

#### Scenario: Re-install over a loaded job
- **WHEN** the service is installed while already loaded
- **THEN** the existing job is booted out first and the new plist is bootstrapped

### Requirement: launchd verbs and idempotent stop
The system SHALL start with `kickstart`, restart with `kickstart -k`, stop with `launchctl kill SIGTERM` treating "no process to signal" (exit 3) as success and printing that it is already stopped, and when the job is not loaded SHALL report that no service is loaded in the current scope domain with an install hint naming the other scope flag.
Evidence: src/service/launchd.rs:217-345
Confidence: inferred

#### Scenario: Stop an already stopped job
- **WHEN** `service stop` runs and launchd reports no process to signal
- **THEN** the command succeeds and prints `<label> is already stopped.`

### Requirement: Restart policy is shared across platforms
The system SHALL apply one default restart policy (restart on failure, 5 second delay) to every backend; launchd ignores the delay.
Evidence: src/service/mod.rs:48-75, src/service/mod.rs tests (restart_policy_default_matches_previous_hardcoded_values), src/service/systemd.rs tests, src/service/task_xml.rs tests

#### Scenario: Policy disabled
- **WHEN** a unit is rendered with restart-on-failure disabled
- **THEN** it contains `Restart=no`

### Requirement: Binary resolution for registrations
The system SHALL put the `iroh-tunnel` found on `PATH` into the registration and otherwise the absolute path of the current executable.
Evidence: src/service/mod.rs:395-410, src/service/mod.rs tests (resolve_binary_returns_a_path)

#### Scenario: Not on PATH
- **WHEN** `iroh-tunnel` is not on `PATH` (for example `cargo run`)
- **THEN** the current executable's absolute path is registered

### Requirement: Test-only instance suffix
The system SHALL, only in builds with the `test-utils` feature, append the value of `IROH_TUNNEL_TEST_SERVICE_SUFFIX` to service, label and task names so test registrations do not collide with real services; production builds use no suffix.
Evidence: src/service/mod.rs:290-300, tests/service_user.rs (Registered::registration uses suffix)

#### Scenario: Suffix set in a test build
- **WHEN** the variable is `-e2e123` in a test-utils build
- **THEN** the registration is named `iroh-tunnel-serve-e2e123.service` (or the label/task equivalent)

### Requirement: User service lifecycle
The system SHALL, for a user service, install and start, report status (running/stopped), stop, start, restart and uninstall through the host's user service manager, with the same node identity across stop/start/restart and the registration removed on uninstall.
Evidence: tests/service_user.rs (user_service_lifecycle_uses_the_pinned_absolute_paths, opt-in via IROH_TUNNEL_E2E_SERVICE=1), src/app.rs:330-352

#### Scenario: Full lifecycle
- **WHEN** a user service is installed, stopped, started, restarted and uninstalled
- **THEN** the status file reports the same node id after each start, is removed after stop, and the registration file no longer exists after uninstall

### Requirement: Start refused when the role already runs
The system SHALL refuse `service start` when the role's ownership check shows the role is already running, before invoking the service manager.
Evidence: src/app.rs:333-345
Confidence: inferred

#### Scenario: Role already running
- **WHEN** `service start` is invoked while the role holds its lock
- **THEN** the command fails from the ownership check rather than starting a second instance

### Requirement: System install resolves the invoking user, never root's home
The system SHALL resolve system-install ownership from a real non-root SUDO_USER, then an explicit config-directory owner, then an interactive username; unresolved ownership SHALL fail with exit 2, actionable guidance and no registration, never using root’s home.
Evidence: src/service/system.rs:53-160 (unit tests), src/app.rs:380-415, tests/service_system.rs

#### Scenario: No SUDO_USER and no terminal
- **WHEN** root with `HOME=/root` and no `SUDO_USER` runs `serve service install`
- **THEN** exit code is 2, stderr contains `--config` and `sudo iroh-tunnel`, no init script is created and nothing references `/root`

#### Scenario: Interactive prompt with unknown name
- **WHEN** the prompt answer names a user that does not exist
- **THEN** the command fails with a configuration error

### Requirement: SUDO_USER and --config must agree
The system SHALL fail with a configuration error naming both users when `SUDO_USER` and the owner of an explicit `--config`'s directory differ, so the service trusts exactly one user.
Evidence: src/service/system.rs:84-120, src/service/system.rs tests (sudo_user_and_a_config_owned_by_someone_else_is_a_named_conflict)

#### Scenario: Mismatched owner
- **WHEN** `SUDO_USER` is `daemon` but the config directory is owned by another account
- **THEN** the error names both users and mentions `--config`, and nothing is changed

### Requirement: System install uses the user's default config and state locations
The system SHALL use the resolved user’s default config/state paths when overrides are absent, canonicalize them and fail with user-run creation guidance if required paths are missing; root SHALL NOT create directories in that user’s home.
Evidence: src/service/system.rs:120-160, src/service/trust.rs:244-258, tests/service_system.rs (SM-PATH-01, symlinked home test)

#### Scenario: Registered paths are alice's
- **WHEN** root installs with `SUDO_USER=it91a` and `HOME=/root`
- **THEN** the registration contains alice's canonical config path and state dir, `IROH_TUNNEL_TRUSTED_UID=<alice uid>`, and no `/root/`

#### Scenario: Symlinked home
- **WHEN** the user's home is a symlink
- **THEN** the real path is pinned at install and the service runs through it

### Requirement: System install verifies files before registering
The system SHALL verify the state directory and the config through the trust policy and validate the config for the role before registering a system service, failing closed on an unsafe path.
Evidence: src/app.rs:395-405, src/service/trust.rs:212-240
Confidence: inferred

#### Scenario: Unsafe config at install
- **WHEN** the target config is group/world-writable at install time
- **THEN** install fails with a permission error and no registration is written

### Requirement: Registration pins the owner
The system SHALL record in every system registration a single-line comment `iroh-tunnel-pin config=<escaped path> uid=<n>` and the environment `IROH_TUNNEL_TRUSTED_UID=<uid>`, and SHALL treat a body without a parseable pin (comment lines only) as an unknown owner.
Evidence: src/service/pin.rs:1-65, src/service/systemd.rs:60-72, src/service/busybox.rs:55-65, src/service/launchd.rs:104-115, src/service/system.rs (unit tests), tests/service_system.rs

#### Scenario: Pin in the script
- **WHEN** alice's system service is installed on BusyBox
- **THEN** the script contains her canonical config path and `IROH_TUNNEL_TRUSTED_UID=<her uid>`

### Requirement: Retargeting another user's service needs confirmation
The system SHALL require interactive y/yes confirmation before retargeting a different or unknown system-service owner; absent confirmation SHALL fail with exit 5 and leave registration and configs unchanged.
Evidence: src/service/system.rs:180-245 (unit tests), src/service/mod.rs:350-365, tests/service_system.rs

#### Scenario: Ownership and consent rules
- **WHEN** a system registration exists
- **THEN** missing pins or unreadable registration mean unknown owner; same-owner reinstall needs no prompt, other owners require case-insensitive y/yes, and EOF, other answers or no terminal cannot authorize retargeting

#### Scenario: Declined retarget
- **WHEN** bob installs over alice's service and answers `n` or empty
- **THEN** exit code is 5, stderr contains `Retarget` and alice's name, the script is byte-identical, and both configs are unchanged

#### Scenario: Approved retarget
- **WHEN** bob answers `y`
- **THEN** the registration pins bob's uid and config and both configs are kept

#### Scenario: Same user again
- **WHEN** the same user installs again
- **THEN** install succeeds without a prompt

### Requirement: Privileged service refuses unsafe config paths
The system SHALL verify pinned config/state paths against root and the trusted uid, refuse unsafe ownership, modes, links or path forms, read only the verified config and never repair permissions implicitly.
Evidence: src/service/trust.rs:119-240,392-420 (unit tests), tests/service_system.rs (SM-TRUST-01)

#### Scenario: Trusted path policy
- **WHEN** `IROH_TUNNEL_TRUSTED_UID` enables privileged validation
- **THEN** no-follow handle-relative traversal from `/` rejects symlinks, non-regular files, relative/non-canonical paths, untrusted owners and writable files/parents, naming path, uid and mode; a sticky writable parent is accepted only for a trusted-owned child

#### Scenario: Writable config
- **WHEN** the config mode is 0666 and the service restarts
- **THEN** the log contains a `permission error` naming the config path and `0666`, and the service never reaches running

#### Scenario: Swapped parent
- **WHEN** a parent directory is replaced by a symlink or owned by another user
- **THEN** the service refuses with `symlink` or `untrusted` and owners and modes remain as set by the test

#### Scenario: Fixed condition
- **WHEN** the unsafe condition is removed and the service is restarted
- **THEN** the service runs again

### Requirement: Privileged service never writes a generated key
The system SHALL, under a pinned system service, fail with a configuration error telling the user to run `config keygen` as that user when the config has no `secret_key`, and SHALL report an invalid edited config without serving it and without changing its owner or mode.
Evidence: src/service/trust.rs:392-420, tests/service_system.rs (SM-EDIT-01)

#### Scenario: User edits config
- **WHEN** alice edits her config and the administrator restarts the service
- **THEN** the new services are served and the file keeps uid and mode 0600

#### Scenario: Invalid edit
- **WHEN** the config is invalid TOML at restart
- **THEN** the log contains `failed to parse`, the service does not run, and the file's owner and mode are unchanged

### Requirement: Root under a service manager without a pin is refused
The system SHALL refuse to run as root under systemd (`INVOCATION_ID`) or launchd (`XPC_SERVICE_NAME`) without `IROH_TUNNEL_TRUSTED_UID`, with a permission error (exit code 3) telling the operator to re-install with `service install --system`; a foreground root run without those variables is unaffected. A non-numeric `IROH_TUNNEL_TRUSTED_UID` is a configuration error.
Evidence: src/service/trust.rs:349-395 (unit tests), tests/service_system.rs

#### Scenario: Legacy systemd unit
- **WHEN** `serve run` as root has `INVOCATION_ID` set and no trusted uid
- **THEN** it exits with code 3 and stderr contains `service install --system`

### Requirement: Status snapshot publishing under a system service is link-safe
The system SHALL, under a pinned system service, write the status snapshot by re-verifying the state directory on every write, creating the temp file exclusively without following links and renaming it atomically, so a symlink planted by the user is replaced rather than written through, and stale temp files or a snapshot left by `kill -9` do not block restart.
Evidence: src/service/trust.rs:260-340 (unit tests), tests/service_system.rs

#### Scenario: Planted symlink
- **WHEN** the user plants `serve-status.json` as a symlink to a root-owned file and the service restarts
- **THEN** the symlink is replaced by a regular file and the victim file content is unchanged

### Requirement: Windows user background task
The system SHALL support Windows user background tasks only, replacing and starting a per-role logon task with least privilege and restart-on-failure; unsupported system-scope operations SHALL fail with service exit code 5 and `--user` guidance.
Evidence: src/service/scope.rs:60-82, src/service/windows.rs:1-150, src/service/task_xml.rs:85-125 (unit tests), tests/service_user_windows.rs

#### Scenario: Task execution policy
- **WHEN** `iroh-tunnel-<role>` is registered
- **THEN** it uses InteractiveToken, IgnoreNew, no execution limit, battery-insensitive settings and restart interval 5 seconds with count 999, then runs immediately

#### Scenario: System scope on Windows
- **WHEN** `serve service install --system --config <path>` runs on Windows
- **THEN** exit code is 5, stderr contains `not supported on Windows` and `--user`, and no task exists

#### Scenario: Task definition
- **WHEN** the task XML is rendered
- **THEN** it contains `<LogonTrigger>`, `InteractiveToken`, `LeastPrivilege` and not `HighestAvailable`

### Requirement: Windows install refuses a non-default state directory
The system SHALL refuse a Windows install when `IROH_TUNNEL_STATE_DIR` selects a directory other than the default (compared case- and separator-insensitively) with a configuration error (exit code 2) naming the variable, registering nothing.
Evidence: src/service/windows.rs:100-125, tests/service_user_windows.rs (install_refuses_a_state_directory_the_task_cannot_carry)

#### Scenario: Custom state directory
- **WHEN** `IROH_TUNNEL_STATE_DIR` points elsewhere and `service install --user` runs
- **THEN** exit code is 2, stderr mentions `IROH_TUNNEL_STATE_DIR`, and no task exists

### Requirement: Windows verbs on a missing task
The system SHALL fail Windows start/stop/status for missing tasks with service exit 5 and install guidance, treat already-stopped stop and absent uninstall as success, and use ownership locks rather than localized task text to detect a running role.
Evidence: src/service/windows.rs:60-100, src/service/windows.rs:170-279, tests/service_user_windows.rs (start_without_an_installed_task_names_the_install_command)

#### Scenario: Not installed
- **WHEN** `serve service start|stop|status --user` runs with no task
- **THEN** each exits with code 5 and stderr contains `service install`

### Requirement: Windows restart waits for the old process
The system SHALL on Windows restart by stopping the task, waiting up to 15 seconds for the role's lock to be released, and then starting it; if the old process has not exited it SHALL fail with a service error and not start a second instance.
Evidence: src/service/windows.rs:206-228
Confidence: inferred

#### Scenario: Slow stop
- **WHEN** the role is still running 15 seconds after `schtasks /End`
- **THEN** restart fails saying the task did not stop and does not start again

### Requirement: Windows access-denied guidance
The system SHALL turn an English "access is denied" failure while registering the Windows task into a permission error stating the scope and account were not changed and nothing was registered.
Evidence: src/service/windows.rs:170-195
Confidence: inferred

#### Scenario: Access denied
- **WHEN** `schtasks /Create` fails with "Access is denied"
- **THEN** a permission error is returned instead of the raw schtasks failure
