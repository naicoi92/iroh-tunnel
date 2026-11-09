# github-actions-integration Specification

## Purpose

Define the iroh-tunnel GitHub Actions published from this repository: the Access action (`naicoi92/iroh-tunnel`, also `naicoi92/iroh-tunnel/access`) and the Serve action (`naicoi92/iroh-tunnel/serve`), which start a job-scoped tunnel from a verified release binary with a fresh identity per step, return when it is usable, and stop it with diagnostics at job end.

## Requirements

### Requirement: Action paths
The system SHALL provide an Access action reachable as `naicoi92/iroh-tunnel@<ref>` and as `naicoi92/iroh-tunnel/access@<ref>`, and a Serve action reachable as `naicoi92/iroh-tunnel/serve@<ref>`. The two Access paths MUST accept the same inputs, set the same outputs and behave identically.
Evidence: design.md D2
Confidence: inferred

#### Scenario: Access through the root path
- **WHEN** a workflow step uses `naicoi92/iroh-tunnel@v0` with `name`, `node-id` and `listen`
- **THEN** it starts an Access tunnel for that service

#### Scenario: Access through the alias path
- **WHEN** the same step is written with `naicoi92/iroh-tunnel/access@v0` and the same inputs
- **THEN** it sets the same outputs and starts the same kind of tunnel as the root path

#### Scenario: Serve path
- **WHEN** a workflow step uses `naicoi92/iroh-tunnel/serve@v0` with `name` and `target`
- **THEN** it starts a Serve tunnel for that service

### Requirement: Action runtime and Marketplace metadata
Each action SHALL be a JavaScript action on the `node24` runtime with a main step and a post step that runs whatever the job result is. The root metadata SHALL carry the name `iroh-tunnel`, a description and `branding`, so the repository can be listed on the GitHub Marketplace.
Evidence: design.md D2
Confidence: inferred

#### Scenario: Root metadata
- **WHEN** the root `action.yml` is read
- **THEN** it has `name: iroh-tunnel`, a description, `branding` with an icon and a color, and `runs.using: node24` with `main`, `post` and `post-if: always()`

#### Scenario: Serve metadata
- **WHEN** `serve/action.yml` is read
- **THEN** it has `runs.using: node24` with `main`, `post` and `post-if: always()`

#### Scenario: Root metadata mirrors the access directory
- **WHEN** the root `action.yml` and `access/action.yml` are compared
- **THEN** they are identical except for `runs.main` and `runs.post`, which both resolve to the same files under `access/dist/`

### Requirement: Access inputs and outputs
The Access action SHALL require the inputs `name`, `node-id` and `listen`, and SHALL accept the optional inputs `protocol`, `relay-url`, `relay-token` and `version`. It SHALL NOT accept a secret key or an input of the Serve role; besides these it MAY declare only the internal input `job-status`, which defaults to the job status and is not documented for callers. It SHALL set the output `node-id` to its own NodeId.
Evidence: design.md D4
Confidence: inferred

#### Scenario: Minimal Access step
- **WHEN** an Access step sets only `name: db`, `node-id: <serve id>` and `listen: 127.0.0.1:15432`
- **THEN** a TCP listener on `127.0.0.1:15432` forwards each connection to service `db` on that Serve node through the n0 public relays, and the `node-id` output is the Access node's NodeId

#### Scenario: Missing required Access input
- **WHEN** an Access step omits `node-id`
- **THEN** the step fails before downloading anything, with a message naming `node-id`

#### Scenario: Input of the other role
- **WHEN** an Access step sets `target`
- **THEN** `target` is not declared in the Access metadata, and the step behaves exactly as if it were absent

#### Scenario: Several Access services
- **WHEN** a Linux job has two Access steps for services `db` and `api`
- **THEN** both listeners work for the rest of the job

### Requirement: Serve inputs and outputs
The Serve action SHALL require the inputs `name` and `target`, where `target` is `host:port` with an IP address or a hostname, and SHALL accept the optional inputs `protocol`, `relay-url`, `relay-token` and `version`. It SHALL NOT accept a secret key or an input of the Access role; besides these it MAY declare only the internal input `job-status`, as for Access. It SHALL set the output `node-id`, the NodeId clients dial.
Evidence: design.md D3, D4
Confidence: inferred

