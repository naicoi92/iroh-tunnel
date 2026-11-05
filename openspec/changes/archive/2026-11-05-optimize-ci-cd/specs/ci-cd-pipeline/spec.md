## ADDED Requirements

### Requirement: Three single-purpose pipeline stages
The system SHALL organise CI/CD into exactly three workflows, each started by one kind of event and none by manual dispatch: `ci.yml` on pull requests (test stage), `cache.yml` on pushes to `main` (cache stage) and `release.yml` on `v*` tag pushes (release stage, which ends in deployment). No job SHALL run in more than one stage, and no other workflow SHALL exist in `.github/workflows/`.
Evidence: design.md D1
Confidence: inferred

#### Scenario: Workflow inventory
- **WHEN** `.github/workflows/` is listed
- **THEN** it contains `ci.yml`, `cache.yml` and `release.yml` and nothing else

#### Scenario: Merge to main
- **WHEN** a pull request is merged into `main`
- **THEN** no test, build, package or publish job runs; only `cache.yml` may run, and only when its paths changed

### Requirement: Pull-request test gate
The system SHALL run, for every pull request and in parallel, only the checks its changed paths need: `lint`, `test-linux` and `test-windows` for code paths, and `scripts` for script and workflow paths. A single `ci-ok` check SHALL always report and SHALL fail when any check failed or was cancelled. The pull-request stage SHALL NOT build release artifacts.
Evidence: design.md D1, D9
Confidence: inferred

#### Scenario: Code change
- **WHEN** a pull request changes a code path: `src/**`, `crates/**`, `tests/**`, `examples/**`, `Cargo.toml`, `Cargo.lock`, `build.rs`, `.github/actions/**` or `.github/workflows/ci.yml`
- **THEN** `lint` (rustfmt check, clippy with warnings denied), `test-linux` and `test-windows` (workspace suite, all features) run in parallel and `ci-ok` reflects their results

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
- **WHEN** any of `lint`, `test-linux`, `test-windows` or `scripts` fails or is cancelled
- **THEN** `ci-ok` fails

#### Scenario: Superseded push
- **WHEN** a new commit is pushed to a pull request whose run is still in progress
- **THEN** the older run is cancelled

### Requirement: Tests cover the application only
The system SHALL keep only tests of application behaviour in the Rust test suite. The suite SHALL NOT contain tests whose subject is the installer scripts, workflow or Dockerfile contents, or release-artifact sets. Installer scripts and workflows SHALL be checked by static linting only.
Evidence: design.md D9, D10
Confidence: inferred

#### Scenario: Suite contents
- **WHEN** the integration tests under `tests/` are listed
- **THEN** none of them executes `install.sh` or `install.ps1` or reads files under `.github/` or `packaging/`

#### Scenario: Script lint
- **WHEN** `scripts` runs
- **THEN** shellcheck (severity warning) checks every tracked `*.sh`, PSScriptAnalyzer (Warning and Error) checks `install.ps1`, actionlint checks the workflows, and any finding fails the job

### Requirement: Windows test job includes the user-service lifecycle
The system SHALL, in `test-windows`, run the `service_user_windows` tests with `IROH_TUNNEL_E2E_SERVICE=1` after the workspace suite in the same job, reusing its build, so the tests gated on that variable drive the real Task Scheduler. The opt-in variable SHALL NOT be set for the Linux test job.
Evidence: tests/service_user_windows.rs:1-20, tests/service_user.rs:336, design.md D2
Confidence: inferred

#### Scenario: Lifecycle runs on Windows
- **WHEN** `test-windows` runs
- **THEN** the gated tests (including `user_background_task_lifecycle`: install, start, status, stop, restart and uninstall of a uniquely named task) run against the real Task Scheduler, and the job fails if any of them fails

#### Scenario: Linux unaffected
- **WHEN** `test-linux` runs
- **THEN** `IROH_TUNNEL_E2E_SERVICE` is not set, so `tests/service_user.rs` does not drive the host service manager

