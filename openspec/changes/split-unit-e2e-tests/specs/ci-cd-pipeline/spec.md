## MODIFIED Requirements

### Requirement: Three single-purpose pipeline stages
The system SHALL organise CI/CD into exactly four workflows, each started by one kind of event and none by manual dispatch: `ci.yml` on pull requests (unit test stage), `e2e.yml` on every push to `main` (end-to-end stage), `cache.yml` on pushes to `main` that change dependencies or toolchain setup (cache stage), and `release.yml` on `v*` tag pushes (release stage, which ends in deployment). No job SHALL run in more than one stage, and no other workflow SHALL exist in `.github/workflows/`.
Evidence: design.md D1, D7
Confidence: inferred

#### Scenario: Workflow inventory
- **WHEN** `.github/workflows/` is listed
- **THEN** it contains `ci.yml`, `e2e.yml`, `cache.yml` and `release.yml` and nothing else

#### Scenario: Merge to main
- **WHEN** a pull request is merged into `main`
- **THEN** `e2e.yml` runs on the merge commit; `cache.yml` runs only when its paths changed; no build, package or publish job runs

### Requirement: Pull-request test gate
The system SHALL run, for every pull request and in parallel, only the checks its changed paths need: `lint` and the unit jobs for code paths, and `scripts` for script and workflow paths. Pull requests SHALL NOT run end-to-end tests or build release artifacts. Unit job names SHALL identify the operating system (`unit (linux)`, `unit (macos)`, `unit (windows)`), and a failing unit job SHALL NOT cancel the others. A single `ci-ok` check SHALL always report and SHALL fail when any check failed or was cancelled.
Evidence: .github/workflows/ci.yml (changes, lint, unit, scripts, ci-ok), design.md D1
Confidence: inferred

#### Scenario: Code change
- **WHEN** a pull request changes a code path: `src/**`, `crates/**`, `tests/**`, `examples/**`, `Cargo.toml`, `Cargo.lock`, `build.rs`, `.github/actions/**` or `.github/workflows/ci.yml`
- **THEN** `lint` (rustfmt check, clippy with warnings denied) and the unit jobs on Linux, macOS and Windows run in parallel, no e2e test runs, and `ci-ok` reflects their results

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
- **WHEN** `lint`, any unit job or `scripts` fails or is cancelled
- **THEN** `ci-ok` fails

#### Scenario: Superseded push
- **WHEN** a new commit is pushed to a pull request whose run is still in progress
- **THEN** the older run is cancelled

### Requirement: Windows test job includes the user-service lifecycle
The system SHALL, in `e2e (windows)` of `e2e.yml`, run the `service_user_windows` tests with `IROH_TUNNEL_E2E_SERVICE=1` after the e2e suite in the same job, reusing its build, so the tests gated on that variable drive the real Task Scheduler. The opt-in variable SHALL NOT be set for any Linux or macOS job, or for any unit job.
Evidence: tests/service_user_windows.rs:1-20, tests/service_user.rs:336, design.md D1, D2, D7
Confidence: inferred

#### Scenario: Lifecycle runs on Windows
- **WHEN** `e2e (windows)` runs
- **THEN** the gated tests (including `user_background_task_lifecycle`: install, start, status, stop, restart and uninstall of a uniquely named task) run against the real Task Scheduler, and the job fails if any of them fails

#### Scenario: Linux unaffected
- **WHEN** a Linux or macOS job runs
- **THEN** `IROH_TUNNEL_E2E_SERVICE` is not set, so `tests/service_user.rs` does not drive the host service manager

### Requirement: Cache stage compiles without running tests
The system SHALL, on a push to `main` that changes a Cargo manifest, `Cargo.lock`, the `setup-rust` action or `cache.yml`, compile the lint target on Linux and the unit targets on Linux, macOS and Windows, with the same flags and cache keys as the pull-request jobs, and save the caches, without running any test.
Evidence: .github/actions/setup-rust/action.yml, .github/workflows/cache.yml, design.md D5
Confidence: inferred

#### Scenario: Dependency bump merged
- **WHEN** a change to `Cargo.lock` is pushed to `main`
- **THEN** clippy compiles on Linux, the unit targets compile with `--no-run` on Linux, macOS and Windows, and the caches are saved

#### Scenario: Source-only change merged
- **WHEN** a push to `main` changes only files under `src/`
- **THEN** `cache.yml` does not run

### Requirement: Release preflight requires a passed test gate
The system SHALL, before building a release, require a `vX.Y.Z[-<pre>]` tag on a commit reachable from `main` that is the merge commit of a pull request whose most recent `ci-ok` check run on its head commit succeeded, and whose own most recent `e2e-ok` check run succeeded. It SHALL NOT rerun lint or tests, and a failed preflight SHALL stop the release before any build. The aggregate checks SHALL keep the names `ci-ok` and `e2e-ok`.
Evidence: .github/workflows/release.yml (preflight), design.md D8
Confidence: inferred

