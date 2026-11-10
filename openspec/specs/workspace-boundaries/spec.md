# workspace-boundaries Specification

## Purpose

Define the three-package workspace (root, host, engine) with a one-way dependency graph from root to host to engine, and which package owns tunnel execution, persistence and platform policy, and terminal interaction. It also fixes runtime task and drain ownership, and the validation gates every member must pass.

## Requirements

### Requirement: Three-package workspace
The repository SHALL be a Cargo workspace of exactly three packages: the root package `iroh-tunnel` (CLI and distribution composition root), `iroh-tunnel-host` at `crates/host`, and `iroh-tunnel-engine` at `crates/engine`. Internal packages SHALL NOT be published as a supported SDK.
Source: design.md Decision 1

#### Scenario: Workspace members and default members
- **WHEN** `cargo metadata --format-version 1 --no-deps` runs at the repository root
- **THEN** the workspace members are exactly `iroh-tunnel`, `iroh-tunnel-host` and `iroh-tunnel-engine`, all three are default members, and the resolver is `2`

#### Scenario: Internal packages are not publishable
- **WHEN** the manifests of `iroh-tunnel-host` and `iroh-tunnel-engine` are inspected
- **THEN** each sets `publish = false`

### Requirement: One-way production dependency graph
The production dependency graph MUST be `iroh-tunnel → iroh-tunnel-host → iroh-tunnel-engine`. Neither internal package SHALL depend on a package above it, including through build or dev dependencies.
Source: design.md Decision 1

#### Scenario: Engine depends on neither host nor root
- **WHEN** the resolved normal, build and dev dependencies of `iroh-tunnel-engine` are inspected
- **THEN** they contain neither `iroh-tunnel-host` nor `iroh-tunnel`

#### Scenario: Host does not depend on the root package
- **WHEN** the resolved normal, build and dev dependencies of `iroh-tunnel-host` are inspected
- **THEN** they do not contain `iroh-tunnel`

### Requirement: Engine owns tunnel execution
`iroh-tunnel-engine` SHALL own execution settings, protocol/ALPN, endpoint construction, Serve and Access forwarding, pipes, UDP decisions and queues, connection observations and the per-run task tree.
Source: design.md Decisions 1, 2

#### Scenario: UDP decision model stays together
- **WHEN** the source locations of Flow admission, Session incarnation rules, Quarantine accounting, Flow cap and Datagram limit are listed
- **THEN** all of them are in `iroh-tunnel-engine`

### Requirement: Engine performs no persistence, platform or presentation effects
`iroh-tunnel-engine` MUST NOT read or write configuration or status files, define the status-file JSON schema, control OS services, install signal handlers, call `std::process::exit`, parse CLI arguments or write user-facing output. Its diagnostics SHALL remain on `tracing`.
Source: design.md Decisions 1, 6

#### Scenario: Startup readiness is reported as a value
- **WHEN** a Serve or Access role finishes startup inside the engine
- **THEN** the engine delivers a typed ready description to a caller-supplied output hook before forwarding admission and writes nothing to stdout itself

#### Scenario: Status observations carry no file schema
- **WHEN** the engine produces a connection or UDP observation for status publishing
- **THEN** it passes a native observation value to an injected save callback, and mapping to the status-file JSON schema happens in `iroh-tunnel-host`

#### Scenario: No exit, signal or print calls
- **WHEN** non-test `iroh-tunnel-engine` sources are searched for `process::exit`, signal-handler installation, `println!` and `eprintln!`
- **THEN** none occur

### Requirement: Host owns persistence and platform policy
`iroh-tunnel-host` SHALL be the single owner of configuration storage, identity persistence, path resolution, trust and ownership checks, foreground orchestration, status-file adapters, OS service backends and the updater. The root package SHALL NOT duplicate that policy.
Source: design.md Decisions 1, 3, 4

#### Scenario: CLI mutations go through host operations
- **WHEN** `iroh-tunnel config set`, `config rm`, `keygen` or `serve service install` mutates a file or registration
- **THEN** a host operation applies the lock, validation, privilege and trust policy, and the CLI only translates arguments, collects decisions and renders the outcome

#### Scenario: Creation requirements decided by host
- **WHEN** `config set` needs to know whether an entry exists to decide which flags are required
- **THEN** the host decides under the writer lock and returns a domain error, and the CLI alone turns it into a usage error with help

