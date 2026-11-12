## MODIFIED Requirements

### Requirement: Binary resolution for registrations
The system SHALL resolve the binary as the `iroh-tunnel` found on `PATH` and otherwise the absolute path of the current executable. A user-scope registration on systemd, launchd or Windows SHALL register that resolved path. A root-run registration on Linux or macOS SHALL register the binary chosen by `unix-service-binary-trust` ("System install registers a trusted binary or a managed copy"): the canonical resolved path when it passes the root-only binary check, otherwise the role's managed copy. A Windows system service SHALL register the resolved path, checked as `windows-service-trust` specifies.
Evidence: src/cli_service.rs (resolve_tunnel_executable_with tests), crates/host/src/service/install.rs

#### Scenario: Not on PATH
- **WHEN** `iroh-tunnel` is not on `PATH` (for example `cargo run`) and a user service is installed
- **THEN** the current executable's absolute path is registered

#### Scenario: Root-run registration from a user-writable binary
- **WHEN** a system service is installed on Linux or macOS and the resolved binary fails the root-only binary check
- **THEN** the registration names the role's managed copy instead of the resolved path
