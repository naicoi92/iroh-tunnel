## RENAMED Requirements

- FROM: `### Requirement: Start refused when the role already runs`
- TO: `### Requirement: Start and conflicting install refused when the role already runs`

---

## MODIFIED Requirements

### Requirement: Start and conflicting install refused when the role already runs
The system SHALL refuse `service start` when the role's ownership check shows the role is already running, before invoking the service manager. The system SHALL refuse `service install`, on every platform and before any registration is written, when the role is held at the lock tier of the scope the install will not register in: the machine-wide tier for an install registering a user service, the per-user tier for one registering a system service. The tier compared SHALL be that of the scope the install effectively registers in, so a backend that serves a user-scope request with a system-wide registration compares the machine-wide tier. A role held at the tier matching the registering scope SHALL NOT refuse the install, so re-installing over the same scope's own service stays supported. A system install SHALL compare the per-user tier of the account it resolved, never root's own. A refusal SHALL be a conflict error naming the holding tier, how to stop it, and how to remove the service in the conflicting scope, and SHALL NOT stop, remove or migrate that service.
Evidence: crates/host/src/service/install.rs (plan_install pre-checks), crates/host/src/service/verb.rs, crates/host/src/ownership.rs (Tier, probe)

#### Scenario: Role already running
- **WHEN** `service start` is invoked while the role holds its lock
- **THEN** the command fails from the ownership check rather than starting a second instance

#### Scenario: User install while a system service of the role runs
- **WHEN** `service install --user` is invoked while a service of the same role holds the machine-wide lock
- **THEN** the command fails with a conflict exit status, no registration is written, stderr names the machine-wide tier and the command that removes the system service, and the running service keeps running

#### Scenario: System install while a user service of the role runs
- **WHEN** `service install --system` is invoked, with privilege, while a service of the same role holds the per-user lock
- **THEN** the command fails with a conflict exit status, no registration is written, and the running service keeps running

#### Scenario: Re-install over the same scope's own running service
- **WHEN** `service install --user` is invoked while the role is held only at the per-user tier
- **THEN** the install proceeds and the registration is replaced, as for an install on a host where the role does not run

#### Scenario: Privilege is reported before the conflict
- **WHEN** `service install --system` is invoked without privilege while a user service of the role runs
- **THEN** the command fails with the permission error for an unprivileged system install, not with the conflict error

#### Scenario: User-scope re-install on a backend with no per-user services
- **WHEN** `service install --user` is invoked as root on a BusyBox host while the role holds the machine-wide lock
- **THEN** the install proceeds, the system-wide script is replaced and its explanatory note is printed, because the machine-wide tier is the tier this install registers in

### Requirement: launchd registration and domains
The system SHALL install a labelled launchd role job in the appropriate user/system domain, with pinned command and state paths, RunAtLoad and restart on unsuccessful exit; the plist SHALL declare Apple's property-list document type. Loading SHALL issue `launchctl enable` for the job label in the target domain, then boot out the old job, then bootstrap the plist; a failure to enable SHALL fail the operation, while a failure booting out an old job SHALL be ignored.
Evidence: crates/host/src/service/backend/launchd.rs (format_plist_with, load)

#### Scenario: launchd registration shape
- **WHEN** a launchd job is registered
- **THEN** `dev.iroh-tunnel.<role>` uses `~/Library/LaunchAgents` with `gui/<uid>` or `/Library/LaunchDaemons` with `system`, XML-escaped arguments and `IROH_TUNNEL_STATE_DIR`

#### Scenario: Plist document type
- **WHEN** a launchd plist is rendered
- **THEN** its document type declaration references `PropertyList-1.0.dtd` and does not reference `PropertyManifest-1.0.dtd`

#### Scenario: Load command order
- **WHEN** a launchd job is loaded
- **THEN** the commands issued are `enable <service-target>`, then `bootout <service-target>`, then `bootstrap <domain> <plist>`, in that order

