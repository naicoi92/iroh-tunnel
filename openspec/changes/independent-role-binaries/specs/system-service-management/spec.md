## REMOVED Requirements

### Requirement: Service management refused for standalone role binaries

**Reason**: Role executables now manage their own services.

**Migration**: Invoke the standalone role's service commands directly; existing combined commands remain valid.

## MODIFIED Requirements

### Requirement: systemd unit content and lifecycle
The system SHALL install a role systemd unit with pinned config/state paths, restart-on-failure after 5 seconds except exit 6, and reload, enable and restart it; user operations SHALL use `systemctl --user`. A re-install SHALL restart the unit even when it is running, the install-time conflict refusal applying only across scopes.

#### Scenario: Unit registration shape
- **WHEN** a unit is generated
- **THEN** `iroh-tunnel-<role>.service` lives under `/etc/systemd/system` or `~/.config/systemd/user`, runs `<binary> <role> run --config <config>` for the combined binary or `<binary> run --config <config>` for its standalone role, sets `IROH_TUNNEL_STATE_DIR` and is wanted by `default.target`

#### Scenario: Unit text
- **WHEN** a serve unit is rendered for `/usr/bin/iroh-tunnel` and config `/etc/cfg`
- **THEN** it contains `ExecStart=/usr/bin/iroh-tunnel serve run --config /etc/cfg`, `Restart=on-failure`, `RestartSec=5` and `RestartPreventExitStatus=6`

#### Scenario: Re-install over a running unit
- **WHEN** the service is installed again in the scope that already holds it
- **THEN** the unit is restarted so the new registration takes effect

### Requirement: BusyBox init script content and lifecycle
The system SHALL install and restart a mode-0755 BusyBox role init script with pinned config/state, start/stop/restart/status support, and a SIGTERM then 5-second SIGKILL stop policy.

#### Scenario: BusyBox paths and stopped status
- **WHEN** a BusyBox role script is generated
- **THEN** it uses `/etc/init.d/S96iroh-tunnel-<role>`, `/var/run/iroh-tunnel-<role>.pid`, `/var/log/iroh-tunnel-<role>.log`, exports `IROH_TUNNEL_STATE_DIR`, runs `<binary> <role> run --config <config>` for the combined binary or `<binary> run --config <config>` for its standalone role in background and returns status 3 when stopped

#### Scenario: Script verbs
- **WHEN** the script is rendered
- **THEN** it begins with `#!/bin/sh`, embeds DAEMON, CONFIG, PIDFILE and LOG, and wires `start`, `stop`, `restart` (stop then start) and `status`

### Requirement: Windows system service registration
The system SHALL install `--system` on Windows as an own-process Windows Service named `iroh-tunnel-<role>` (plus the test-only suffix), with automatic start and the LocalSystem account. Its command line SHALL be `"<binary>" <role> run --config "<config>"` for the combined binary and `"<binary>" run --config "<config>"` for its standalone role, and its service environment SHALL carry the pinned state directory and the pinned SID.

#### Scenario: Service definition
- **WHEN** `serve service install --system` succeeds on Windows using the combined binary
- **THEN** a service `iroh-tunnel-serve` exists with automatic start, LocalSystem account and the quoted `serve run --config` command line, and it is running

#### Scenario: Standalone service definition
- **WHEN** a standalone role binary installs a Windows system service
- **THEN** the service command invokes that binary with `run --config` and no role argument

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
