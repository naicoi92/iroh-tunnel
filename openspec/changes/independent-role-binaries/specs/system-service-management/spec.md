## REMOVED Requirements

### Requirement: Service management refused for standalone role binaries

**Reason**: Role executables now manage their own services.

**Migration**: Invoke the standalone role's service commands directly; existing combined commands remain valid.

## MODIFIED Requirements

### Requirement: Binary resolution for registrations
The system SHALL resolve the combined binary as the `iroh-tunnel` found on `PATH` and otherwise the absolute path of the current executable. For a standalone invocation it SHALL resolve the current standalone executable without relying on a combined executable. A user-scope registration on systemd, launchd or Windows SHALL register that resolved path. A root-run registration on Linux or macOS SHALL register the binary chosen by `unix-service-binary-trust` ("System install registers a trusted binary or a managed copy"): the canonical resolved path when it passes the root-only binary check, otherwise the role's managed copy. A Windows system service SHALL register the resolved path, checked as `windows-service-trust` specifies. Registrations and managed copies SHALL invoke the executable with arguments appropriate to its actual combined or standalone entry point.

#### Scenario: Not on PATH
- **WHEN** `iroh-tunnel` is not on `PATH` (for example `cargo run`) and a user service is installed
- **THEN** the current executable's absolute path is registered

#### Scenario: Root-run registration from a user-writable binary
- **WHEN** a system service is installed on Linux or macOS and the resolved binary fails the root-only binary check
- **THEN** the registration names the role's managed copy instead of the resolved path

#### Scenario: Standalone executable alone
- **WHEN** a standalone role executable installs a supported service without the combined executable present
- **THEN** the registration invokes that standalone executable or its trusted managed copy using its role-specific argument form

#### Scenario: Windows standalone service startup
- **WHEN** Windows starts a standalone system service through SCM
- **THEN** the correct role enters service context and follows existing readiness and stop rules
