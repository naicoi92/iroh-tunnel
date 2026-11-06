# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `e2e-manual-dispatch`
**Iteration**: `1`
**Applied at**: `2026-11-06 20:49`
**Executor**: `subagent-driven-development`. Implementer `superspec.implementer`; review `superspec.task-reviewer`. Tasks 1 and 2 ran as a single batched dispatch because they are small, touch the same file, and run in sequence. Each task has its own commit.

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.e2e-manual-dispatch`
- **Worktree branch**: `e2e-manual-dispatch`
- **Feature branch**: `e2e-manual-dispatch`. Worktrunk created the worktree before planning, so there is no separate feature branch.
- **Base branch**: `main` (at `360386b` when the branch was created)

---

## Commits

- **Range**: `5064679..b96bc5c`. `5064679` is the commit that scaffolds the artifacts. The implementation commits are:
  - `f899c82` ci(e2e): allow manual dispatch with a separate e2e-ok (manual) check
  - `b96bc5c` docs: document manual e2e runs before merge
- **Count**: `2` implementation commits, plus this receipt.

---

## Tasks

- **Completed**: `2 of 3` checkboxes in tasks.md.
- **Remaining**: `3.1` is the post-merge verification. By plan it stays open through apply and verify with the reason "post-merge". The parent runs it after the PR merges, ticks it with the run URLs as evidence, and then archives.

---

## Verification evidence

- `actionlint .github/workflows/*.yml` exits 0. This was run before the change and again after Task 1 and Task 2, and checked by the controller.
- `rg -l workflow_dispatch .github/workflows` lists only `.github/workflows/e2e.yml`.
- `rg -n "secrets\." .github/workflows/e2e.yml` returns no matches, and `permissions: contents: read` is still present.
- `git diff -U0 5064679..f899c82 -- .github/workflows/e2e.yml` shows hunks only for `on:`, `concurrency.group` and the `e2e-ok` `name:` line. Nothing changed inside any `steps:`.
- `rg -n "gh workflow run e2e.yml --ref"` finds 2 hits, in the `e2e.yml` header and in `CONTRIBUTING.md`. `rg -n "e2e-ok \(manual\)" CONTRIBUTING.md` finds 2 hits.
- There are no Rust or TypeScript changes, so the project test suites are unaffected. `ci.yml`'s `scripts` job runs actionlint on the PR.

---

## Review summary

- **Combined task and whole-branch review** of `5064679..b96bc5c`: **Approved**. No Critical or Important issues. Minor items, deferred:
  - In `CONTRIBUTING.md`, the new sentence about manual e2e runs sits awkwardly in its paragraph and has a long line.
  - The check-run name produced by a job `name:` expression can only be verified on a real run. Task 3.1 covers this through the GitHub API, after merge.

---

## Contract deviations

- none

---

## Next step

Run `/opsx-verify e2e-manual-dispatch`.
