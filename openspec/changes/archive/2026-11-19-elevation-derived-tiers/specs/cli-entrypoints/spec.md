## ADDED Requirements

### Requirement: Service verbs follow the invoking tier
The system SHALL offer `service install|uninstall|start|stop|restart|status` with no scope flag, acting in the invoking tier as `system-service-management` "Service tier follows privilege" specifies. The CLI SHALL NOT prompt for a scope, a user name or a retarget confirmation and SHALL never elevate itself.
Evidence: src/cli.rs:280-333, src/app.rs:273-349, src/app.rs:354-378
Confidence: inferred

#### Scenario: Scope flags are rejected
- **WHEN** `service install --system` or `service stop --user` is given
- **THEN** parsing fails with an unexpected-argument error

#### Scenario: No prompt for a non-elevated user
- **WHEN** a non-elevated user with a valid user-tier config runs `service install` on a terminal of a systemd, launchd or Windows host
- **THEN** a user service is installed and nothing is read from stdin

## MODIFIED Requirements

### Requirement: Consistent short options across roles
The system SHALL use the same short letter for the same option in every role form and every command, with no duplicate short option within one command:
- `-c` config
- `-n` name
- `-t` target
- `-l` listen
- `-N` node-id
- `-p` protocol
- `-r` relay-url
- `-k` key-file
- `-y` yes

`--reveal`, `--print`, `--relay-token`, `--relay-token-file` and `--clear-relays` SHALL have no short form.
Evidence: src/cli.rs:386-417 (unit test), tests/role_binaries.rs:463-554

#### Scenario: Short letters do not collide
- **WHEN** the command definitions of `it`, `its`, `ita`, `serve`, `access` and all their subcommands are built
- **THEN** no short option maps to two long options, and each letter above maps to its long option wherever it appears

#### Scenario: Long-only options
- **WHEN** the same definitions are inspected
- **THEN** `reveal`, `print`, `relay-token`, `relay-token-file` and `clear-relays` have no short letter, and no command defines `system` or `user`

### Requirement: Service install pins absolute paths and validates config
The system SHALL pin an absolute config path and absolute state directory (`IROH_TUNNEL_STATE_DIR` or the tier's default) into an installed service and SHALL refuse to install when the config is missing or invalid for the role, with a config error suggesting `config keygen` or `--config`.
Evidence: src/app.rs:236-251,310-328,427-468
Confidence: inferred

#### Scenario: Invalid config
- **WHEN** `service install` is run with a missing or invalid config
- **THEN** it fails with "cannot install the service" and registers nothing

### Requirement: Service start refuses a running role
The system SHALL refuse `service start` when the role is already running in the invoking tier, before asking the service manager to start it.
Evidence: src/app.rs:333-344
Confidence: inferred

#### Scenario: Role already running
- **WHEN** `service start` is invoked while the role holds ownership in the invoking tier
- **THEN** it fails (Conflict, exit 6) and the service is not started

### Requirement: Status command
The system SHALL provide a `status` command per role that prints the status table of the invoking tier's role, or the raw file with `--json`, preceded on stderr by the other-tier notice when the role also runs in the other tier.
Evidence: src/cli.rs:202-207, src/app.rs:208, src/status_cmd.rs:414-484 (unit tests)
Confidence: inferred

#### Scenario: Status JSON
- **WHEN** `status --json` is run
- **THEN** the status file is printed verbatim on stdout

#### Scenario: Status from the system tier
- **WHEN** `sudo iroh-tunnel serve status` runs while the system-tier serve is running
- **THEN** the system-tier status snapshot is shown

## REMOVED Requirements

### Requirement: Service scope selection and privilege

**Reason**: Scope flags and prompts are removed; replaced by "Service verbs follow the invoking tier".

**Migration**: Drop `--user`/`--system`; use `sudo` or an elevated terminal for the system service.