### Requirement: Cache stage compiles without running tests
The system SHALL, on a push to `main` that changes a Cargo manifest, `Cargo.lock`, the `setup-rust` action or `cache.yml`, compile the lint and test targets with the same flags and cache keys as the pull-request jobs and save the caches, without running any test.
Evidence: .github/actions/setup-rust/action.yml, design.md D3
Confidence: inferred

#### Scenario: Dependency bump merged
- **WHEN** a change to `Cargo.lock` is pushed to `main`
- **THEN** clippy and `cargo test --no-run` compile on ubuntu and windows and the caches are saved

#### Scenario: Source-only change merged
- **WHEN** a push to `main` changes only files under `src/`
- **THEN** `cache.yml` does not run

### Requirement: Release preflight requires a passed test gate
The system SHALL, before building a release, require a `vX.Y.Z[-<pre>]` tag on a commit reachable from `main` that is the merge commit of a pull request whose most recent `ci-ok` check run on its head commit succeeded. It SHALL NOT rerun lint or tests, and a failed preflight SHALL stop the release before any build. The aggregate check SHALL keep the name `ci-ok`.
Evidence: design.md D4
Confidence: inferred

#### Scenario: Commit passed CI
- **WHEN** `v1.2.3` is pushed on a merge commit whose pull request head has a successful `ci-ok`
- **THEN** preflight passes and outputs version `1.2.3` and pre-release `false`

#### Scenario: Pre-release tag
- **WHEN** `v1.2.3-rc.1` passes preflight
- **THEN** pre-release is `true`

#### Scenario: Commit without a green gate
- **WHEN** the tagged commit is not the merge commit of a pull request, or that pull request's most recent `ci-ok` did not succeed
- **THEN** the release fails at preflight and nothing is built or published

#### Scenario: Tag off main
- **WHEN** the tagged commit is not reachable from `main`
- **THEN** the release fails at preflight

#### Scenario: Malformed tag
- **WHEN** a tag such as `v1.2` is pushed
- **THEN** the release fails at preflight

### Requirement: Release stage publishes automatically
The system SHALL, after preflight and without any manual step, build all release targets in parallel, package, sign and verify the signatures, and then publish the GitHub release, the role images and (stable versions only) the Homebrew cask. No publish job SHALL start before signature verification succeeds, the cask SHALL be updated only after the GitHub release exists, and an existing release for the tag SHALL NOT be modified.
Evidence: design.md D5-D8
Confidence: inferred

#### Scenario: Stable tag
- **WHEN** `v1.2.3` passes preflight and every build, package and sign job succeeds
- **THEN** a published (non-draft) GitHub release `v1.2.3` holds every archive, package, checksum file and signature bundle, images `v1.2.3`, `1.2.3`, `v1.2`, `v1` and `latest` are pushed for each role, and the tap's `Casks/iroh-tunnel.rb` points at version `1.2.3`

#### Scenario: Pre-release tag
- **WHEN** `v1.2.3-rc.1` is released
- **THEN** the GitHub release is marked pre-release, images get no `latest` tag, and the Homebrew cask is not changed

#### Scenario: Signature verification fails
- **WHEN** a signature bundle does not verify against the pinned issuer and identity
- **THEN** no release, image or cask is published

#### Scenario: Build target fails
- **WHEN** any release target fails to build
- **THEN** the remaining targets are cancelled and nothing is published

#### Scenario: Release already exists
- **WHEN** a release for the tag already exists
- **THEN** the GitHub release job fails without changing it

#### Scenario: Signing identity unchanged
- **WHEN** `release.yml` runs for tag `<tag>`
- **THEN** signing and signature verification are jobs of `release.yml` itself (not of a called workflow), only the signing job holds `id-token: write`, and the certificate identity is `https://github.com/naicoi92/iroh-tunnel/.github/workflows/release.yml@refs/tags/<tag>`

#### Scenario: Concurrent runs
- **WHEN** a second run for the same tag starts while one is in progress
- **THEN** it queues behind the in-progress run, which is not cancelled
