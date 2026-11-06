## ADDED Requirements

### Requirement: Manual end-to-end runs on any ref
The system SHALL let a maintainer start `e2e.yml` through `workflow_dispatch`, without inputs, on any ref chosen at dispatch; the run SHALL execute the push run's e2e jobs on that ref's commit, report its aggregate as `e2e-ok (manual)` (same pass/fail rule), never cancel or delay a push run, and keep the push run's read-only permissions. At most one manual run per commit executes at a time.
Evidence: .github/workflows/e2e.yml, design.md D1-D4
Confidence: inferred

#### Scenario: Dispatch on an unmerged branch
- **WHEN** `gh workflow run e2e.yml --ref feature-x` is run for a branch that contains this workflow
- **THEN** `e2e (linux)`, `e2e (macos)`, `e2e (windows)` and `e2e (action in container)` run on the head commit of `feature-x`
- **AND** a check named `e2e-ok (manual)` reports their combined result on that commit
- **AND** no check named `e2e-ok` is created by that run

#### Scenario: Dispatch on main
- **WHEN** `e2e.yml` is dispatched with `--ref main`
- **THEN** the run reports `e2e-ok (manual)` on main's head commit and that commit's `e2e-ok` is neither created nor changed by it

#### Scenario: Manual run fails like a push run
- **WHEN** any e2e job of a manual run fails or is cancelled
- **THEN** `e2e-ok (manual)` fails

#### Scenario: Manual and push run on the same commit
- **WHEN** a manual run is in progress for a commit and that commit is then pushed to `main`
- **THEN** the push run starts without waiting for the manual run, neither run is cancelled, and the commit ends with both `e2e-ok (manual)` and `e2e-ok`

#### Scenario: No extra privileges
- **WHEN** `e2e.yml` is inspected
- **THEN** it declares `permissions: contents: read` and references no `secrets.*` other than the default token, for push and manual runs alike

### Requirement: Manual end-to-end runs are documented
The system SHALL document in `CONTRIBUTING.md` and in the `e2e.yml` header how to run the end-to-end stage before merge (`gh workflow run e2e.yml --ref <branch>`), that the branch must contain the dispatch trigger, and that `e2e-ok (manual)` does not satisfy the release preflight.
Evidence: design.md D5
Confidence: inferred

#### Scenario: Contributor guide
- **WHEN** `CONTRIBUTING.md` is read
- **THEN** it shows the dispatch command, the requirement that the branch contains the trigger, and that only `e2e-ok` from a push run on `main` counts for releases

## MODIFIED Requirements

### Requirement: Three single-purpose pipeline stages
The system SHALL organise CI/CD into exactly four workflows:
- `ci.yml` runs on pull requests (unit test stage);
- `e2e.yml` runs on every push to `main` (end-to-end stage), and may also be started manually as specified by "Manual end-to-end runs on any ref";
- `cache.yml` runs on pushes to `main` that change dependencies or toolchain setup (cache stage);
- `release.yml` runs on `v*` tag pushes (release stage, which ends in deployment).

Each workflow SHALL be started by one kind of event. The only exception is `e2e.yml`'s manual dispatch; no other workflow SHALL be started by manual dispatch. No job SHALL run in more than one stage, and no other workflow SHALL exist in `.github/workflows/`.
Evidence: design.md D1, D7
Confidence: inferred

#### Scenario: Workflow inventory
- **WHEN** `.github/workflows/` is listed
- **THEN** it contains `ci.yml`, `e2e.yml`, `cache.yml` and `release.yml` and nothing else

#### Scenario: Merge to main
- **WHEN** a pull request is merged into `main`
- **THEN** `e2e.yml` runs on the merge commit; `cache.yml` runs only when its paths changed; no build, package or publish job runs

#### Scenario: Only e2e is dispatchable
- **WHEN** the triggers of the four workflows are inspected
- **THEN** only `e2e.yml` declares `workflow_dispatch`

### Requirement: Release preflight requires a passed test gate
The system SHALL, before building a release, require a `vX.Y.Z[-<pre>]` tag on a commit that:
- is reachable from `main`;
- is the merge commit of a pull request whose most recent `ci-ok` check run on its head commit succeeded;
- has its own most recent `e2e-ok` check run succeeded.

It SHALL NOT rerun lint or tests, and a failed preflight SHALL stop the release before any build. The aggregate checks SHALL keep the names `ci-ok` and `e2e-ok`. An `e2e-ok (manual)` check SHALL NOT satisfy the e2e requirement.
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
- **WHEN** the tagged commit's most recent `e2e-ok` failed, was cancelled, or does not exist yet, regardless of any `e2e-ok (manual)`
- **THEN** the release fails at preflight with a message naming `e2e-ok`, and nothing is built or published

#### Scenario: Only a manual e2e run exists
- **WHEN** the tagged commit has a successful `e2e-ok (manual)` but no `e2e-ok`
- **THEN** the release fails at preflight with a message naming `e2e-ok`, and nothing is built or published

#### Scenario: Tag off main
- **WHEN** the tagged commit is not reachable from `main`
- **THEN** the release fails at preflight

#### Scenario: Malformed tag
- **WHEN** a tag such as `v1.2` is pushed
- **THEN** the release fails at preflight

### Requirement: End-to-end stage on main
The system SHALL, on every push to `main` and without a path filter, run `e2e (linux)`, `e2e (macos)` and `e2e (windows)` in parallel without one cancelling another. It SHALL report an always-present `e2e-ok` check on that commit, which fails when any e2e job failed or was cancelled. A newer push SHALL NOT cancel the run for an older commit, and a manual run SHALL NOT delay or cancel a push run.
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