#### Scenario: Serve in front of a runner-mapped job service
- **WHEN** a job service maps PostgreSQL to `5432` on the runner and a Serve step sets `name: db` and `target: 127.0.0.1:5432`
- **THEN** a client dialing the `node-id` output with ALPN `iroh-tunnel/db` reaches PostgreSQL

#### Scenario: Serve in a container job with a hostname target
- **WHEN** the job runs in `container:` with a job service named `postgres` and a Serve step sets `target: postgres:5432`
- **THEN** the target is resolved by name and tunnel streams reach that service

#### Scenario: Missing required Serve input
- **WHEN** a Serve step omits `target`
- **THEN** the step fails before downloading anything, with a message naming `target`

### Requirement: Shared optional inputs
For both actions, `protocol` SHALL default to `tcp` and accept `udp`; `relay-url` SHALL take one URL per line; `version` SHALL select the iroh-tunnel release to install.
Evidence: design.md D4, D6
Confidence: inferred

#### Scenario: UDP service
- **WHEN** a Serve step sets `protocol: udp` and `target: 127.0.0.1:5353`
- **THEN** UDP datagrams from clients of that service reach the target

#### Scenario: Two relays
- **WHEN** `relay-url` has two lines
- **THEN** the tunnel uses both relays, the first as home relay

### Requirement: Inputs validated before side effects
Each action SHALL validate every input before downloading, writing files or starting a process, and SHALL fail with a message that names the offending input.
Evidence: design.md D1
Confidence: inferred

#### Scenario: Invalid service name
- **WHEN** `name` is `My_DB`
- **THEN** the step fails naming `name` and the allowed characters, and no binary is downloaded

#### Scenario: Valid input
- **WHEN** every input is valid
- **THEN** validation passes and provisioning starts

### Requirement: Input value rules
`name` MUST use lowercase letters, digits and `-`, 1 to 63 bytes. `protocol` MUST be `tcp` or `udp`. `node-id` MUST be 64 hex or 52 base32 characters. Each `relay-url` line MUST be an `https` URL. `relay-token`, after trimming surrounding whitespace, MUST be a single non-empty line. `version` MUST be `X.Y.Z` or `vX.Y.Z`, optionally with a pre-release suffix, or `latest`.
Evidence: design.md D1, D6; crates/host/src/config/dto.rs:36,394-401,451-456,479-481,505-508
Confidence: inferred

#### Scenario: Malformed NodeId
- **WHEN** `node-id` has 10 characters
- **THEN** the step fails naming `node-id`

#### Scenario: Unknown protocol
- **WHEN** `protocol` is `sctp`
- **THEN** the step fails naming `protocol` and listing `tcp` and `udp`

#### Scenario: Relay URL not https
- **WHEN** a `relay-url` line is `relay.example.com` or `http://relay.example.com`
- **THEN** the step fails naming `relay-url` and stating that `https://` is required

#### Scenario: Relay token with a trailing newline
- **WHEN** `relay-token` is a secret value followed by a newline
- **THEN** the trimmed value is used

#### Scenario: Multi-line relay token
- **WHEN** `relay-token` has two non-empty lines
- **THEN** the step fails naming `relay-token` without printing its value

#### Scenario: Malformed version
- **WHEN** `version` is `0.6`
- **THEN** the step fails naming `version`

### Requirement: Address input rules
`listen` and `target` MUST be `host:port`, where host is an IPv4 address, a bracketed IPv6 address or, for `target` only, a hostname, and port is 1 to 65535.
Evidence: design.md D1, D3; crates/host/src/config/dto.rs:459-463
Confidence: inferred

#### Scenario: Port out of range
- **WHEN** `listen` is `127.0.0.1:70000` or `127.0.0.1:0`
- **THEN** the step fails naming `listen`

#### Scenario: Port boundary accepted
- **WHEN** `listen` is `127.0.0.1:65535`
- **THEN** validation accepts it

#### Scenario: IPv6 address
- **WHEN** `target` is `[::1]:5432`
- **THEN** validation accepts it, and `target` `::1:5432` without brackets fails naming `target`

#### Scenario: Hostname not allowed for listen
- **WHEN** `listen` is `localhost:15432`
- **THEN** the step fails naming `listen`

