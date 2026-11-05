## MODIFIED Requirements

### Requirement: Pull-request test gate
The system SHALL run, for every pull request and in parallel, only the checks its changed paths need: `lint`, the unit jobs and the e2e jobs for code paths, and `scripts` for script and workflow paths. Test job names SHALL identify the kind and the operating system (`unit (linux)`, `e2e (windows)`, ...), and a failing test job SHALL NOT cancel any other test job. A single `ci-ok` check SHALL always report and SHALL fail when any check failed or was cancelled. The pull-request stage SHALL NOT build release artifacts.
Evidence: .github/workflows/ci.yml (changes, lint, unit, e2e, scripts, ci-ok), design.md D1
Confidence: inferred

#### Scenario: Code change
- **WHEN** a pull request changes a code path: `src/**`, `crates/**`, `tests/**`, `examples/**`, `Cargo.toml`, `Cargo.lock`, `build.rs`, `.github/actions/**` or `.github/workflows/ci.yml`
- **THEN** `lint` (rustfmt check, clippy with warnings denied), the unit jobs on Linux, macOS and Windows, and the e2e jobs on Linux, macOS and Windows run in parallel and `ci-ok` reflects their results

#### Scenario: Installer script change only
- **WHEN** a pull request changes only `install.sh` or `install.ps1`
- **THEN** only `scripts` runs (shellcheck on tracked `*.sh`, PSScriptAnalyzer on `install.ps1`, actionlint on the workflows), and no Rust test is compiled

#### Scenario: Workflow change only
- **WHEN** a pull request changes only `.github/workflows/release.yml`
- **THEN** only `scripts` runs, including actionlint

#### Scenario: Docs-only change
- **WHEN** a pull request changes no code, script or workflow path
- **THEN** every check is skipped and `ci-ok` passes

#### Scenario: A check fails
- **WHEN** `lint`, any unit job, any e2e job or `scripts` fails or is cancelled
- **THEN** `ci-ok` fails

#### Scenario: Superseded push
- **WHEN** a new commit is pushed to a pull request whose run is still in progress
- **THEN** the older run is cancelled

### Requirement: Windows test job includes the user-service lifecycle
The system SHALL, in `e2e (windows)`, run the `service_user_windows` tests with `IROH_TUNNEL_E2E_SERVICE=1` after the e2e suite in the same job, reusing its build, so the tests gated on that variable drive the real Task Scheduler. The opt-in variable SHALL NOT be set for the Linux or macOS jobs, or for any unit job.
Evidence: tests/service_user_windows.rs:1-20, tests/service_user.rs:336, design.md D1, D2
Confidence: inferred

#### Scenario: Lifecycle runs on Windows
- **WHEN** `e2e (windows)` runs
- **THEN** the gated tests (including `user_background_task_lifecycle`: install, start, status, stop, restart and uninstall of a uniquely named task) run against the real Task Scheduler, and the job fails if any of them fails

#### Scenario: Linux unaffected
- **WHEN** a Linux or macOS job runs
- **THEN** `IROH_TUNNEL_E2E_SERVICE` is not set, so `tests/service_user.rs` does not drive the host service manager

### Requirement: Cache stage compiles without running tests
The system SHALL, on a push to `main` that changes a Cargo manifest, `Cargo.lock`, the `setup-rust` action or `cache.yml`, compile the lint target on Linux and the unit and e2e targets on Linux, macOS and Windows, with the same flags and cache keys as the pull-request jobs, and save the caches, without running any test.
Evidence: .github/actions/setup-rust/action.yml, .github/workflows/cache.yml, design.md D5
Confidence: inferred

#### Scenario: Dependency bump merged
- **WHEN** a change to `Cargo.lock` is pushed to `main`
- **THEN** clippy compiles on Linux, the unit targets and the e2e targets compile with `--no-run` on Linux, macOS and Windows, and the caches are saved

#### Scenario: Source-only change merged
- **WHEN** a push to `main` changes only files under `src/`
- **THEN** `cache.yml` does not run

## ADDED Requirements

### Requirement: Unit and end-to-end tests are separate jobs
The system SHALL run unit tests (`--lib --bins`: tests inside the crates' library and binary targets) and end-to-end tests (`--test '*'`: every integration test under `tests/`) as separate jobs on Linux, macOS and Windows. The selectors SHALL be disjoint and together cover every library, binary and integration-test target; doctests, if added, SHALL run in the unit jobs.
Evidence: design.md D1; local run 2026-11-05: 324 unit + 174 e2e = 498, the full workspace count
Confidence: inferred

#### Scenario: Unit failure reported early
- **WHEN** a pull request breaks a unit test under `crates/*/src` or `src`
- **THEN** `unit (<os>)` fails for each affected operating system, and the e2e jobs and the other unit jobs keep running to completion

#### Scenario: New integration test file
- **WHEN** a new file is added under `tests/`
- **THEN** it runs in the e2e jobs without any workflow change, and not in the unit jobs

#### Scenario: Exact coverage
- **WHEN** the unit and e2e selectors are run on a commit without doctests
- **THEN** the sum of their passed tests equals the passed count of `cargo test --workspace --all-features --locked` on that commit (contributor check documented in CONTRIBUTING)

#### Scenario: Doctest added
- **WHEN** a doctest is added to a crate
- **THEN** the unit jobs run it (`--doc`), and the e2e jobs do not

### Requirement: Relay e2e runs against a self-hosted relay
The system SHALL, in `e2e (linux)` and regardless of the e2e suite result, run `tests/serve_access_tunnel.rs` against a self-hosted `n0computer/iroh-relay` container (tag = the `iroh` version in `Cargo.lock`, HTTPS, job-generated certificate) named by `IROH_TUNNEL_TEST_RELAY_URL`. This suite SHALL NOT use public relays in CI; without the variable it SHALL behave as before.
Evidence: design.md D3, D4; crates/engine/src/endpoint/create.rs:32-35
Confidence: inferred

#### Scenario: Relay suite in CI
- **WHEN** the Linux e2e job runs
- **THEN** the relay container reports healthy, every `serve_access_tunnel` test runs with `--ignored` against the self-hosted relay and passes, and the container is removed afterwards

#### Scenario: Relay does not start
- **WHEN** the relay container is not healthy within 60 seconds
- **THEN** the job prints the container logs and fails

#### Scenario: Local run without a relay
- **WHEN** a developer runs `cargo test` without `IROH_TUNNEL_TEST_RELAY_URL`
- **THEN** the `serve_access_tunnel` tests are ignored and no network relay is contacted
