## ADDED Requirements

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

## MODIFIED Requirements

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
