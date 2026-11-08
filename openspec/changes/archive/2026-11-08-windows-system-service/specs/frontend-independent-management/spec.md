## ADDED Requirements

### Requirement: No elevation, account switch or scope fallback
Service operations MUST NOT self-elevate, switch accounts, fall back to another scope or substitute a different platform service mechanism, on any supported platform.
Source: design.md Decision 4 (refactor-codebase); windows-system-service design.md D8

#### Scenario: Windows system scope without elevation
- **WHEN** a non-elevated caller requests a system-scope operation on Windows
- **THEN** the operation returns a permission-classified error with elevated-terminal rerun guidance, and creates no Windows Service, scheduled task or user task

#### Scenario: Windows system scope uses only a Windows Service
- **WHEN** an elevated caller installs system scope on Windows
- **THEN** only a Windows Service is registered, and no scheduled task is created as a substitute

## MODIFIED Requirements

### Requirement: Service operations enforce existing policy
The host SHALL provide install, uninstall, start, stop, restart and status operations taking a typed role, scope and resolved paths. They MUST use the same backend selection and privilege, account, pin/trust, installability and ownership policies as the CLI; raw backend dispatch SHALL NOT be public.
Source: design.md Decision 4; openspec/specs/system-service-management

#### Scenario: Unprivileged system scope
- **WHEN** an unprivileged caller requests install with system scope
- **THEN** the operation returns a permission-classified error whose guidance names the `sudo` rerun command and absolute config path, and no registration is written

#### Scenario: Non-elevated Windows system scope
- **WHEN** a non-elevated caller requests install with system scope on Windows
- **THEN** the operation returns a permission-classified error whose guidance names the elevated-terminal rerun and absolute config path, and no service is registered

#### Scenario: BusyBox user scope unchanged
- **WHEN** a caller requests a user-scope operation on a host where the BusyBox/SysV backend is selected
- **THEN** the operation behaves as the existing BusyBox policy specifies and does not invent per-user support

## REMOVED Requirements

### Requirement: No elevation or scope fallback

**Reason**: Its only scenario stated that Windows remains user-only, which this change reverses.

**Migration**: The same rule continues as "No elevation, account switch or scope fallback", whose Windows scenarios cover system scope.