### Requirement: Host performs no terminal interaction
`iroh-tunnel-host` MUST NOT depend on clap, read from stdin, render tables or prompts, or call `std::process::exit`.
Source: design.md Decision 1

#### Scenario: Host has no clap dependency
- **WHEN** the normal dependency graph of `iroh-tunnel-host` is inspected
- **THEN** it does not include `clap`

#### Scenario: No exit or stdin reads
- **WHEN** non-test `iroh-tunnel-host` sources are searched for `process::exit` and reads from stdin
- **THEN** none occur

### Requirement: Run-scoped drain context
Each foreground role invocation SHALL create its own drain context, shared by its engine task tree and the CLI watchdog observer. No drain deadline SHALL be process-global. The drain grace, the watchdog period, the repeated-signal policy and the forced exit codes MUST follow `role-ownership-shutdown`.
Source: refactor-codebase design.md Decision 6

#### Scenario: Two runs in one process do not share a deadline
- **WHEN** two foreground role runs start sequentially in the same test process and the first is drained
- **THEN** the second run starts with a fresh, unexpired drain context governed only by its own deadline

#### Scenario: Forced exit codes are unchanged
- **WHEN** a foreground role receives a stop signal after the drain grace has elapsed, or its shutdown has not finished 10 seconds after the drain deadline
- **THEN** the process prints "shutdown abandoned" and exits with code 130 for the signal or code 1 for the watchdog, as `role-ownership-shutdown` "Repeated stop signal forces exit after the grace" and "Stuck shutdown watchdog" and `cli-entrypoints` "Forced shutdown exit" specify

### Requirement: Management calls do not touch lifecycle state
Management-only host operations SHALL NOT install signal handlers and SHALL NOT trigger or inherit any foreground run's drain context.
Source: design.md Decision 6

#### Scenario: Management call installs no signal handler
- **WHEN** a host configuration, service or status operation runs in a process with no foreground role
- **THEN** no signal handler is installed and no drain context is created or observed

### Requirement: Runtime tasks have explicit owners
Every task spawned by a running role SHALL have an owner that retains its handle, observes its failure and joins or aborts it before the role future completes. Channel, connection and Quarantine sink tasks SHALL be joined or aborted by the drain deadline of `role-ownership-shutdown` "Single drain deadline of three seconds"; the status flush task SHALL follow the bound of "Status file removal requires writer quiescence".
Source: refactor-codebase design.md Decision 6

#### Scenario: No task survives the role
- **WHEN** a role future completes after graceful stop
- **THEN** no task spawned by that role, including status flush and Quarantine sink tasks, is still running

### Requirement: Quarantine sinks are owned for the role lifetime
UDP Quarantine sinks MUST outlive their Session incarnation but not role shutdown, and their Flow cap slot MUST be released on every exit path. Quarantine duration, caps and late-reply behaviour SHALL follow `udp-datagram-forwarding` "Quarantine of expired upstream sockets", and known deviations SHALL be those documented in `docs/udp-integration-contract.md`.
Source: refactor-codebase design.md Decision 6; docs/udp-integration-contract.md

#### Scenario: Quarantine sink outlives its connection
- **WHEN** an Access connection whose Session incarnation has Flows in Quarantine disconnects while the role keeps running
- **THEN** the Quarantine sinks continue for the 30-second Quarantine duration and the disconnect does not wait for that duration

#### Scenario: Quarantine sinks stop with the role
- **WHEN** the role is gracefully stopped while Quarantine sinks are active
- **THEN** every sink task is joined or aborted by the drain deadline

#### Scenario: Slot released on sink failure
- **WHEN** a Quarantine sink task fails or is aborted
- **THEN** its Flow cap slot is released and the failure is observed by the role-lifetime owner

### Requirement: Status file removal requires writer quiescence
On graceful stop, the owned status file SHALL be removed only after the status flush task has joined within five seconds and the endpoint has closed. Otherwise graceful cleanup MUST leave the file in place, to be reported as stale as `connection-path-status` "Stale snapshot warnings" specifies.
Source: refactor-codebase design.md Decision 6; openspec/specs/connection-path-status

#### Scenario: Writer joins in time
- **WHEN** a role stops gracefully and the status flush task joins within five seconds
- **THEN** the owned status file is removed after the endpoint closes