### Requirement: Ephemeral identity per step
Each step SHALL start its tunnel with a newly generated secret key, so two steps, in the same job or in concurrent jobs, MUST get different NodeIds. The key SHALL exist only in a file inside the step's own temporary directory under `RUNNER_TEMP`, readable only by the runner user on Unix.
Evidence: design.md D3; local run of two `serve run` processes without `secret_key` printed two different NodeIds
Confidence: inferred

#### Scenario: Concurrent jobs
- **WHEN** two jobs start a Serve step at the same time
- **THEN** their `node-id` outputs differ

#### Scenario: Key file permissions
- **WHEN** a Linux step has started its tunnel
- **THEN** the file holding the key has mode `0600`

### Requirement: Secret key never leaves the step
The generated key MUST NOT appear in any output, log line or summary written by the action, and the post step SHALL delete the step's temporary directory.
Evidence: design.md D3, D8
Confidence: inferred

#### Scenario: No key leaks
- **WHEN** a step completes, successfully or not
- **THEN** neither its outputs nor the job log contain the generated secret key

#### Scenario: Key removed at job end
- **WHEN** the post step has run
- **THEN** the step's temporary directory, including the file with the key, no longer exists

### Requirement: Relay settings
Each action SHALL use the n0 public relays when `relay-url` is empty, and otherwise exactly the listed relays in order. When `relay-token` is set, the action SHALL register it as a secret with the runner before any use, so it is masked in logs, and SHALL pass it to the tunnel as the relay token.
Evidence: design.md D3, D9
Confidence: inferred

#### Scenario: Own relay with a token
- **WHEN** a Serve step sets `relay-url: https://relay.example.com` and `relay-token: ${{ secrets.T }}`
- **THEN** the rendered config lists that relay and the token, the tunnel uses them, and the token appears masked wherever it would be printed

#### Scenario: Default relays
- **WHEN** `relay-url` is not set
- **THEN** the tunnel uses the n0 public relays

### Requirement: Release version selection
Each action SHALL install the release named by `version`, defaulting to the release recorded in the action at build time, so `@vX.Y.Z` installs `X.Y.Z` and a moving major tag installs the release it points to. `latest` SHALL select the latest stable (non-pre-release) release. A version lower than the minimum version recorded in the action (the first release that ships the actions) SHALL be rejected.
Evidence: design.md D6
Confidence: inferred

#### Scenario: Default version
- **WHEN** a step uses `naicoi92/iroh-tunnel@v0.6.0` without `version`
- **THEN** iroh-tunnel 0.6.0 is installed

#### Scenario: Release not found
- **WHEN** `version` names a release that does not exist
- **THEN** the step fails naming the version and the URL it tried

#### Scenario: Version too old
- **WHEN** `version` is lower than the recorded minimum version
- **THEN** the step fails naming the minimum supported version

#### Scenario: Latest
- **WHEN** `version` is `latest` and the newest release is a pre-release
- **THEN** the newest stable release is installed

### Requirement: Verified binary download
Each action SHALL download its role archive (`iroh-tunnel-access` or `iroh-tunnel-serve`) and the matching checksum file, verify the SHA-256 digest before extracting, and reuse a cached copy of the same version and role on the same runner.
Evidence: design.md D6; install-release-packaging spec "Release build matrix and archives"
Confidence: inferred

#### Scenario: Checksum mismatch
- **WHEN** the downloaded archive's digest differs from the checksum file
- **THEN** the step fails naming the archive, and nothing is extracted or started

#### Scenario: Cached binary
- **WHEN** a second step on the same runner needs the same version and role
- **THEN** it uses the cached executable without downloading again

### Requirement: Supported platforms
Each action SHALL support Linux amd64 and arm64, using the musl build when the step runs on musl libc and the glibc build otherwise, macOS arm64, and Windows amd64 and arm64, and SHALL fail before downloading on any other platform.
Evidence: design.md D6
Confidence: inferred

#### Scenario: Unsupported runner
- **WHEN** the step runs on macOS x86_64 or Linux riscv64
- **THEN** it fails before downloading, listing the supported platforms

#### Scenario: Alpine container job
- **WHEN** the job runs in an Alpine `container:` on amd64
- **THEN** the `linux-amd64-musl` archive is installed

#### Scenario: Ubuntu runner
- **WHEN** the step runs directly on `ubuntu-latest`
- **THEN** the `linux-amd64` (glibc) archive is installed

