# cli-entrypoints Specification

## Purpose

TBD

## Requirements

### Requirement: Three executables share one command surface
The system SHALL provide three executables, `iroh-tunnel` (alias `it`, both roles), `iroh-tunnel-serve` (alias `its`, Serve only) and `iroh-tunnel-access` (alias `ita`, Access only), each of which parses its own arguments and runs without needing another binary.
Evidence: src/main.rs:3-5, src/bin_serve.rs:3-5, src/bin_access.rs:3-5, src/app.rs:23-36, src/cli.rs:19,75-77,109-111, tests/role_binaries.rs:265-310

#### Scenario: All-in-one binary runs both roles
- **WHEN** `it serve` and `it access` are started in quick mode against each other
- **THEN** bytes are forwarded end to end

#### Scenario: Standalone binaries run by alias or full name
- **WHEN** `its`/`ita` (short options) or `iroh-tunnel-serve`/`iroh-tunnel-access` (long options) are started
- **THEN** the tunnel forwards bytes in each form

### Requirement: Standalone binaries take role options without a role subcommand
The system SHALL let the standalone Serve and Access binaries accept the role's quick options and role commands (`run`, `config`, `status`, `service`, `update`) directly, and SHALL reject the other role's name as an argument.
Evidence: src/cli.rs:75-141, tests/role_binaries.rs:574-655

#### Scenario: Other role name rejected
- **WHEN** `its access` or `ita serve` is invoked
- **THEN** the command fails with a diagnostic naming the unexpected word and writes nothing to the home directory

### Requirement: Top-level options and help
The system SHALL accept `-v/--verbose` (repeatable), `-q/--quiet` and `--color` (default `auto`) on every executable, and SHALL print help and version (`-V`) successfully, with help listing the roles, role commands, quick options (with short forms), `--relay-url`, the `[default: tcp]` protocol and the aliases `it`, `its`, `ita`.
Evidence: src/cli.rs:17-35,76-92,143-186, tests/role_binaries.rs:463-554

#### Scenario: Help lists options and aliases
- **WHEN** `--help` is run on `it`, `it serve`, `its`, `it access` or `ita`
- **THEN** it exits successfully and lists `-n/--name`, `-p/--protocol`, `-k/--key-file`, `--relay-url`, role commands and the role's alias (Serve adds `-t/--target`; Access adds `-l/--listen` and `-N/--node-id`)

#### Scenario: Version on standalone binaries
- **WHEN** `its -V` or `ita -V` is run
- **THEN** it exits successfully

### Requirement: Consistent short options across roles
The system SHALL use the same short letter for the same option in every role form, with no duplicate short option within one command: `-n` name, `-p` protocol, `-k` key-file.
Evidence: src/cli.rs:386-417 (unit test), tests/role_binaries.rs:463-554

#### Scenario: Short letters do not collide
- **WHEN** the command definitions of `it`, `its`, `ita`, `serve` and `access` are built
- **THEN** no short option maps to two long options, and `n`, `p`, `k` map to `name`, `protocol`, `key-file`

### Requirement: Role-specific quick options are rejected by the other role
The system SHALL reject Access-only quick options (`-l/--listen`, `-N/--node-id`) in Serve and Serve-only options (`-t/--target`) in Access.
Evidence: src/cli.rs:143-186, tests/role_binaries.rs:574-640

#### Scenario: Wrong-role flag
- **WHEN** `serve -n echo -t 127.0.0.1:1 --listen 127.0.0.1:1` (or `access ... --target ...`) is run
- **THEN** the command fails, stderr names the offending flag, and no files are created in the home directory

### Requirement: Quick mode forwards a single service from flags
The system SHALL, when a role is invoked without a role command, run a single-service tunnel built only from flags: Serve needs `--name` and `--target IP:PORT`; Access needs `--name`, `--listen IP:PORT` and `--node-id`; protocol is `tcp` by default or `udp`; `--relay-url` is optional; IPv4 and IPv6 addresses are accepted.
Evidence: src/app.rs:120-175, tests/quick_run.rs:230-330, tests/quick_run.rs:608-640

#### Scenario: TCP quick forwarding
- **WHEN** quick Serve targets a TCP echo address and quick Access listens locally with Serve's node id
- **THEN** bytes sent to the Access listener reach the target and the reply returns

#### Scenario: UDP quick forwarding
- **WHEN** both quick roles are started with `--protocol udp`
- **THEN** datagrams are forwarded to the fixed target

#### Scenario: IPv6 addresses
- **WHEN** the target or listen address is an IPv6 socket address
- **THEN** TCP forwarding works

#### Scenario: Short and long flags equivalent
- **WHEN** one side uses short flags and the other long flags
- **THEN** both reach the same fixed target

### Requirement: Quick mode input validation
The system SHALL reject invalid quick-mode input with exit status 2 and an actionable diagnostic before opening listeners or writing config, key or runtime files.
Evidence: src/app.rs:120-188, tests/quick_run.rs:509-570, tests/quick_run.rs:723-784

