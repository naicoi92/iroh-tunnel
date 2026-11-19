## ADDED Requirements

### Requirement: Service operations never need more input
Service operations SHALL NOT prompt or guess; every input they need SHALL be part of the request, so no service operation returns a need. Execution MUST recheck privilege, ownership, trust and registration facts before mutating.
Source: refactor-codebase design.md Decision 4

#### Scenario: Install completes without a need
- **WHEN** a caller requests install with a valid config
- **THEN** the operation returns a report or a failure, never a request for more input

### Requirement: Service operations derive the tier from privilege
The host SHALL provide install, uninstall, start, stop, restart and status operations taking a typed role, the caller's injected privilege and resolved paths, deriving the tier from that privilege and taking no scope. They MUST use the CLI's backend selection, trust, installability and ownership policies, keep raw backend dispatch private, and return the other-tier notice as a structured service note.
Source: refactor-codebase design.md Decision 4; openspec/specs/system-service-management

#### Scenario: Unprivileged caller
- **WHEN** an unprivileged caller with a valid user-tier config requests install on a systemd, launchd or Windows host
- **THEN** a user-tier registration is written and nothing system-wide is written

#### Scenario: Elevated caller on Windows
- **WHEN** an elevated caller requests install on Windows
- **THEN** a Windows Service is registered from the system-tier config

#### Scenario: BusyBox without privilege
- **WHEN** an unprivileged caller requests a mutating operation on a host where the BusyBox/SysV backend is selected
- **THEN** the operation returns a permission-classified error as `system-service-management` "BusyBox backend needs root" specifies and does not invent per-user support

#### Scenario: Other tier note
- **WHEN** a caller installs while the role runs in the other tier
- **THEN** the report carries an other-tier-running note and the install proceeds

## MODIFIED Requirements

### Requirement: No elevation, account switch or scope fallback
Service operations MUST NOT self-elevate, switch accounts, act in a tier other than the caller's, or substitute a different platform service mechanism, on any supported platform.
Source: design.md Decision 4 (refactor-codebase); windows-system-service design.md D8

#### Scenario: Windows system scope without elevation
- **WHEN** a non-elevated caller requests install on Windows
- **THEN** only the caller's logon task is registered, and no Windows Service is created or changed

#### Scenario: Windows system scope uses only a Windows Service
- **WHEN** an elevated caller installs on Windows
- **THEN** only a Windows Service is registered, and no scheduled task is created as a substitute

### Requirement: Explicit lifecycle operations keep composite semantics
Only explicit stop, restart or uninstall SHALL change service lifetime. Install SHALL still enable and start; uninstall SHALL still stop, disable and remove; start SHALL keep its same-tier ownership refusal before the manager call. A failed operation SHALL return the effects already applied, recorded as each one happens. The system MUST NOT perform a compensating rollback unless a backend requirement demands it (the Windows system install, per `system-service-management` "Windows system install replaces and rolls back"); when it does, the rollback's own effects SHALL be returned too. On the Windows system service, a created service counts as enabled and a deleted one as disabled.
Source: design.md Decision 4; openspec/specs/system-service-management

#### Scenario: Start refused when the role already runs
- **WHEN** a caller requests start while the Serve role is already running in the caller's tier
- **THEN** the operation returns a conflict-classified refusal before calling the service manager

#### Scenario: Partial install reports applied effects
- **WHEN** a registration write succeeds but enabling or starting fails on a backend without a rollback requirement
- **THEN** the operation returns the failure with the effects already applied and performs no compensating rollback

#### Scenario: Windows system reinstall fails after deleting the old service
- **WHEN** a Windows system install stops and deletes the role's existing service and then fails to create the new one
- **THEN** the operation returns the failure with stopped and disabled among the applied effects

#### Scenario: Windows system install rolls back a service stuck starting
- **WHEN** a Windows system install creates and starts the service, the service is still start-pending when the install stops waiting for running, and the install rolls it back
- **THEN** the operation returns the failure with enabled, started, stopped and disabled, in that order, as the applied effects

#### Scenario: Windows system install rolls back a service that stopped by itself
- **WHEN** a Windows system install creates and starts the service and the service stops by itself before running
- **THEN** the operation returns the failure with enabled, started and disabled, in that order, and no stopped effect

## REMOVED Requirements

### Requirement: Insufficient information yields a structured need

**Reason**: No scope, invoking-user or retarget input remains; replaced by "Service operations never need more input".

**Migration**: Frontends drop need handling for service operations.

### Requirement: Service operations enforce existing policy

**Reason**: Service operations no longer take a scope; replaced by "Service operations derive the tier from privilege".

**Migration**: Frontends pass the caller's privilege instead of a scope.

### Requirement: Retarget confirmation is specific and fresh

**Reason**: Retargeting no longer exists; a system service is not owned by a user.

**Migration**: Frontends drop retarget handling.

