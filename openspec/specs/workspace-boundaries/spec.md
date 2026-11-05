# workspace-boundaries Specification

## Purpose

TBD

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

### Requirement: No new external dependencies
The workspace split MUST NOT add an external crate that the pre-change `Cargo.lock` did not already resolve.
Source: proposal.md Dependencies and scope

#### Scenario: Lockfile crate set comparison
- **WHEN** the external crate names in `Cargo.lock` after the change are compared with those before the change
- **THEN** no new external crate name appears

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
- **WHEN** `iroh-tunnel config add`, `config remove`, `keygen` or `serve service install` mutates a file or registration
- **THEN** a host operation applies the lock, validation, privilege and trust policy, and the CLI only translates arguments, collects decisions and renders the outcome

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
Each foreground role invocation SHALL create its own drain context shared by its engine task tree and the CLI watchdog observer, replacing the process-global first-wins `DRAIN_DEADLINE`. Drain and watchdog budgets, repeated-signal policy and forced exit codes MUST remain unchanged.
Source: design.md Decision 6

#### Scenario: Two runs in one process do not share a deadline
- **WHEN** two foreground role runs start sequentially in the same test process and the first is drained
- **THEN** the second run starts with a fresh, unexpired drain context governed only by its own deadline

#### Scenario: Forced exit codes are unchanged
- **WHEN** a foreground role receives a repeated shutdown signal or exceeds the watchdog budget
- **THEN** the process exits with the same code and stderr classification as before the refactor

### Requirement: Management calls do not touch lifecycle state
Management-only host operations SHALL NOT install signal handlers and SHALL NOT trigger or inherit any foreground run's drain context.
Source: design.md Decision 6

#### Scenario: Management call installs no signal handler
- **WHEN** a host configuration, service or status operation runs in a process with no foreground role
- **THEN** no signal handler is installed and no drain context is created or observed

### Requirement: Runtime tasks have explicit owners
Every task spawned by a running role SHALL have an owner that retains its handle, observes its failure and joins or aborts it within the existing role stop deadline.
Source: design.md Decision 6

#### Scenario: No task survives the role
- **WHEN** a role future completes after graceful stop
- **THEN** no task spawned by that role, including status flush and Quarantine sink tasks, is still running

### Requirement: Quarantine sinks are owned for the role lifetime
UDP Quarantine sinks MUST outlive their Session incarnation but not role shutdown, and their Flow cap slot MUST be released on every exit path. Quarantine duration, caps, late-reply behaviour and documented UDP deviations SHALL remain unchanged.
Source: design.md Decision 6; docs/udp-integration-contract.md

#### Scenario: Quarantine sink outlives its connection
- **WHEN** an Access connection whose Session incarnation has Flows in Quarantine disconnects while the role keeps running
- **THEN** the Quarantine sinks continue for the existing Quarantine duration and the disconnect does not wait for that duration

#### Scenario: Quarantine sinks stop with the role
- **WHEN** the role is gracefully stopped while Quarantine sinks are active
- **THEN** every sink task is joined or aborted within the existing role stop deadline

#### Scenario: Slot released on sink failure
- **WHEN** a Quarantine sink task fails or is aborted
- **THEN** its Flow cap slot is released and the failure is observed by the role-lifetime owner

### Requirement: Status file removal requires writer quiescence
On graceful stop, the owned status file SHALL be removed only after the status flush task has joined within the existing five-second bound and the endpoint has closed. Otherwise the existing stale-file fallback MUST apply.
Source: design.md Decision 6; openspec/specs/connection-path-status

#### Scenario: Writer joins in time
- **WHEN** a role stops gracefully and the status flush task joins within five seconds
- **THEN** the owned status file is removed after the endpoint closes

#### Scenario: Writer does not quiesce
- **WHEN** the status flush task does not join within five seconds or fails during join
- **THEN** graceful cleanup does not remove the status file

### Requirement: Status publishing ordering and change detection are preserved
The status callback SHALL preserve write-before-admission ordering, the approximately five-second change-only flush cadence, initial-success seeding and retry after a failed write. Observation equality MUST match the previous rendered-value change detection and MUST NOT include a sampling timestamp.
Source: design.md Decision 6; openspec/specs/connection-path-status

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
Every workspace type that holds a node `secret_key` or other secret material, including enclosing requests, DTOs, edit attempts and outcomes, SHALL implement `Debug` without revealing it. The TOML storage format and key encoding MUST remain unchanged.
Source: design.md Decisions 3, 8

#### Scenario: Debug of a loaded configuration
- **WHEN** a configuration loaded from a file containing a `secret_key` is formatted with `{:?}`
- **THEN** the output does not contain the key material

#### Scenario: Error context does not echo secrets
- **WHEN** a host operation fails while handling a document that contains a secret line
- **THEN** neither the error chain's `Display` nor `Debug` output contains that line

