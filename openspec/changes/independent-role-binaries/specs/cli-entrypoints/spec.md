## REMOVED Requirements

### Requirement: Standalone binaries refuse service management

**Reason**: Standalone binaries now support service management for their own role.

**Migration**: Use `its service ...` or `ita service ...` directly; existing combined commands remain supported.

## ADDED Requirements

### Requirement: Standalone role service commands
The system SHALL allow each standalone executable to perform its own role's service lifecycle operations without the combined executable and SHALL retain the other-role rejection.

#### Scenario: Service operations without combined executable
- **WHEN** `its` or `ita` manages its role service with no combined executable installed
- **THEN** the operation follows the corresponding role's existing service policy rather than refusing standalone operation

## MODIFIED Requirements

### Requirement: Exit status mapping
The system SHALL exit 0 on success. On failure it SHALL exit:
- 2 for usage errors (with help, as specified) and config errors;
- 1 for unclassified errors;
- 3 for permission errors;
- 4 for Iroh endpoint errors;
- 5 for service errors;
- 6 for conflicts (role or identity already running).

Every failure SHALL print `error: <message>` to stderr.

#### Scenario: Usage error
- **WHEN** a quick-mode required flag is missing
- **THEN** exit code is 2

#### Scenario: Config error
- **WHEN** `config list` runs on an invalid config file
- **THEN** exit code is 2

#### Scenario: Service error
- **WHEN** a service operation encounters a service error
- **THEN** exit code is 5