### Requirement: Tunnel runs for the rest of the job
Each action SHALL start its tunnel in the background and keep it running after the main step completes, until the job's post steps. Outputs SHALL be set only when the main step succeeds.
Evidence: design.md D8
Confidence: inferred

#### Scenario: Later steps use the tunnel
- **WHEN** a Serve step succeeds and three later steps run tests
- **THEN** the tunnel serves clients during all three steps

#### Scenario: No outputs on failure
- **WHEN** the main step fails
- **THEN** `node-id` is not set

### Requirement: Step returns only when the tunnel is usable
Each action SHALL complete its main step only when the tunnel is usable, within 60 seconds. Serve is usable when its status file reports its NodeId and its home relay. Access over TCP is usable when its listener is bound and its connection to the Serve peer is established, which the action triggers by opening and closing one local connection to `listen` (to the loopback address of the same family when `listen` is `0.0.0.0` or `[::]`).
Evidence: design.md D7; issue #183
Confidence: inferred

#### Scenario: Access ready
- **WHEN** an Access TCP step completes successfully
- **THEN** a connection opened to `listen` by the next step reaches the remote service without waiting for a dial

#### Scenario: Serve ready
- **WHEN** a Serve step completes successfully
- **THEN** its `node-id` output is set and its node is registered with its home relay, so a client using the same relays can dial it through that relay without a retry window for relay registration

### Requirement: Serve readiness waits for the home relay
The Serve action SHALL treat its tunnel as usable only when the status file reports both `node_id` and a non-empty `home_relay`. The docs SHALL say that Serve waits for its home relay, and SHALL NOT tell callers to retry early dials for relay registration.
Evidence: issue #183; #187 (`home_relay` reported within one status flush, about 5 s); `action-src/build.ts` (minimum version 0.6.0)
Confidence: inferred

#### Scenario: Home relay not yet reported
- **WHEN** the Serve status file has `node_id` and `home_relay: null`
- **THEN** the step keeps waiting and does not set `node-id`

#### Scenario: Home relay reported
- **WHEN** the Serve status file has `node_id` and `home_relay: https://relay.example.com./`
- **THEN** the step completes and sets `node-id`

#### Scenario: Docs describe Serve readiness
- **WHEN** a caller reads the Behavior section of the docs page
- **THEN** it says Serve is ready once its status file reports a NodeId and a home relay, and does not advise retrying dials for relay registration

### Requirement: UDP Access readiness is listener-only
An Access step with `protocol: udp` SHALL be usable once its listener is bound, and the action SHALL log that the peer connection is not checked.
Evidence: design.md D7
Confidence: inferred

#### Scenario: UDP Access step
- **WHEN** an Access step with `protocol: udp` has bound its listener
- **THEN** the step completes and its log says the peer connection was not checked

### Requirement: Start failures are reported with the tunnel log
When the tunnel is not usable within 60 seconds, exits before it is usable, or logs that its listener failed to bind, the main step SHALL fail with the reason, the process exit code and error class when it exited, the last tunnel status, and the end of the tunnel log. When a Serve step times out after its status file reported `node_id` but no home relay, the reason SHALL state that no home relay was registered and name the `relay-url` and `relay-token` inputs.
Evidence: design.md D7; docs/monitoring.md (exit codes); issue #183
Confidence: inferred

#### Scenario: Readiness timeout
- **WHEN** an Access step cannot reach the Serve peer within 60 seconds
- **THEN** the step fails with a message saying it was not ready in time, the last tunnel status and the end of the tunnel log

#### Scenario: Serve times out without a home relay
- **WHEN** a Serve step's status file reports `node_id` but `home_relay` stays `null` for 60 seconds, for example because `relay-url` points at an unreachable relay
- **THEN** the step fails with a message saying it was not ready in time and no home relay was registered, naming `relay-url` and `relay-token`, with the last tunnel status and the end of the tunnel log, and `node-id` is not set

#### Scenario: Tunnel exits during start
- **WHEN** the tunnel process exits before it is usable because its config is rejected
- **THEN** the step fails at once with the exit code, its error class and the end of the tunnel log

#### Scenario: Listen port taken
- **WHEN** the Access listener cannot bind because `listen` is already in use (the process keeps running and logs `failed to bind`)
- **THEN** the step fails at once naming `listen` and quoting the bind error, without waiting for the 60-second limit, and the post step stops the process

