## MODIFIED Requirements

### Requirement: Locked executable on Windows fails cleanly
The system SHALL, on Windows, report the OS error and keep the old executable when the target is locked by a running process, including the updater itself. The message SHALL add guidance that a system service using the executable must first be stopped from an elevated terminal (Run as administrator), naming both concrete commands, `iroh-tunnel serve service stop` and `iroh-tunnel access service stop`, so that either can be pasted into an elevated PowerShell or cmd as is. No rollback is promised, and the updater SHALL NOT stop any process itself.

#### Scenario: Running image
- **WHEN** another process runs the target exe on Windows
- **THEN** the update fails and the exe remains usable

#### Scenario: Guidance for a system service
- **WHEN** the update fails because the executable is locked on Windows
- **THEN** stderr contains `Run as administrator`, `iroh-tunnel serve service stop` and `iroh-tunnel access service stop`, contains no `--system` and no `<role>` placeholder, and no service or process was stopped by the updater