#### Scenario: Rejected input classes
- **WHEN** a required flag is missing, an address, port, name or node id is invalid, an unknown option is used, or the protocol is not tcp/udp
- **THEN** validation fails without side effects and identifies the invalid input

#### Scenario: Bad protocol
- **WHEN** `serve -p sctp` or `access -p sctp` is run
- **THEN** it exits with code 2 and stderr contains "tcp or udp"

#### Scenario: Bad address or missing value
- **WHEN** the address is `not-an-address`, `127.0.0.1:65536`, `127.0.0.1:0`, or a required flag is omitted
- **THEN** it exits with code 2 and the diagnostic names the option or "port"; no listener is left on the address

#### Scenario: Insecure relay URL
- **WHEN** `--relay-url http://localhost:1` is given
- **THEN** it is rejected with a message mentioning `https://`

### Requirement: Quick flags cannot be mixed with role commands
The system SHALL reject quick flags (`--name`, `--protocol`, `--relay-url`, `--target`, `--listen`, `--node-id`, and their short forms) combined with a role command, without side effects.
Evidence: src/cli.rs:41,49,77,111, tests/quick_run.rs:572-606

#### Scenario: Mixed invocation
- **WHEN** `serve --name echo config path` or `access --listen ... run --config file` is run
- **THEN** it exits with code 2, stderr contains "cannot be used with", and user files and state directory are unchanged

### Requirement: Role commands without quick flags use the default TCP protocol
The system SHALL accept role commands without any quick option, so the implicit `tcp` protocol default does not conflict with them.
Evidence: src/cli.rs:41-55,153, tests/quick_run.rs:786-808

#### Scenario: Config path command
- **WHEN** `serve config path` or `access config path` is run with a test home
- **THEN** it succeeds and prints `<home>/iroh-tunnel/<role>.toml`

### Requirement: Quick mode key file
The system SHALL read an optional persistent secret key from `-k/--key-file` (base64 key as produced by `config keygen`), so the public identity stays the same across restarts in every role form, and SHALL use an ephemeral identity when it is not given.
Evidence: src/cli.rs:158-161,182-185, src/app.rs:470-496, tests/role_binaries.rs:356-440

#### Scenario: Stable identity
- **WHEN** a role is started twice with the same key file
- **THEN** it reports the same node id both times

### Requirement: Unusable key files fail without leaking contents
The system SHALL fail with a config error mentioning `--key-file` when the key file cannot be read, is empty, or is not a valid base64 32-byte secret key, SHALL NOT print the file's contents, and SHALL NOT write any config.
Evidence: src/app.rs:476-496, tests/role_binaries.rs:656-710

#### Scenario: Missing or invalid key
- **WHEN** `-k` names a missing file or a file with a too-short key
- **THEN** the command fails, stderr contains `--key-file` but not the file's text, and the home directory stays empty

### Requirement: Quick mode leaves user files untouched
The system SHALL NOT create, modify or delete config or key files when running quick mode; only role runtime state (ownership/status) may appear.
Evidence: src/app.rs:120-175, tests/quick_run.rs:386-455

#### Scenario: Empty home
- **WHEN** quick roles are started and stopped repeatedly in an empty home
- **THEN** no config or key file exists afterwards

#### Scenario: Existing files
- **WHEN** quick roles restart with ephemeral identity while config and key files exist
- **THEN** their contents are unchanged

### Requirement: Run command requires an existing config
The system SHALL resolve the `run` config path from `--config` or the platform default and fail when the file does not exist, before starting the role.
Evidence: src/app.rs:197-206,230-234, src/cli.rs:191-196
Confidence: inferred

#### Scenario: Missing config
- **WHEN** `serve run` is invoked and the config file is absent
- **THEN** the command fails with an error from the existence check and does not start the role

### Requirement: Config actions dispatch
The system SHALL expose `config keygen` (with `--config`, `--yes`; `keygen random` prints a key without saving and conflicts with those options), `add`, `remove --name`, `list`, `show [--reveal]`, `edit` and `path`, each acting on the role's config file or the one given by `--config`.
Evidence: src/cli.rs:197-278, src/app.rs:253-271, tests/quick_run.rs:786-808

#### Scenario: Config path
- **WHEN** `config path` is run
- **THEN** the role's config path is printed

#### Scenario: Keygen random conflicts
- **WHEN** `config keygen random` is combined with `--config` or `--yes`
- **THEN** parsing rejects the combination

### Requirement: Status command
The system SHALL provide a `status` command per role that prints the role's status table, or the raw file with `--json`.
Evidence: src/cli.rs:202-207, src/app.rs:208, src/status_cmd.rs:414-484 (unit tests)
Confidence: inferred

#### Scenario: Status JSON
- **WHEN** `status --json` is run
- **THEN** the status file is printed verbatim

