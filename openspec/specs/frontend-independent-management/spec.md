# frontend-independent-management Specification

## Purpose

TBD

## Requirements

### Requirement: Management operations are a non-terminal Rust boundary
`iroh-tunnel-host` SHALL expose documented Rust operations for the existing configuration, OS service lifecycle and role-status responsibilities. They MUST accept plain request values, return data, decision-needed outcomes or errors, and MUST NOT read stdin, write to the caller's stdout/stderr or call `std::process::exit`.
Source: proposal.md What Changes; design.md Decisions 3, 4, 8

#### Scenario: Caller without a terminal lists services
- **WHEN** a Rust caller with stdin closed and stdout/stderr redirected invokes the host list-services operation for an existing config
- **THEN** it receives the service entries as data and nothing is read from stdin or written to stdout/stderr

#### Scenario: No clap types in requests
- **WHEN** the public request types of the host configuration, service and update operations are inspected
- **THEN** none is or contains a clap `Args`, `Parser` or `Subcommand` type

### Requirement: Management boundary is not a remote protocol
The management boundary SHALL be an in-process Rust-caller contract only. It MUST NOT introduce a daemon, control socket, remote API or IPC serialization commitment.
Source: design.md Non-Goals, Decision 8

#### Scenario: No listener is opened by management calls
- **WHEN** a caller performs any host configuration, service or status operation
- **THEN** no network listener, control socket or background management process is created

### Requirement: Configuration operations return outcomes
The host SHALL provide configuration operations for path resolution, set (create or update a service entry), remove, list, show, check, keygen, random key generation and edit. Each operation MUST return the affected service, path or data instead of a printed message. Set MUST return one of the following:
- `Created` with the service name and path;
- `Updated` with the path and the list of changed fields, each with its old and new value;
- `Unchanged` with the path.
Source: design.md D3, D7; openspec/specs/config-management-commands

#### Scenario: Add returns the affected entry
- **WHEN** a caller sets a valid Serve service entry whose name is not yet in an existing config
- **THEN** the operation returns `Created` with the service name and config path

#### Scenario: Random key touches no file
- **WHEN** a caller requests a random key
- **THEN** the secret key is returned explicitly and no config file is created or modified

#### Scenario: Update returns the changes
- **WHEN** a caller sets only a new port on an existing entry
- **THEN** the operation returns `Updated` with exactly one change, for the address, holding the old and new values

#### Scenario: Unchanged writes nothing
- **WHEN** a caller sets values equal to the existing entry's values
- **THEN** the operation returns `Unchanged` and the config file is not rewritten

#### Scenario: Check returns the validated path
- **WHEN** a caller checks a valid existing config
- **THEN** the operation returns the resolved path without taking the writer lock or writing

### Requirement: Configuration mutations keep store guarantees
Host configuration mutations MUST apply the existing writer lock, validation, all-or-nothing semantics, owner-only permissions, symlink write-through, atomic rename/fsync ordering and formatting preservation. When set would create an entry without the attributes required for creation, the host MUST return a typed missing-for-create error naming the missing attributes, decided under the writer lock, and MUST NOT write.
Source: design.md D2, D3; openspec/specs/config-management-commands

#### Scenario: Invalid mutation changes nothing
- **WHEN** a caller sets a new Access service without a node ID
- **THEN** the operation returns a missing-for-create error naming the node ID, and the config file bytes are unchanged

#### Scenario: Concurrent mutations serialize
- **WHEN** two callers set different new service entries on the same config concurrently
- **THEN** both entries are present afterwards and neither write is lost

### Requirement: Show redacts secrets unless reveal is explicit
The host show operation SHALL redact secret values unless the request carries an explicit reveal decision, matching CLI `config show` behaviour.
Source: openspec/specs/config-management-commands (Show redacts secrets by default)

#### Scenario: Default show
- **WHEN** a caller requests show without a reveal decision
- **THEN** the returned document has the `secret_key` value redacted