### Requirement: Exit classification survives package boundaries
`CliError` SHALL live in `iroh-tunnel-host` and keep its downcast identity through `anyhow` context, so the CLI still maps success 0, unclassified 1, config 2, permission 3, Iroh 4, service 5 and conflict 6. `iroh-tunnel-engine` MUST NOT import `CliError`.
Source: design.md Decision 8; openspec/specs/cli-entrypoints

#### Scenario: Permission failure keeps exit code 3
- **WHEN** a non-root user runs `iroh-tunnel serve service install --system`
- **THEN** the process exits with code 3

#### Scenario: Ownership conflict keeps exit code 6
- **WHEN** `iroh-tunnel serve` starts while another Serve role of the same user is running
- **THEN** the process exits with code 6

#### Scenario: Invalid configuration keeps exit code 2
- **WHEN** `iroh-tunnel serve --config <path>` loads a file that fails existing validation
- **THEN** the process exits with code 2

### Requirement: Observable contracts are unchanged
The restructuring SHALL preserve every behaviour specified by the eleven existing capability specs, including executable names, CLI output and exit codes, config and status formats, service policies and ALPN/wire compatibility.
Source: proposal.md Compatibility

#### Scenario: Existing capability tests still pass
- **WHEN** the requirement-linked tests of the eleven existing capabilities run against the workspace
- **THEN** they pass, and any changed assertion maps to an approved, documented reason

### Requirement: Validated types do not tighten acceptance
Introducing a validated or semantic type MUST NOT reject a previously accepted input or move a failure to an earlier stage.
Source: design.md Decision 2

#### Scenario: Previously accepted configuration still loads
- **WHEN** a config with unknown fields, legacy `multiplex` keys or a hostname target that loaded before the change is loaded after it
- **THEN** it yields the same result, and any failure occurs at the same stage as before

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
The existing gates SHALL run workspace-wide on Rust 1.91: `fmt --all`, `clippy --workspace --all-targets --all-features --locked -D warnings` and `test --workspace --all-features --locked`. Default-feature and release builds without test features MUST also succeed.
Source: design.md Decision 9

#### Scenario: Gates fail on a member-only defect
- **WHEN** a clippy warning or failing test exists only in `iroh-tunnel-engine`
- **THEN** the workspace-wide gate commands fail

### Requirement: No test is lost by relocation
An inventory of discovered test names, including ignored and opt-in cases, taken before the change MUST map to tests discovered after it.
Source: design.md Decision 9

#### Scenario: Test inventory comparison
- **WHEN** pre-change and post-change `cargo test --workspace --all-features -- --list` inventories, including ignored tests, are compared
- **THEN** every pre-change test maps to a post-change test or to a documented split, merge or removal reason

### Requirement: Binary and version identities are preserved
The root package SHALL keep the three binary target names and aliases, root `build.rs` `APP_VERSION` injection, and one product version shared by all three packages and binaries, including the updater user-agent.
Source: design.md Decisions 1, 9

#### Scenario: Binary targets after the move
- **WHEN** `cargo build --release --locked` runs at the repository root
- **THEN** the same three executables are produced at the same target paths and report the same `--version` format with the workspace version

#### Scenario: Build workflow reads the workspace version
- **WHEN** the CI build workflow determines the product version
- **THEN** it reads the version inherited by all three packages rather than a literal root `[package].version`

### Requirement: Distribution entrypoints follow moved paths
Root `install.sh`/`install.ps1` and their URLs, Dockerfile outputs, and release archive, checksum and signature names SHALL be preserved. CI path filters, Docker build contexts and release scripts SHALL change in the same increment as the paths they reference.
Source: design.md Decision 9; openspec/specs/install-release-packaging

#### Scenario: Docker build includes member packages
- **WHEN** the repository Dockerfile builds the release image
- **THEN** all workspace member manifests and sources are present and the image has the same executable paths

#### Scenario: Installer URLs unchanged
- **WHEN** the documented installer URLs are resolved against the repository
- **THEN** they still point to root `install.sh` and `install.ps1`

### Requirement: Public internal APIs are justified and documented
Every widened `pub` item in `iroh-tunnel-host` or `iroh-tunnel-engine` SHALL have an in-workspace caller and rustdoc covering ownership, cancellation and errors. Test-only entrypoints, ownership bypasses, raw backend dispatch, OS command construction, trust handles and lock internals MUST NOT be public without `test-utils`.
Source: design.md Decision 8

#### Scenario: Former test bypass is not a production API
- **WHEN** the public API of `iroh-tunnel-host` is built without `test-utils`
- **THEN** no entrypoint that runs a role while skipping the ownership claim is exported

### Requirement: Rust API migration map
The change MUST document an old-to-new map of module paths, removed public items and repository paths for every caller-visible Rust API change.
Source: proposal.md BREAKING — Rust API

#### Scenario: Migration map covers moved modules
- **WHEN** a reviewer looks up a former public path such as `iroh_tunnel::config` or `iroh_tunnel::service`
- **THEN** the map names its new location or states that it became private, with the replacement operation
