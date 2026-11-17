## MODIFIED Requirements

### Requirement: Pull-request test gate
The system SHALL run, for every pull request and in parallel, only the checks its changed paths need: the lint jobs and the unit jobs for code paths, `scripts` for script and workflow paths, and `action` for GitHub Action paths. Pull requests SHALL NOT run end-to-end tests or build release artifacts. Lint and unit job names SHALL identify the operating system (`lint (linux)`, `lint (macos)`, `lint (windows)`, `unit (linux)`, `unit (macos)`, `unit (windows)`), and a failing lint or unit job SHALL NOT cancel the others. A single `ci-ok` check SHALL always report and SHALL fail when any check failed or was cancelled.
Each lint job SHALL run `cargo clippy --workspace --all-targets --all-features --locked` with warnings denied on its operating system, using the toolchain pinned by the `setup-rust` action (the `Cargo.toml` `rust-version`). `lint (linux)` SHALL also run the rustfmt check and `cargo doc --workspace --no-deps --locked` with `RUSTDOCFLAGS=-D warnings`; the other lint jobs SHALL NOT repeat them.
Evidence: .github/workflows/ci.yml (changes, lint, unit, scripts, ci-ok), design.md D1; optimize-ci-cd design.md D10; cleanup-config-dto-rustdoc design.md D5; ci-cross-platform-clippy design.md D1-D3
Confidence: inferred

#### Scenario: Code change
- **WHEN** a pull request changes a code path: `src/**`, `crates/**`, `tests/**`, `examples/**`, `Cargo.toml`, `Cargo.lock`, `build.rs`, `.github/actions/**` or `.github/workflows/ci.yml`
- **THEN** the lint jobs (clippy with warnings denied on Linux, macOS and Windows; the rustfmt check and workspace rustdoc build with warnings denied on Linux) and the unit jobs on Linux, macOS and Windows run in parallel, no e2e test runs, and `ci-ok` reflects their results

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
- **THEN** every job whose path filter matches runs, so the lint jobs, the unit jobs and `action` all run

#### Scenario: Docs-only change
- **WHEN** a pull request changes no code, script, workflow or action path
- **THEN** every check is skipped and `ci-ok` passes

#### Scenario: A check fails
- **WHEN** any lint job, any unit job, `scripts` or `action` fails or is cancelled
- **THEN** `ci-ok` fails

#### Scenario: Superseded push
- **WHEN** a new commit is pushed to a pull request whose run is still in progress
- **THEN** the older run is cancelled

#### Scenario: Broken rustdoc link
- **WHEN** a pull request changes a code path and introduces an intra-doc link that does not resolve on an item documented for the Linux lint target
- **THEN** `lint (linux)` fails and `ci-ok` fails

#### Scenario: Platform-only lint finding
- **WHEN** a pull request adds a clippy finding in code compiled only on one operating system (for example under `#[cfg(windows)]` or `#[cfg(target_os = "macos")]`)
- **THEN** the lint job of that operating system fails, the other lint jobs keep running to completion, and `ci-ok` fails

### Requirement: Cache stage compiles without running tests
The system SHALL, on a push to `main` that changes a Cargo manifest, `Cargo.lock`, the `setup-rust` action or `cache.yml`, compile the lint target and the unit targets on Linux, macOS and Windows, with the same flags and cache keys as the pull-request jobs, and save the caches, without running any test.
Evidence: .github/actions/setup-rust/action.yml, .github/workflows/cache.yml, design.md D5; ci-cross-platform-clippy design.md D4
Confidence: inferred

#### Scenario: Dependency bump merged
- **WHEN** a change to `Cargo.lock` is pushed to `main`
- **THEN** on each of Linux, macOS and Windows, clippy compiles the lint target and the unit targets compile with `--no-run`, and the caches are saved

#### Scenario: Source-only change merged
- **WHEN** a push to `main` changes only files under `src/`
- **THEN** `cache.yml` does not run