#### Scenario: Commit passed CI
- **WHEN** `v1.2.3` is pushed on a merge commit whose pull request head has a successful `ci-ok` and which itself has a successful `e2e-ok`
- **THEN** preflight passes and outputs version `1.2.3` and pre-release `false`

#### Scenario: Pre-release tag
- **WHEN** `v1.2.3-rc.1` passes preflight
- **THEN** pre-release is `true`

#### Scenario: Commit without a green gate
- **WHEN** the tagged commit is not the merge commit of a pull request, or that pull request's most recent `ci-ok` did not succeed
- **THEN** the release fails at preflight and nothing is built or published

#### Scenario: E2E red or missing on the tagged commit
- **WHEN** the tagged commit's most recent `e2e-ok` failed, was cancelled, or does not exist yet
- **THEN** the release fails at preflight with a message naming `e2e-ok`, and nothing is built or published

#### Scenario: Tag off main
- **WHEN** the tagged commit is not reachable from `main`
- **THEN** the release fails at preflight

#### Scenario: Malformed tag
- **WHEN** a tag such as `v1.2` is pushed
- **THEN** the release fails at preflight

## ADDED Requirements

### Requirement: Unit and end-to-end tests are separate jobs
The system SHALL run unit tests (`--lib --bins`) in the pull-request stage and end-to-end tests (`--test '*'`, every file under `tests/`) in the end-to-end stage, each on Linux, macOS and Windows. The selectors SHALL be disjoint and together cover every library, binary and integration-test target; doctests SHALL run with the unit tests.
Evidence: design.md D1; local run 2026-11-05: 324 unit + 174 e2e = 498, the full workspace count
Confidence: inferred

#### Scenario: Unit failure reported early
- **WHEN** a pull request breaks a unit test under `crates/*/src` or `src`
- **THEN** `unit (<os>)` fails for each affected operating system, and the other unit jobs keep running to completion

#### Scenario: New integration test file
- **WHEN** a new file is added under `tests/`
- **THEN** it runs in the e2e jobs on `main` without any workflow change, and not in the unit jobs

#### Scenario: Exact coverage
- **WHEN** the unit and e2e selectors are run on a commit without doctests
- **THEN** the sum of their passed tests equals the passed count of `cargo test --workspace --all-features --locked` on that commit (contributor check documented in CONTRIBUTING)

#### Scenario: Doctest added
- **WHEN** a doctest is added to a crate
- **THEN** the unit jobs run it (`--doc`), and the e2e jobs do not

### Requirement: End-to-end stage on main
The system SHALL, on every push to `main` and without a path filter, run `e2e (linux)`, `e2e (macos)` and `e2e (windows)` in parallel without one cancelling another, and report an always-present `e2e-ok` check on that commit that fails when any e2e job failed or was cancelled. A newer push SHALL NOT cancel the run for an older commit.
Evidence: .github/workflows/e2e.yml, design.md D7
Confidence: inferred

#### Scenario: Merge runs e2e
- **WHEN** a pull request is merged into `main`
- **THEN** the e2e jobs run on the merge commit and `e2e-ok` reports their combined result on it

#### Scenario: Docs-only merge
- **WHEN** a merge to `main` changes only documentation
- **THEN** the e2e jobs still run, so the commit has an `e2e-ok` and can be tagged

#### Scenario: Back-to-back merges
- **WHEN** a second merge lands while `e2e.yml` is still running for the first
- **THEN** both commits complete their own e2e run and get their own `e2e-ok`

### Requirement: Relay e2e runs against a self-hosted relay
The system SHALL, in `e2e (linux)` and regardless of the e2e suite result, run `tests/serve_access_tunnel.rs` against a self-hosted `n0computer/iroh-relay` container (tag = the `iroh` version in `Cargo.lock`, HTTPS, job-generated certificate) named by `IROH_TUNNEL_TEST_RELAY_URL`. This suite SHALL NOT use public relays in CI; without the variable it SHALL behave as before.
Evidence: design.md D3, D4; crates/engine/src/endpoint/create.rs:32-35
Confidence: inferred

#### Scenario: Relay suite in CI
- **WHEN** `e2e (linux)` runs
- **THEN** the relay container reports healthy, every `serve_access_tunnel` test runs with `--ignored` against the self-hosted relay and passes, and the container is removed afterwards

#### Scenario: Relay does not start
- **WHEN** the relay container is not healthy within 60 seconds
- **THEN** the job prints the container logs and fails

#### Scenario: Local run without a relay
- **WHEN** a developer runs `cargo test` without `IROH_TUNNEL_TEST_RELAY_URL`
- **THEN** the `serve_access_tunnel` tests are ignored and no network relay is contacted