#### Scenario: Re-install over a loaded job
- **WHEN** the service is installed while already loaded
- **THEN** the job label is enabled, the existing job is booted out, and the new plist is bootstrapped

#### Scenario: Enabling the label fails
- **WHEN** `launchctl enable` fails for the target
- **THEN** the operation fails reporting that command, and no bootstrap is attempted

#### Scenario: Booting out a job that is not loaded
- **WHEN** `launchctl bootout` fails because no job is registered
- **THEN** the failure is ignored, its output is not shown, and the load continues to bootstrap

### Requirement: launchd verbs and idempotent stop
The system SHALL start with `kickstart`, restart with `kickstart -k`, stop with `launchctl kill SIGTERM` treating "no process to signal" (exit 3) as success and printing that it is already stopped, and when the job is not loaded SHALL report that no service is loaded in the current scope domain with an install hint naming the other scope flag. A failed bootstrap SHALL print diagnostic guidance naming the domain's disable list among the things to inspect.
Evidence: crates/host/src/service/backend/launchd.rs, src/render.rs
Confidence: inferred

#### Scenario: Stop an already stopped job
- **WHEN** `service stop` runs and launchd reports no process to signal
- **THEN** the command succeeds and prints `<label> is already stopped.`

#### Scenario: Bootstrap fails
- **WHEN** a bootstrap fails during install
- **THEN** the guidance printed to stderr contains `launchctl print-disabled` with the target domain

### Requirement: Restart policy is shared across platforms
The system SHALL apply one default restart policy (restart on failure, 5 second delay) to every backend; launchd ignores the delay. The systemd unit SHALL exempt the role's "already running" exit status from restart. The launchd job SHALL restart on unsuccessful exit only and SHALL NOT carry an exit-status exemption, because launchd's restart condition cannot express one; a macOS job that exits because the role is already running is therefore respawned until it is booted out. The install-time conflict refusal keeps that off the `service install` path for a cross-scope conflict, but not where the role already runs in the scope being installed, nor for a job loaded by other means.
Evidence: crates/host/src/service/types.rs (RestartPolicy), crates/host/src/service/backend/systemd.rs (format_unit), crates/host/src/service/backend/launchd.rs (format_plist_with)

#### Scenario: Policy disabled
- **WHEN** a unit is rendered with restart-on-failure disabled
- **THEN** it contains `Restart=no`

#### Scenario: systemd exempts the refused-start status
- **WHEN** a systemd unit is rendered with the default policy
- **THEN** it contains `RestartPreventExitStatus=6`

#### Scenario: launchd carries no exit-status exemption
- **WHEN** a launchd plist is rendered with the default policy
- **THEN** its `KeepAlive` dictionary sets `SuccessfulExit` to false and contains no exit-status exemption, there being no launchd key that expresses one

### Requirement: systemd unit content and lifecycle
The system SHALL install a role systemd unit with pinned config/state paths, restart-on-failure after 5 seconds except exit 6, and reload, enable and restart it; user operations SHALL use `systemctl --user`. A re-install SHALL restart the unit even when it is running, the install-time conflict refusal applying only across scopes.
Evidence: crates/host/src/service/backend/systemd.rs (unit tests), crates/host/src/service/types.rs (RestartPolicy tests)

#### Scenario: Unit registration shape
- **WHEN** a unit is generated
- **THEN** `iroh-tunnel-<role>.service` lives under `/etc/systemd/system` or `~/.config/systemd/user`, runs `<binary> <role> run --config <config>`, sets `IROH_TUNNEL_STATE_DIR` and is wanted by `default.target`

#### Scenario: Unit text
- **WHEN** a serve unit is rendered for `/usr/bin/iroh-tunnel` and config `/etc/cfg`
- **THEN** it contains `ExecStart=/usr/bin/iroh-tunnel serve run --config /etc/cfg`, `Restart=on-failure`, `RestartSec=5` and `RestartPreventExitStatus=6`

#### Scenario: Re-install over a running unit
- **WHEN** the service is installed again in the scope that already holds it
- **THEN** the unit is restarted so the new registration takes effect
