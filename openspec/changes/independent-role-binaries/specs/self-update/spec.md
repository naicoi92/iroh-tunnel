## MODIFIED Requirements

### Requirement: Locked executable on Windows fails cleanly
The system SHALL, on Windows, report the OS error and keep the old executable when the target is locked by a running process, including the updater itself. The message SHALL give concrete elevated system-service stop commands executable using the invoked binary: both role commands for the combined executable, or the matching standalone role command for a standalone executable. No rollback is promised, and the updater SHALL NOT stop any process itself.

#### Scenario: Running image
- **WHEN** another process runs the target exe on Windows
- **THEN** the update fails and the exe remains usable

#### Scenario: Guidance for a system service
- **WHEN** the combined executable's update fails because the executable is locked on Windows
- **THEN** stderr contains `iroh-tunnel serve service stop --system` and `iroh-tunnel access service stop --system` and no `<role>` placeholder, and no service or process was stopped by the updater

#### Scenario: Standalone guidance for a system service
- **WHEN** a standalone role executable's update fails because the executable is locked on Windows
- **THEN** stderr names the matching standalone binary's concrete `service stop --system` command without requiring the combined executable and does not stop any process

### Requirement: Managed installs are updated through their owner
The system SHALL direct package/Homebrew installs to their owner and exit unchanged unless `--yes` is given; `--yes` SHALL warn before replacing only the invoked file, and `--insecure` alone SHALL NOT override ownership. Homebrew guidance SHALL name the cask corresponding to the invoked executable.

#### Scenario: Default
- **WHEN** a package-owned file is updated without `--yes`
- **THEN** guidance is printed and the file is unchanged

#### Scenario: Insecure only
- **WHEN** `--insecure` is given without `--yes` for a package-owned file
- **THEN** the file is unchanged

#### Scenario: Confirmed
- **WHEN** `--yes` is given for a package or Homebrew owned file
- **THEN** a warning is printed on stderr and only that file is replaced

#### Scenario: Role-specific Homebrew guidance
- **WHEN** a Homebrew-owned standalone executable is updated without `--yes`
- **THEN** the file is unchanged and guidance names its own cask
