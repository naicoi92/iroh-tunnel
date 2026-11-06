## Why

The end-to-end suite runs only on pushes to `main`. It covers real tunnels on Linux, macOS and Windows, the Windows Task Scheduler lifecycle, the self-hosted relay and the GitHub Action round trips. A change whose risk lies in OS-specific end-to-end code is therefore proven only after merge, and a red result leaves `main` red until a fix-forward lands. PR #179 is an example: its Windows integration tests could not be compiled locally. A supported way to run the same suite on an unmerged branch closes that gap without weakening the release gate.

## What Changes

**Manual end-to-end runs**
- From: `e2e.yml` starts only on `push` to `main`, and the spec forbids manual dispatch for every workflow.
- To: `e2e.yml` also starts on `workflow_dispatch`, with no inputs. The ref to test is the dispatch ref (`gh workflow run e2e.yml --ref <branch>`). A dispatched run executes the same jobs as a push run, on that ref's commit.
- Reason: lets a maintainer prove Windows, macOS and Linux end-to-end behaviour before merging.
- Impact: non-breaking. Push runs on `main` are unchanged.

**Distinct aggregate for manual runs**
- From: the aggregate check is always `e2e-ok`.
- To: a dispatched run reports `e2e-ok (manual)`, and a push run still reports `e2e-ok`. The release preflight keeps reading only `e2e-ok`.
- Reason: a manual run must never stamp the check that releases trust.
- Impact: none on releases. Branch protection does not reference `e2e-ok`.

**Concurrency**
- Runs are grouped by event and commit, so a manual run and a push run for the same commit never cancel each other. Runs for different commits are still never cancelled.

**Docs**
- The `e2e.yml` header comment and `CONTRIBUTING.md` describe how to run end-to-end tests before merge, and state that `e2e-ok (manual)` does not count for releases.

**Non-goals**
- Running e2e automatically on pull requests.
- Dispatch inputs.
- Changing what the e2e jobs test.
- Making `ci.yml`, `cache.yml` or `release.yml` dispatchable.
- Extra permissions or secrets.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `ci-cd-pipeline`: the stage inventory allows manual dispatch for `e2e.yml` only. A new requirement covers manual end-to-end runs: same jobs, a distinct aggregate name, and no cancellation against push runs. The release preflight requirement gains a scenario stating that it ignores `e2e-ok (manual)`.

## Impact

- **Code:** `.github/workflows/e2e.yml` (trigger, concurrency group, aggregate job name, header comment).
- **Docs:** `CONTRIBUTING.md` (code quality gates and releasing sections).
- **Tests:** no Rust or TypeScript tests change. The workflow is checked by `actionlint`, which runs in `ci.yml`'s `scripts` job and locally. Behaviour is checked by a real dispatch after merge: the dispatch is run on PR #179's branch, and the next push to `main` still reports `e2e-ok`.
- **Dependencies:** none.
- **Compatibility:** non-breaking. Release preflight and branch protection are unchanged.
- **Security:** permissions stay `contents: read`, and no secrets are added. Running a workflow dispatch requires write access to the repository (GitHub default). A dispatched run executes the chosen ref's workflow and code with the same read-only token a push run has.
