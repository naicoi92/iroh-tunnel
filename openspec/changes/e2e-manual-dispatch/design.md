## Context

`.github/workflows/e2e.yml` currently works like this:
- It starts on `push` to `main`.
- It runs the `e2e (linux|macos|windows)` matrix and the `e2e (action in container)` job.
- It reports an `e2e-ok` aggregate job (`if: always()`, fails on any `failure` or `cancelled` upstream result).
- Its concurrency group is `e2e-${{ github.sha }}` with `cancel-in-progress: false`.
- Its permissions are `contents: read`.

`release.yml` preflight queries `commits/$GITHUB_SHA/check-runs?check_name=e2e-ok` for the tagged commit. It requires the latest such check run to be `success`, and requires that commit to be the merge commit of a PR with a green `ci-ok`.

The `main` branch has no branch protection or rulesets (checked with the GitHub API; both return empty or 404), so no required-check name has to be kept in sync.

The `ci-cd-pipeline` spec states that no workflow is started by manual dispatch.

The bounded context is CI/CD only. No application code, crate or test changes.

## Goals / Non-Goals

**Goals:**
- Dispatch `e2e.yml` on any ref, with no inputs, running the same jobs.
- Name the aggregate `e2e-ok (manual)` for dispatched runs.
- Keep push-run behaviour byte-for-byte except for the trigger, concurrency key and job name expressions.

**Non-Goals:**
- Running on PRs.
- Inputs or an OS subset.
- New checks or secrets.
- Any change to `release.yml`, `ci.yml` or `cache.yml`.

## Decisions

### D1. Trigger
- `on:` gains `workflow_dispatch:` with no `inputs`.
- **Why:** the dispatch ref already selects the branch. `actions/checkout` checks out `github.sha`, which for a dispatch is the head of the selected ref, so every job tests that commit without other changes.

### D2. Aggregate name depends on the event
- The `e2e-ok` job keeps the job id `e2e-ok`, and its display name becomes `${{ github.event_name == 'workflow_dispatch' && 'e2e-ok (manual)' || 'e2e-ok' }}`.
- A check run's name is the job's display name, so a push run produces `e2e-ok` and a dispatch produces `e2e-ok (manual)`. Release preflight filters by `check_name=e2e-ok`, which is an exact match, so it never sees the manual check.
- **Alternatives:**
  - A separate `e2e-ok-manual` job guarded by `if:` would leave a `skipped` check named `e2e-ok` on dispatched runs. A skipped check counts as neither `success` nor missing for the preflight, so it would be safe but confusing.
  - A separate workflow file is forbidden by the four-workflow inventory.

### D3. Concurrency per event
- The group becomes `e2e-${{ github.event_name }}-${{ github.sha }}`, still with `cancel-in-progress: false`.
- **Why:** a manual run on a commit that is later pushed to `main` (fast-forward), or a dispatch on `main` itself, must not queue behind or block the push run for the same SHA. Runs for different commits are still never cancelled.

### D4. Permissions and secrets unchanged
- `permissions: contents: read` is unchanged, and the jobs read no repository secrets.
- **Why:** a dispatch can run any ref's version of the workflow. Keeping the token read-only bounds what such a run can do. Dispatch itself already requires write access.

### D5. Documentation
- The `e2e.yml` header comment and `CONTRIBUTING.md` (code quality gates and releasing) gain the dispatch command and state that `e2e-ok (manual)` does not count for releases.

## Risks / Trade-offs

- **[A dispatched ref carries an older `e2e.yml` without the dispatch trigger]** → GitHub refuses the dispatch ("Workflow does not have 'workflow_dispatch' trigger"). The branch must contain this change, for example by being rebased on `main` after it merges. This is documented in `CONTRIBUTING.md`.
- **[Someone tags a commit whose only green e2e is manual]** → Release preflight reports `e2e-ok` missing and stops. This is the intended behaviour, now covered by a spec scenario.
- **[Expression in a job `name:` is not evaluated as expected]** → `actionlint` validates the expression context in CI. The first real dispatch is verified to produce a check run named `e2e-ok (manual)`, and the next push run to produce `e2e-ok`.
- **[Cost of a full matrix run on demand]** → It is manual and opt-in only, with no automatic trigger.

## Migration Plan

- Merge to `main`, then dispatch once on a branch rebased onto the merge (PR #179) to confirm the check name, and confirm the following push run on `main` still reports `e2e-ok`.
- Rollback: revert the commit. No state is kept.

## Open Questions

- None.
