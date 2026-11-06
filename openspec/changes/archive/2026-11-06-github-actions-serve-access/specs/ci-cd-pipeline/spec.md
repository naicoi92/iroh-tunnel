## MODIFIED Requirements

### Requirement: Pull-request test gate
The system SHALL run, for every pull request and in parallel, only the checks its changed paths need: `lint` and the unit jobs for code paths, `scripts` for script and workflow paths, and `action` for GitHub Action paths. Pull requests SHALL NOT run end-to-end tests or build release artifacts. Unit job names SHALL identify the operating system (`unit (linux)`, `unit (macos)`, `unit (windows)`), and a failing unit job SHALL NOT cancel the others. A single `ci-ok` check SHALL always report and SHALL fail when any check failed or was cancelled.
Evidence: .github/workflows/ci.yml (changes, lint, unit, scripts, ci-ok), design.md D1; this change's design.md D10
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

#### Scenario: Action change only
- **WHEN** a pull request changes only GitHub Action paths: `action.yml`, `access/**`, `serve/**` or `action-src/**`
- **THEN** only `action` runs: it installs the action dependencies from the lockfile, type-checks, runs the action unit tests, and fails when the committed bundles or the generated root `action.yml` differ from a fresh build; no Rust test is compiled

#### Scenario: Version bump
- **WHEN** a pull request changes the root `Cargo.toml` version
- **THEN** `action` also runs, because the bundles embed that version, so a stale bundle fails `check-dist`

#### Scenario: Mixed change
- **WHEN** a pull request changes both `crates/**` and `action-src/**`, or changes `.github/workflows/ci.yml`
- **THEN** every job whose path filter matches runs, so `lint`, the unit jobs and `action` all run

#### Scenario: Docs-only change
- **WHEN** a pull request changes no code, script, workflow or action path
- **THEN** every check is skipped and `ci-ok` passes

#### Scenario: A check fails
- **WHEN** `lint`, any unit job, `scripts` or `action` fails or is cancelled
- **THEN** `ci-ok` fails

#### Scenario: Superseded push
- **WHEN** a new commit is pushed to a pull request whose run is still in progress
- **THEN** the older run is cancelled

### Requirement: Tests cover the application only
The system SHALL keep only tests of application behaviour in the Rust test suite. The suite SHALL NOT contain tests whose subject is the installer scripts, workflow or Dockerfile contents, the GitHub Actions, or release-artifact sets. Installer scripts and workflows SHALL be checked by static linting only. The GitHub Actions SHALL be tested by their own unit tests under `action-src/` and by running them in the end-to-end stage, never from the Rust suite.
Evidence: design.md D9, D10; this change's design.md D10
Confidence: inferred

#### Scenario: Suite contents
- **WHEN** the integration tests under `tests/` are listed
- **THEN** none of them executes `install.sh` or `install.ps1`, runs the GitHub Actions, or reads files under `.github/`, `packaging/`, `access/`, `serve/` or `action-src/`

#### Scenario: Script lint
- **WHEN** `scripts` runs
- **THEN** shellcheck (severity warning) checks every tracked `*.sh`, PSScriptAnalyzer (Warning and Error) checks `install.ps1`, actionlint checks the workflows, and any finding fails the job

#### Scenario: Action tests
- **WHEN** `action` runs
- **THEN** the tests it runs are the action unit tests under `action-src/`, with the network, file system and processes replaced by fakes

## ADDED Requirements

### Requirement: End-to-end stage runs the GitHub Actions
The system SHALL, in `e2e (linux)`, `e2e (macos)` and `e2e (windows)`, run the actions from the checkout with executables built from the commit and the n0 public relays: a Serve step in front of a local TCP target, an Access step through the root path dialing its `node-id`, and a round trip through the tunnel. A failure of any of these steps or of a post step SHALL fail the e2e job and `e2e-ok`.
Evidence: this change's design.md D10
Confidence: inferred

#### Scenario: Container job on Linux
- **WHEN** `e2e (linux)` runs
- **THEN** it also runs the Serve action in a job running in `container:` with a hostname target, and a round trip through that tunnel succeeds

#### Scenario: Actions on every OS
- **WHEN** `e2e.yml` runs on a merge commit
- **THEN** on each operating system the Serve and root-path Access steps succeed, bytes sent through the Access listener return from the target, and every post step reports the tunnel stopped

#### Scenario: Action regression
- **WHEN** a merge breaks the Access action's readiness on Windows
- **THEN** `e2e (windows)` fails and `e2e-ok` fails, so the commit cannot be released

#### Scenario: No release download in CI
- **WHEN** the actions run in `e2e.yml`
- **THEN** they use the executables built by the job through the CI-only `IROH_TUNNEL_ACTION_BIN_DIR` hook and download no release asset

### Requirement: End-to-end stage covers the Access alias path
The system SHALL, in `e2e (linux)` and `e2e (macos)`, also run an Access step through the `access` path in the same job and complete a round trip through it. In `e2e (windows)` it SHALL instead check that this second Access step fails with the one-step-per-role message.
Evidence: this change's design.md D10
Confidence: inferred

#### Scenario: Alias path on Linux and macOS
- **WHEN** `e2e (linux)` or `e2e (macos)` runs
- **THEN** the `access` path step succeeds and bytes sent through its listener return from the target

#### Scenario: Windows limit asserted
- **WHEN** `e2e (windows)` runs
- **THEN** the second Access step fails with the one-step-per-role message, and the job fails if it succeeds or fails for another reason

### Requirement: Release moves the major action tag
The system SHALL, in the release stage and only after the GitHub release for a stable tag `vX.Y.Z` is published, move the tag `vX` to the same commit when `X.Y.Z` is the highest stable version with major `X`, so callers of every action path at `@vX` receive that release. Pre-release tags and lower versions SHALL NOT move it, and a failed move SHALL fail the release run without changing the published release.
Evidence: this change's design.md D9
Confidence: inferred

#### Scenario: Stable release
- **WHEN** the release for `v0.6.0` is published
- **THEN** `v0` points at the `v0.6.0` commit, and a step using `naicoi92/iroh-tunnel@v0` installs iroh-tunnel 0.6.0

#### Scenario: Pre-release
- **WHEN** the release for `v0.6.1-rc.1` is published
- **THEN** `v0` is not moved

#### Scenario: Release job failed
- **WHEN** the GitHub release job fails
- **THEN** `v0` is not moved

#### Scenario: Older patch released later
- **WHEN** `v0.5.9` is released after `v0.6.0`
- **THEN** `v0` stays on the `v0.6.0` commit

#### Scenario: Tag move fails
- **WHEN** pushing the moved `v0` tag fails
- **THEN** the release run fails, the published release is unchanged, and the run log states how to move the tag by hand

### Requirement: Marketplace listing is a documented manual step
The release checklist in CONTRIBUTING SHALL tell the owner to list each release on the GitHub Marketplace by hand, because no API can do it.
Evidence: this change's design.md D9
Confidence: inferred

#### Scenario: Marketplace step
- **WHEN** a maintainer follows the release checklist in CONTRIBUTING
- **THEN** it tells them to tick "Publish this Action to the GitHub Marketplace" on the release, and to accept the Marketplace agreement on the first release