### Requirement: Key rotation requires explicit consent
When keygen would replace an existing identity, the host operation SHALL return a decision-needed outcome with the current public NodeId unless the request carries rotation consent. Consent MUST be rechecked under the writer lock; first-time generation SHALL NOT require consent.
Source: design.md Decision 3; openspec/specs/config-management-commands (Key rotation needs confirmation)

#### Scenario: Rotation without consent
- **WHEN** a caller requests keygen for a config that already has a `secret_key` and supplies no consent
- **THEN** the operation returns a rotation-consent-needed outcome with the current public NodeId and the file is unchanged

#### Scenario: Rotation with consent
- **WHEN** the caller repeats the request with rotation consent
- **THEN** a new key is persisted and the new public NodeId is returned

#### Scenario: First generation needs no consent
- **WHEN** a caller requests keygen for a config whose `secret_key` is empty
- **THEN** a key is generated and persisted without a consent outcome

#### Scenario: CLI confirmation behaviour unchanged
- **WHEN** `iroh-tunnel keygen` would rotate an existing key on a non-TTY without `--yes`
- **THEN** it refuses exactly as before the refactor

### Requirement: Edit is prepared and committed without holding the lock
The host edit operation SHALL prepare an owner-only sibling copy, record the original bytes or absence, and hold no writer lock while the caller's editor runs. Commit MUST compare the current file with the original under the lock, validate, then atomically commit or keep the existing `.rejected` sidecar.
Source: design.md Decision 3; openspec/specs/config-management-commands

#### Scenario: Concurrent change rejects the edit
- **WHEN** the live config is modified after prepare and before commit
- **THEN** commit returns a conflict outcome, the live file keeps the concurrent change, and the edited content is preserved in the rejection sidecar

#### Scenario: Invalid edit is rejected
- **WHEN** the edited copy fails validation at commit
- **THEN** the live file is unchanged and the returned error names the sidecar path

### Requirement: Abandoned edits leave no trace
An edit attempt that fails or is dropped without commit MUST remove its scratch copy, MUST NOT overwrite the live file and MUST NOT report a save.
Source: design.md Decision 3

#### Scenario: Dropped attempt
- **WHEN** a prepared edit attempt is dropped without commit
- **THEN** its scratch copy is removed and the live file is unchanged

### Requirement: Service operations enforce existing policy
The host SHALL provide install, uninstall, start, stop, restart and status operations taking a typed role, scope and resolved paths. They MUST use the same backend selection and privilege, account, pin/trust, installability and ownership policies as the CLI; raw backend dispatch SHALL NOT be public.
Source: design.md Decision 4; openspec/specs/system-service-management

#### Scenario: Unprivileged system scope
- **WHEN** an unprivileged caller requests install with system scope
- **THEN** the operation returns a permission-classified error whose guidance names the `sudo` rerun command and absolute config path, and no registration is written

#### Scenario: Non-elevated Windows system scope
- **WHEN** a non-elevated caller requests install with system scope on Windows
- **THEN** the operation returns a permission-classified error whose guidance names the elevated-terminal rerun and absolute config path, and no service is registered

#### Scenario: BusyBox user scope unchanged
- **WHEN** a caller requests a user-scope operation on a host where the BusyBox/SysV backend is selected
- **THEN** the operation behaves as the existing BusyBox policy specifies and does not invent per-user support

### Requirement: Insufficient information yields a structured need
When a service operation lacks information, it SHALL return a structured need (scope selection, invoking-user input or retarget confirmation) instead of prompting or guessing. Execution MUST recheck privilege, ownership, trust and registration facts before mutating.
Source: design.md Decision 4

#### Scenario: Install without scope from an unprivileged caller
- **WHEN** an unprivileged caller requests install with no scope decision
- **THEN** the operation returns a scope-selection need and registers nothing

#### Scenario: CLI keeps its scope prompt
- **WHEN** a non-elevated user runs `iroh-tunnel serve service install` on a terminal with no scope flag
- **THEN** the existing `user`/`system` prompt appears on stderr as before

