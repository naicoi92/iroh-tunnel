## MODIFIED Requirements

### Requirement: Explicit lifecycle operations keep composite semantics
Only explicit stop, restart or uninstall SHALL change service lifetime. Install SHALL still enable and start; uninstall SHALL still stop, disable and remove; start SHALL keep its ownership refusal before the manager call. A failed operation SHALL return the effects already applied, recorded as each one happens. The system MUST NOT perform a compensating rollback unless a backend requirement demands it (the Windows system install, per `system-service-management` "Windows system install replaces and rolls back"); when it does, the rollback's own effects SHALL be returned too. On the Windows system service, a created service counts as enabled and a deleted one as disabled.
Source: design.md Decision 4; openspec/specs/system-service-management

#### Scenario: Start refused when the role already runs
- **WHEN** a caller requests start while the Serve role is already running for that user
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