#### Scenario: Writer does not quiesce
- **WHEN** the status flush task does not join within five seconds or fails during join
- **THEN** graceful cleanup does not remove the status file

### Requirement: Status publishing ordering and change detection are preserved
The status callback SHALL keep write-before-admission ordering, the change-only flush cadence of `connection-path-status` "Status flush cadence", initial-success seeding and retry after a failed write. Two observations MUST be equal exactly when their rendered status snapshots are equal, and equality MUST NOT include a sampling timestamp.
Source: refactor-codebase design.md Decision 6; openspec/specs/connection-path-status

#### Scenario: Initial snapshot precedes admission
- **WHEN** a Serve role starts with status publishing enabled
- **THEN** the initial status file is written before the first inbound connection is admitted

#### Scenario: Unchanged observation is not rewritten
- **WHEN** two consecutive flush intervals produce observations with equal rendered values
- **THEN** the status file is not rewritten for the second interval

#### Scenario: Failed write is retried
- **WHEN** a status write fails and the next observation is unchanged
- **THEN** the next flush interval retries the write

### Requirement: Secret-bearing values redact their Debug output
Every workspace type that holds a node `secret_key` or other secret material, including enclosing requests, DTOs, edit attempts and outcomes, SHALL implement `Debug` without revealing it. Redaction MUST NOT change the TOML storage format or the key encoding that `tunnel-configuration` specifies.
Source: refactor-codebase design.md Decisions 3, 8

#### Scenario: Debug of a loaded configuration
- **WHEN** a configuration loaded from a file containing a `secret_key` is formatted with `{:?}`
- **THEN** the output does not contain the key material

#### Scenario: Error context does not echo secrets
- **WHEN** a host operation fails while handling a document that contains a secret line
- **THEN** neither the error chain's `Display` nor `Debug` output contains that line

### Requirement: Exit classification survives package boundaries
`CliError` SHALL live in `iroh-tunnel-host` and keep its downcast identity through `anyhow` context, so the CLI maps success 0, unclassified 1, config 2, permission 3, Iroh 4, service 5 and conflict 6. `iroh-tunnel-engine` MUST NOT import `CliError`.
Source: refactor-codebase design.md Decision 8; openspec/specs/cli-entrypoints

#### Scenario: Permission failure keeps exit code 3
- **WHEN** a non-root user runs `iroh-tunnel serve service install --system`
- **THEN** the process exits with code 3

#### Scenario: Ownership conflict keeps exit code 6
- **WHEN** `iroh-tunnel serve` starts while another Serve role of the same user is running
- **THEN** the process exits with code 6

#### Scenario: Invalid configuration keeps exit code 2
- **WHEN** `iroh-tunnel serve config list` loads a config file that cannot be parsed
- **THEN** the process exits with code 2

### Requirement: Test-only TLS override stays in the explicit test feature graph
The `test-utils` feature SHALL be forwarded explicitly from `iroh-tunnel` through `iroh-tunnel-host` to `iroh-tunnel-engine`. No default feature, normal dependency or shared workspace declaration MUST enable it, and no self dev-dependency SHALL create a package cycle.
Source: design.md Decision 9

#### Scenario: Release graph excludes test-utils
- **WHEN** the normal feature graph of `iroh-tunnel` is resolved without explicit features
- **THEN** neither any workspace `test-utils` feature nor `iroh/test-utils` is enabled

#### Scenario: Test graph enables the override explicitly
- **WHEN** workspace tests run with `--all-features`
- **THEN** local-relay tests that need the TLS override compile and run against the in-process relay only

### Requirement: Validation gates cover every member
The validation gates SHALL run workspace-wide on Rust 1.91: `fmt --all`, `clippy --workspace --all-targets --all-features --locked -D warnings`, `test --workspace --all-features --locked` and `doc --workspace --no-deps --locked` with rustdoc warnings denied; the doc gate covers items compiled for the target it runs on. Default-feature and release builds without test features MUST also succeed.
Source: refactor-codebase design.md Decision 9; cleanup-config-dto-rustdoc design.md D5

#### Scenario: Gates fail on a member-only defect
- **WHEN** a clippy warning or failing test exists only in `iroh-tunnel-engine`
- **THEN** the workspace-wide gate commands fail

#### Scenario: Doc gate fails on a member-only broken link
- **WHEN** a broken intra-doc link exists only in `iroh-tunnel-host`, on an item compiled for the target the gate runs on
- **THEN** the workspace-wide `doc` gate fails

