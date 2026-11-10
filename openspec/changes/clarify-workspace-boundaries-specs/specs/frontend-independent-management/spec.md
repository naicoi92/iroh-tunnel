## MODIFIED Requirements

### Requirement: Key rotation requires explicit consent
When keygen would replace an existing identity, the host operation SHALL return a decision-needed outcome with the current public NodeId unless the request carries rotation consent. Consent MUST be rechecked under the writer lock; first-time generation SHALL NOT require consent.
Source: refactor-codebase design.md Decision 3; openspec/specs/config-management-commands (Key rotation needs confirmation)

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
- **WHEN** `iroh-tunnel serve config keygen` would rotate an existing key without a terminal and without `--yes`
- **THEN** it fails with an error mentioning `--yes` and the stored key is unchanged, as `config-management-commands` "Key rotation needs confirmation" specifies

### Requirement: Insufficient information yields a structured need
When a service operation lacks information, it SHALL return a structured need (scope selection, invoking-user input or retarget confirmation) instead of prompting or guessing. Execution MUST recheck privilege, ownership, trust and registration facts before mutating.
Source: refactor-codebase design.md Decision 4

#### Scenario: Install without scope from an unprivileged caller
- **WHEN** an unprivileged caller requests install with no scope decision
- **THEN** the operation returns a scope-selection need and registers nothing

#### Scenario: CLI keeps its scope prompt
- **WHEN** a non-elevated user runs `iroh-tunnel serve service install` on a terminal with no scope flag
- **THEN** the CLI turns the scope-selection need into the prompt offering `user` and `system` on stderr, as `system-service-management` "Install scope selection" specifies

### Requirement: Registered and updated executables are supplied explicitly
Host service install SHALL take the absolute path of the all-in-one tunnel executable to register, and the host update operation SHALL take its target executable. Neither MUST fall back to the calling process's `current_exe()`.
Source: refactor-codebase design.md Decisions 4, 7

#### Scenario: Management application registers the tunnel binary
- **WHEN** a non-CLI caller requests install and supplies the path of the `iroh-tunnel` executable
- **THEN** the registration's command line references that path and not the caller's executable

#### Scenario: CLI resolution unchanged
- **WHEN** `iroh-tunnel serve service install` runs with `iroh-tunnel` on `PATH`
- **THEN** the CLI supplies the `PATH` binary to the host operation and the registration references it, as `system-service-management` "Binary resolution for registrations" specifies

### Requirement: CLI keeps native passthrough and ordering
The CLI SHALL request inherited native streams for every service command whose native backend output it forwards to the terminal. Notes preceding a native command MUST reach a synchronous observer at that stage, so they are written before that command's output and partial-failure diagnostics are retained.
Source: refactor-codebase design.md Decision 4

#### Scenario: Ordering on a terminal
- **WHEN** `iroh-tunnel serve service status` runs on a terminal
- **THEN** native backend output appears on the inherited streams, and each CLI note is written before the output of the native command it precedes

### Requirement: Service operations enforce existing policy
The host SHALL provide install, uninstall, start, stop, restart and status operations taking a typed role, scope and resolved paths. They MUST use the same backend selection and privilege, account, pin/trust, installability and ownership policies as the CLI; raw backend dispatch SHALL NOT be public.
Source: refactor-codebase design.md Decision 4; openspec/specs/system-service-management

#### Scenario: Unprivileged system scope
- **WHEN** an unprivileged caller requests install with system scope
- **THEN** the operation returns a permission-classified error whose guidance names the `sudo` rerun command and absolute config path, and no registration is written

#### Scenario: Non-elevated Windows system scope
- **WHEN** a non-elevated caller requests install with system scope on Windows
- **THEN** the operation returns a permission-classified error whose guidance names the elevated-terminal rerun and absolute config path, and no service is registered

#### Scenario: BusyBox user scope unchanged
- **WHEN** a caller requests a user-scope operation on a host where the BusyBox/SysV backend is selected
- **THEN** the operation behaves as `system-service-management` "BusyBox backend needs root and ignores user scope" specifies and does not invent per-user support

### Requirement: Optional status facts do not cause failure
A supplemental status fact that cannot be gathered SHALL be reported as unavailable, and MUST NOT fail a status call whose verb policy check and native status command succeed.
Source: refactor-codebase design.md Decision 4

#### Scenario: Owner metadata unavailable
- **WHEN** the role is live but owner metadata cannot be read
- **THEN** status reports owner metadata as unavailable and succeeds

### Requirement: Raw status JSON stays byte-identical
CLI `status --json` SHALL forward the validated original status-file bytes followed by one newline, rather than re-serializing the typed value.
Source: refactor-codebase design.md Decision 7; openspec/specs/connection-path-status (Status command output)

#### Scenario: Raw JSON passthrough
- **WHEN** `iroh-tunnel status --json` runs while a role publishes a status file
- **THEN** its output equals the validated file bytes followed by one newline

### Requirement: Updater is a host operation with explicit inputs
The host update operation SHALL accept target executable, version selection, overwrite and insecure decisions instead of `UpdateArgs`, and return an outcome without terminal output. It MUST apply the verification, delegation, version selection and atomic replacement rules of `self-update`.
Source: refactor-codebase design.md Decision 7; openspec/specs/self-update

#### Scenario: Package-manager install is delegated
- **WHEN** the update operation targets an executable owned by a package manager
- **THEN** without overwrite consent it returns a delegation outcome and does not replace the file, as `self-update` "Managed installs are updated through their owner" specifies