### Requirement: Retarget confirmation is specific and fresh
A retarget confirmation SHALL identify both the old and the proposed pin. A confirmation whose old pin no longer matches the registration MUST NOT be applied.
Source: design.md Decision 4; openspec/specs/system-service-management (Retargeting another user's service needs confirmation)

#### Scenario: Retarget without confirmation
- **WHEN** a privileged install would retarget a service pinned to another user and the request lacks a matching confirmation
- **THEN** the operation returns a retarget-confirmation need naming both pins and changes nothing

#### Scenario: Stale confirmation
- **WHEN** the registered pin changed after the confirmation was produced
- **THEN** the operation refuses or returns a new retarget-confirmation need

### Requirement: Registered and updated executables are supplied explicitly
Host service install SHALL take the absolute path of the all-in-one tunnel executable to register, and the host update operation SHALL take its target executable. Neither MUST fall back to the calling process's `current_exe()`.
Source: design.md Decisions 4, 7

#### Scenario: Management application registers the tunnel binary
- **WHEN** a non-CLI caller requests install and supplies the path of the `iroh-tunnel` executable
- **THEN** the registration's command line references that path and not the caller's executable

#### Scenario: CLI resolution unchanged
- **WHEN** `iroh-tunnel serve service install` runs with `iroh-tunnel` on `PATH`
- **THEN** the registration references the `PATH` binary exactly as before the refactor

### Requirement: Service lifetime is independent of the caller
A registered service's process SHALL be owned by the OS service manager. Dropping any host request, result or value, or the caller process exiting, MUST NOT stop, uninstall or signal the service, and service operations SHALL NOT return a lifecycle lease or kill-on-drop handle.
Source: brainstorm.md Key Decisions 1; design.md Decision 4

#### Scenario: Caller exits after start
- **WHEN** a management caller starts a user-scope Serve service and then exits
- **THEN** the service is still running and its status file is still published

### Requirement: Explicit lifecycle operations keep composite semantics
Only explicit stop, restart or uninstall SHALL change service lifetime. Install SHALL still enable and start; uninstall SHALL still stop, disable and remove; start SHALL keep its ownership refusal before the manager call.
Source: design.md Decision 4; openspec/specs/system-service-management

#### Scenario: Start refused when the role already runs
- **WHEN** a caller requests start while the Serve role is already running for that user
- **THEN** the operation returns a conflict-classified refusal before calling the service manager

#### Scenario: Partial install reports applied effects
- **WHEN** a registration write succeeds but enabling or starting fails
- **THEN** the operation returns the failure with the effects already applied and performs no compensating rollback

### Requirement: Service status separates facts without inventing truth
Host service status SHALL report registration/backend facts, kernel-backed role liveness, owner metadata and captured diagnostics as separate fields, with unavailable facts explicit. It MUST NOT derive liveness from manager state or parse localized `systemctl`, `launchctl` or `schtasks` prose.
Source: design.md Decision 4

#### Scenario: Foreground role without a registration
- **WHEN** a Serve role runs in the foreground and no service is registered
- **THEN** status reports the registration as absent and the role as live, without claiming the registration launched it

### Requirement: Optional status facts do not cause failure
A supplemental status fact that cannot be gathered SHALL be reported as unavailable, and MUST NOT fail a status call whose existing required checks succeed.
Source: design.md Decision 4

#### Scenario: Owner metadata unavailable
- **WHEN** the role is live but owner metadata cannot be read
- **THEN** status reports owner metadata as unavailable and succeeds

### Requirement: Native output is captured for non-terminal callers
For non-terminal callers, service operations SHALL capture native command stdout/stderr and exit status and return them without terminal output. Backend-specific non-zero status rules, such as a stopped systemd unit not being a failure, MUST be preserved.
Source: design.md Decision 4

#### Scenario: Captured diagnostics
- **WHEN** a non-terminal caller requests status for a registered systemd user service
- **THEN** the result contains the captured `systemctl` output and exit status, and nothing is written to the caller's terminal

### Requirement: CLI keeps native passthrough and ordering
The CLI SHALL request inherited native streams where it forwards native output today. Notes preceding a native command MUST reach a synchronous observer at that stage so ordering and partial-failure diagnostics are retained.
Source: design.md Decision 4

#### Scenario: Ordering on a terminal
- **WHEN** `iroh-tunnel serve service status` runs on a terminal
- **THEN** native backend output appears on the inherited streams in the same order relative to the CLI's notes as before the refactor

### Requirement: Role-status reads return classified data
The host role-status read SHALL return the validated raw body when present, a typed snapshot, probed liveness and owner data, and a freshness condition: live-without-snapshot, absent-and-not-running, unreadable or invalid file, stale pid, missing owner metadata or current snapshot.
Source: design.md Decision 7; openspec/specs/connection-path-status

#### Scenario: Role not running
- **WHEN** a caller reads Serve status with no status file and no live role
- **THEN** the result is absent-and-not-running

#### Scenario: Stale snapshot
- **WHEN** a status file exists but its recorded pid is not a live role
- **THEN** the result is stale-pid and includes the typed snapshot

### Requirement: Role-status reads have no side effects
Reading role status MUST NOT generate keys, create a config or role state, or start a service.
Source: design.md Decision 7

#### Scenario: Read in an empty state directory
- **WHEN** a caller reads Access status with no config and an empty state directory
- **THEN** no file or key is created

### Requirement: Raw status JSON stays byte-identical
CLI `status --json` SHALL forward the validated original status-file bytes with the existing line behaviour rather than re-serializing the typed value.
Source: design.md Decision 7; openspec/specs/connection-path-status (Status command output)

#### Scenario: Raw JSON passthrough
- **WHEN** `iroh-tunnel status --json` runs while a role publishes a status file
- **THEN** its output equals the validated file bytes followed by the existing line behaviour

### Requirement: Host operations define their execution model
Configuration and OS service operations SHALL be synchronous finite operations meant to run on a caller-owned blocking worker. The host MUST NOT create a nested runtime or assume a global executor, and SHALL NOT add service-command timeouts or retries.
Source: design.md Decision 5

#### Scenario: Called from a blocking worker
- **WHEN** a Tokio caller runs a host service operation inside `spawn_blocking`
- **THEN** it completes without creating a runtime or panicking about a nested runtime

### Requirement: Cancellation is not rollback
Cancellation before the first side effect SHALL leave configuration and registrations unchanged. Dropping the awaiter after a blocking write or manager command has begun MUST NOT be treated as rollback and SHALL NOT trigger a compensating stop.
Source: design.md Decision 5

#### Scenario: Cancelled before side effects
- **WHEN** a caller abandons an install request before any registration write or manager command starts
- **THEN** no registration exists and the config is unchanged

#### Scenario: Dropped awaiter during a manager command
- **WHEN** a caller drops the future awaiting a blocking-worker start after the manager command has started
- **THEN** the command runs to completion and no stop is issued

### Requirement: Updater is a host operation with explicit inputs
The host update operation SHALL accept target executable, version selection, overwrite and insecure decisions instead of `UpdateArgs`, and return an outcome without terminal output. It MUST preserve the existing verification bounds, delegation, version selection and atomic replacement.
Source: design.md Decision 7; openspec/specs/self-update

#### Scenario: Package-manager install is delegated
- **WHEN** the update operation targets an executable owned by a package manager
- **THEN** it returns the existing delegation outcome and does not replace the file

### Requirement: Update does not restart services
The host update operation SHALL NOT stop or restart running tunnels or registered services.
Source: design.md Decision 7

#### Scenario: Running service untouched
- **WHEN** an update replaces the executable used by a running registered service
- **THEN** the service is not stopped or restarted by the update operation

### Requirement: No elevation, account switch or scope fallback
Service operations MUST NOT self-elevate, switch accounts, fall back to another scope or substitute a different platform service mechanism, on any supported platform.
Source: design.md Decision 4 (refactor-codebase); windows-system-service design.md D8

#### Scenario: Windows system scope without elevation
- **WHEN** a non-elevated caller requests a system-scope operation on Windows
- **THEN** the operation returns a permission-classified error with elevated-terminal rerun guidance, and creates no Windows Service, scheduled task or user task

#### Scenario: Windows system scope uses only a Windows Service
- **WHEN** an elevated caller installs system scope on Windows
- **THEN** only a Windows Service is registered, and no scheduled task is created as a substitute