### Requirement: Binary and version identities are preserved
The root package SHALL own the three binary targets `iroh-tunnel`, `iroh-tunnel-serve` and `iroh-tunnel-access` with the aliases `cli-entrypoints` specifies, root `build.rs` `APP_VERSION` injection, and one product version shared by all three packages and binaries, including the updater user-agent.
Source: refactor-codebase design.md Decisions 1, 9

#### Scenario: Binary targets after the move
- **WHEN** `cargo build --release --locked` runs at the repository root
- **THEN** `target/release/` contains `iroh-tunnel`, `iroh-tunnel-serve` and `iroh-tunnel-access` (with the platform executable suffix), and each reports the `APP_VERSION` that `install-release-packaging` "Build version derived from git tag" specifies

#### Scenario: Build workflow reads the workspace version
- **WHEN** the CI build workflow determines the product version
- **THEN** it reads the version inherited by all three packages rather than a literal root `[package].version`

### Requirement: Distribution entrypoints follow moved paths
`install.sh` and `install.ps1` SHALL live at the repository root, the source `Dockerfile` SHALL install `/usr/local/bin/iroh-tunnel`, and release archive, checksum and signature names SHALL follow `install-release-packaging`. CI path filters, Docker build contexts and release scripts SHALL change in the same increment as the paths they reference.
Source: refactor-codebase design.md Decision 9; openspec/specs/install-release-packaging

#### Scenario: Docker build includes member packages
- **WHEN** the repository Dockerfile builds the release image
- **THEN** all workspace member manifests and sources are present and the image contains `/usr/local/bin/iroh-tunnel`

#### Scenario: Installer URLs unchanged
- **WHEN** the documented installer URLs are resolved against the repository
- **THEN** they point to `install.sh` and `install.ps1` at the repository root

### Requirement: Public internal APIs are justified and documented
Every `pub` item of `iroh-tunnel-host` and `iroh-tunnel-engine` MUST have rustdoc (`#![deny(missing_docs)]`), and public operations SHALL document ownership, cancellation and errors. A change that makes an item of either package `pub`, by adding it or by widening its visibility, SHALL name a caller in another workspace package. Test-only entrypoints, ownership bypasses, raw backend dispatch, OS command construction, trust handles and lock internals MUST NOT be public without `test-utils`.
Source: refactor-codebase design.md Decision 8

#### Scenario: Former test bypass is not a production API
- **WHEN** the public API of `iroh-tunnel-host` is built without `test-utils`
- **THEN** no entrypoint that runs a role while skipping the ownership claim is exported

### Requirement: Configuration documents carry no filesystem behaviour
`ServeConfig`, `AccessConfig` and the crate-private `RoleDoc` SHALL hold only schema, validation and node-section access. Reading, writing and key persistence MUST be crate-private host `config` store operations, which apply the writer lock, key adoption and the load diagnostics below.
Source: refactor-codebase design.md Decision 3; cleanup-config-dto-rustdoc design.md D1–D3

#### Scenario: Public DTO API has no file methods
- **WHEN** the public API of `iroh-tunnel-host` is built
- **THEN** neither `ServeConfig` nor `AccessConfig` exposes a `load`, `save`, `resolve_and_save_key` or other method that takes a filesystem path

#### Scenario: No file methods remain on the documents internally
- **WHEN** the non-test sources of `iroh-tunnel-host` are inspected
- **THEN** neither the DTOs nor `RoleDoc` define a method that reads or writes a file, the replacement load, parse and key-persistence functions are `pub(crate)` store functions, and no whole-document save function exists

#### Scenario: Run still persists a generated key
- **WHEN** `run` starts a role whose config has an empty `secret_key`, and the file still has no key when the writer lock is taken
- **THEN** a key is generated and written into `[node].secret_key` under the writer lock, legacy `multiplex` fields are removed, and the started NodeId matches the persisted key

#### Scenario: Run adopts a concurrently persisted key
- **WHEN** `run` loaded a config with an empty `secret_key`, but another process persisted a valid key before the writer lock was taken
- **THEN** the role adopts that key without rewriting the file, and the started NodeId matches the adopted key

#### Scenario: Load diagnostics are unchanged
- **WHEN** a role config is missing or contains a TOML syntax error on a line that holds a secret
- **THEN** the error reads `failed to read config: <path>` or `failed to parse config: <path> (line N): <message>`, and it does not contain the secret

