## MODIFIED Requirements

### Requirement: Pull-request test gate
The system SHALL run, for every pull request and in parallel, only the checks its changed paths need: `lint` and the unit jobs for code paths, `scripts` for script and workflow paths, and `action` for GitHub Action paths. Pull requests SHALL NOT run end-to-end tests or build release artifacts. Unit job names SHALL identify the operating system (`unit (linux)`, `unit (macos)`, `unit (windows)`), and a failing unit job SHALL NOT cancel the others. A single `ci-ok` check SHALL always report and SHALL fail when any check failed or was cancelled.
`lint` SHALL run the rustfmt check, clippy with warnings denied, and `cargo doc --workspace --no-deps --locked` with `RUSTDOCFLAGS=-D warnings`.
Evidence: .github/workflows/ci.yml (changes, lint, unit, scripts, ci-ok), design.md D1; optimize-ci-cd design.md D10; cleanup-config-dto-rustdoc design.md D5
Confidence: inferred

#### Scenario: Code change
- **WHEN** a pull request changes a code path: `src/**`, `crates/**`, `tests/**`, `examples/**`, `Cargo.toml`, `Cargo.lock`, `build.rs`, `.github/actions/**` or `.github/workflows/ci.yml`
- **THEN** `lint` (rustfmt check, clippy with warnings denied, workspace rustdoc build with warnings denied) and the unit jobs on Linux, macOS and Windows run in parallel, no e2e test runs, and `ci-ok` reflects their results

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

#### Scenario: Broken rustdoc link
- **WHEN** a pull request changes a code path and introduces an intra-doc link that does not resolve on an item documented for the Linux `lint` target
- **THEN** `lint` fails and `ci-ok` fails