### Requirement: Service scope selection and privilege
The system SHALL offer `service install|uninstall|start|stop|restart|status` with mutually exclusive `--user` and `--system` flags; without a flag `install` asks a non-elevated interactive user and defaults to system for an elevated one, other verbs default by privilege; system scope without privilege is refused with rerun guidance and never elevated or replaced by a user service.
Evidence: src/cli.rs:280-333, src/app.rs:273-349, src/app.rs:354-378
Confidence: inferred

#### Scenario: Conflicting scope flags
- **WHEN** both `--user` and `--system` are given
- **THEN** parsing fails

#### Scenario: Unprivileged system install
- **WHEN** `service install --system` runs without elevation on Unix
- **THEN** it is refused with a rerun command using an absolute `--config` path

### Requirement: Service install pins absolute paths and validates config
The system SHALL pin an absolute config path and absolute state directory (`IROH_TUNNEL_STATE_DIR` or the platform default) into an installed user service and SHALL refuse to install when the config is missing or invalid for the role, with a config error suggesting `config keygen` or `--config`.
Evidence: src/app.rs:236-251,310-328,427-468
Confidence: inferred

#### Scenario: Invalid config
- **WHEN** `service install --user` is run with a missing or invalid config
- **THEN** it fails with "cannot install the service" and registers nothing

### Requirement: Service start refuses a running role
The system SHALL refuse `service start` when the role is already running, before asking the service manager to start it.
Evidence: src/app.rs:333-344
Confidence: inferred

#### Scenario: Role already running
- **WHEN** `service start` is invoked while the role holds ownership
- **THEN** it fails (Conflict, exit 6) and the service is not started

### Requirement: Standalone binaries refuse service management
The system SHALL refuse `service` subcommands on the standalone binaries with a service error (exit 5) containing "not yet supported", pointing to `iroh-tunnel <role> service ...`, and SHALL write nothing.
Evidence: src/app.rs:219-225, tests/role_binaries.rs:713-728

#### Scenario: Standalone install
- **WHEN** `its service install` or `ita service install` is run
- **THEN** exit code is 5, stderr contains "not yet supported", and the home directory is empty

### Requirement: Update acts only on the invoked executable
The system SHALL provide `update` (options `--to VERSION`, `-i/--insecure`, `-y/--yes`) at top level of `iroh-tunnel` and as a role command of standalone binaries, replacing only the running executable, and SHALL reject `<role> update` on the all-in-one binary with a config error telling the user to run `iroh-tunnel update`.
Evidence: src/cli.rs:56-73,213-214, src/app.rs:111-113,209-218
Confidence: inferred

#### Scenario: All-in-one role update
- **WHEN** `iroh-tunnel serve update` is run
- **THEN** it exits 2 with "update acts on the invoked executable"

#### Scenario: Standalone update
- **WHEN** `its update` is run
- **THEN** the update runs for executable name `iroh-tunnel-serve`

### Requirement: Logging verbosity precedence
The system SHALL choose the log filter with `RUST_LOG` first (overrides everything), then `--quiet` (errors only), then `-v` count: none or `-v` = info, `-vv` = debug, `-vvv` or more = trace; the default is info.
Evidence: src/cli.rs:362-384; docs/monitoring.md (Log levels)
Confidence: inferred

#### Scenario: Quiet
- **WHEN** `-q` is given without `RUST_LOG`
- **THEN** only error-level logs are emitted

#### Scenario: RUST_LOG overrides
- **WHEN** `RUST_LOG=iroh_tunnel=debug` is set with `-q`
- **THEN** the `RUST_LOG` filter is used

### Requirement: Exit status mapping
The system SHALL exit 0 on success and, on failure, print `error: <message>` to stderr and exit 1 for unclassified errors, 2 for config errors, 3 permission, 4 Iroh endpoint, 5 service and 6 conflict (role or identity already running).
Evidence: src/error.rs:1-30, src/app.rs:87-101, tests/quick_run.rs:509-570, tests/role_binaries.rs:713-728

#### Scenario: Config error
- **WHEN** a quick-mode validation fails
- **THEN** exit code is 2

#### Scenario: Service error
- **WHEN** a standalone binary receives `service install`
- **THEN** exit code is 5

### Requirement: Forced shutdown exit
The system SHALL, when a forced exit (repeated stop after grace, or watchdog) occurs while the role is running, drop the running work, print "shutdown abandoned", remove its own status file (when a role) and exit with the forced exit's code (130 for repeated stop, 1 for watchdog).
Evidence: src/app.rs:60-86 (exit codes per comment in src/app.rs:78)
Confidence: inferred

#### Scenario: Repeated stop
- **WHEN** a stop signal is repeated after the grace period
- **THEN** the process exits with status 130 after removing its own status file

### Requirement: Operator guidance on target protection
The system SHALL state in README and Access `--help` that the NodeId is public identity, not target authorization, and that target services must enforce their own access protection.
Evidence: src/cli.rs:173, tests/quick_run.rs:824-838, README.md

#### Scenario: Access help
- **WHEN** `access --help` is run
- **THEN** it exits successfully and contains "public identity, not authorization"