#### Scenario: Legacy fields still ignored and dropped on serialize
- **WHEN** a config carrying any legacy `multiplex` value is parsed and serialized again
- **THEN** the parsed document equals the one without the field, and the serialized text has no `multiplex` key

### Requirement: Public rustdoc links resolve
`cargo doc --workspace --no-deps --locked` on Rust 1.91 SHALL complete with no warnings. `iroh-tunnel-host` and `iroh-tunnel-engine` MUST deny `rustdoc::broken_intra_doc_links` and `rustdoc::private_intra_doc_links`, so public documentation never links to an unresolved or crate-private item.
Source: issue #167; cleanup-config-dto-rustdoc design.md D5

#### Scenario: Clean doc build
- **WHEN** `RUSTDOCFLAGS="-D warnings" cargo +1.91 doc --workspace --no-deps --locked` runs on Linux
- **THEN** it succeeds with no warnings

#### Scenario: Broken link is rejected
- **WHEN** a doc comment on an item compiled for the current target in `iroh-tunnel-host` or `iroh-tunnel-engine` links to an item that does not resolve or is crate-private while the documented item is public
- **THEN** `cargo doc` for that crate fails

### Requirement: Drop counter docs expose the unusable-datagram deviation
The rustdoc of the `limit_shrink` drop counter, in both the engine observation and the host status schema, SHALL state that until #152 is resolved a connection whose datagrams are never usable also counts every datagram as `limit_shrink`, consistent with `docs/udp-integration-contract.md`.
Source: issue #167, #152

#### Scenario: Deviation visible in API docs
- **WHEN** the rustdoc of `limit_shrink` in `iroh-tunnel-engine` and `iroh-tunnel-host` is read
- **THEN** both mention the never-usable-datagrams case and reference #152

### Requirement: Engine dependencies are inherited from the workspace
Every dependency of `iroh-tunnel-engine` MUST be inherited from the root manifest's `[workspace.dependencies]`. This covers its normal, build and dev dependency tables, including target-specific ones: each entry SHALL be declared with `workspace = true`, and none SHALL carry its own version, path or git source.
Source: issue #164 (replaces the refactor-codebase lockfile baseline check)

#### Scenario: Engine manifest inherits every dependency
- **WHEN** every dependency table of `crates/engine/Cargo.toml` is inspected, including `[target.<cfg>.*]` tables
- **THEN** every entry is a table with `workspace = true`

#### Scenario: Directly versioned engine dependency is rejected
- **WHEN** an entry such as `foo = "1"` or `foo = { path = "../foo" }` is added to any engine dependency table
- **THEN** the workspace layout test fails and names the entry

### Requirement: Validated types accept every specified input
Validated or semantic types in the workspace MUST accept every input that the capability specs accept, and an otherwise valid role configuration MUST load when it contains fields its schema does not define. A validated type MUST NOT report a failure at an earlier stage than the capability spec that owns the rule.
Source: refactor-codebase design.md Decision 2; evidence crates/host/src/config/dto.rs (unit test unknown_fields_and_legacy_multiplex_still_load)

#### Scenario: Unknown fields are ignored
- **WHEN** an otherwise valid role config containing a field its schema does not define, such as `[node] foo = 1`, is loaded
- **THEN** it loads without error

#### Scenario: Legacy multiplex keys load
- **WHEN** an otherwise valid access config whose service carries `multiplex = true` is loaded
- **THEN** it loads, as `tunnel-configuration` "Legacy multiplex field is ignored and dropped on save" specifies

#### Scenario: Hostname target is kept
- **WHEN** an otherwise valid serve config whose service has `host = "db.internal"` is loaded
- **THEN** it loads, and the service host is `db.internal`

#### Scenario: Schema validation fails at load
- **WHEN** `iroh-tunnel serve config check --config <path>` loads an otherwise valid file whose service has `port = 0`
- **THEN** loading fails with a configuration error naming the path and `invalid port`, and the process exits with code 2

#### Scenario: Secret key is checked at key resolution
- **WHEN** a role config whose `secret_key` is not valid base64 is parsed
- **THEN** parsing succeeds, and the key is rejected when it is resolved, as `tunnel-configuration` "Secret key is a base64 32-byte value generated and persisted on first use" specifies