### Requirement: Job-end cleanup
Each action's post step SHALL run whatever the job result is, stop the tunnel and fail if the tunnel process is still running afterwards, so no tunnel process started by the action outlives the job. On Unix it SHALL send a termination signal and force a kill after 5 seconds; on Windows it SHALL terminate the process.
Evidence: design.md D8
Confidence: inferred

#### Scenario: Normal stop
- **WHEN** the post step runs after a successful job
- **THEN** the tunnel process exits and the post step succeeds

#### Scenario: Main step failed before starting
- **WHEN** the main step failed during input validation
- **THEN** the post step finds no tunnel and completes without error

### Requirement: Job-end diagnostics
Each action's post step SHALL print the tunnel's final status in a collapsed log group, and SHALL print the tunnel log only when the job failed.
Evidence: design.md D8
Confidence: inferred

#### Scenario: Successful job
- **WHEN** the job's steps all succeed
- **THEN** the post step prints the status and does not print the tunnel log

#### Scenario: Failed job
- **WHEN** a later step of the job fails
- **THEN** the post step prints the status and the tunnel log

### Requirement: Per-step isolation
Each step SHALL run its tunnel with its own state directory inside the step's temporary directory, and on Linux and macOS its own home directory, so steps of the same role in one job and concurrent jobs of one runner user do not refuse each other.
Evidence: design.md D5; role-ownership-shutdown spec "Ownership scope is the user's default state directory"
Confidence: inferred

#### Scenario: Two Serve steps on Linux
- **WHEN** a Linux job has Serve steps for `db` and `cache`
- **THEN** both start and get different NodeIds

#### Scenario: Concurrent jobs on one self-hosted runner user
- **WHEN** two macOS jobs run Access steps at the same time as the same user
- **THEN** both start

### Requirement: Windows limit of one step per role
On Windows, where the role lock cannot be moved, a job SHALL support at most one Access step and one Serve step; a further step of the same role in the same job SHALL fail with a message that names this limit.
Evidence: design.md D5; crates/host/src/paths/default_state_dir.rs:19-20; crates/host/src/ownership.rs:106-109
Confidence: inferred

#### Scenario: Second Serve step on Windows
- **WHEN** a Windows job runs a second Serve step while the first is running
- **THEN** the second step fails with a message stating one Serve and one Access step per job on Windows

#### Scenario: Access and Serve together on Windows
- **WHEN** a Windows job has one Serve step and one Access step
- **THEN** both start

### Requirement: Role already running is explained
When the tunnel exits with the conflict status (exit code 6) during start, the step SHALL fail with a message saying the role is already running for this runner user, naming a running system service or another step as possible causes, and on Windows the one-step-per-role limit.
Evidence: design.md D5, D7; docs/monitoring.md (exit codes); role-ownership-shutdown spec "Machine-tier lock between system service and user processes"
Confidence: inferred

#### Scenario: System service on a self-hosted Linux runner
- **WHEN** a Serve step runs on a self-hosted Linux runner where an iroh-tunnel Serve system service is running
- **THEN** the step fails with the already-running message instead of only the exit code

### Requirement: Consumer documentation
The system SHALL document the actions for other projects: a README section with one copy-paste example per role, and a docs page with examples, the input and output tables, the fixed behaviour (including the one connection Access readiness opens and closes to the remote service), version pinning, the supported platforms and the Windows limit.
Evidence: design.md D11
Confidence: inferred

#### Scenario: Copy-paste adoption
- **WHEN** a user copies the README Access example and replaces the NodeId and port
- **THEN** the workflow runs without any other input

#### Scenario: Docs page examples
- **WHEN** the docs page is read
- **THEN** it has examples for Access to a private service, Serve in front of a job service, Serve in a `container:` job, two services as two steps, an own relay with a token, and Serve and Access in one job

### Requirement: Embedded client contract documented
The docs page SHALL state how an application with an embedded iroh client reaches a service published by the Serve action: dial the `node-id` output with ALPN `iroh-tunnel/<name>`, open one bidirectional stream per TCP connection carrying the raw bytes, and use the same relays as the Serve step.
Evidence: design.md D11; crates/engine/src/proto.rs; crates/engine/src/tcp.rs
Confidence: inferred

#### Scenario: Embedded client contract
- **WHEN** a developer reads the docs page to connect an application with an embedded iroh client
- **THEN** it states the ALPN, the stream-per-connection rule, the raw byte framing and the relay requirement
