## MODIFIED Requirements

### Requirement: Machine-tier lock between system service and user processes
The system SHALL prevent a system process and a user process of one role from running at the same time, using a machine lock that its creator holds exclusively and other processes hold shared, regardless of start order. The tier SHALL exist on Linux, macOS and Windows.

#### Scenario: Machine lock location
- **WHEN** a machine-tier lock is resolved
- **THEN** its path is `/run/iroh-tunnel/<role>.lock` on Linux, `/var/run/iroh-tunnel/<role>.lock` on macOS and `%ProgramData%\iroh-tunnel\<role>.lock` on Windows

#### Scenario: System lock held, user start
- **WHEN** the machine lock is held exclusively on a read-only file and a user serve is started
- **THEN** it exits with code 6 and stderr contains "system service"

#### Scenario: User running, system start
- **WHEN** a user serve process is running
- **THEN** the machine lock cannot be taken exclusively by another holder because the user process holds it shared

#### Scenario: Windows user task while the system service runs
- **WHEN** the Windows system service of a role is running and the same role is started in the logged-in user's session
- **THEN** the user start exits with code 6 and stderr contains "system service"

### Requirement: Stop signals begin a graceful drain
The system SHALL treat SIGINT (Ctrl+C), SIGTERM (SIGINT only on non-Unix) and, for a Windows system service, the Service Control Manager's stop, shutdown and pre-shutdown requests as stop requests. It SHALL log which one was received, and begin shutdown by stopping admission before draining active channels.

#### Scenario: SIGTERM
- **WHEN** a role receives SIGTERM
- **THEN** it logs "received SIGTERM" and starts the drain

#### Scenario: Service stop request
- **WHEN** a Windows system service receives a stop request from the Service Control Manager
- **THEN** it logs the stop request and starts the same drain
