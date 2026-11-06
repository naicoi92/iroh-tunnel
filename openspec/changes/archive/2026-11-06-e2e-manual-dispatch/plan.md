# e2e Manual Dispatch Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** Let `e2e.yml` be started manually on any ref, reporting `e2e-ok (manual)` so that the release gate keeps trusting only push runs on `main`.

**Architecture:** This is a workflow-only change. It adds a `workflow_dispatch` trigger to `e2e.yml`, makes the concurrency group and the aggregate job's display name depend on `github.event_name`, and updates the docs. No application code changes.

**Tech Stack:** GitHub Actions YAML, actionlint, gh CLI.

## Global Constraints

- The workflows are exactly `ci.yml`, `e2e.yml`, `cache.yml` and `release.yml`. Only `e2e.yml` declares `workflow_dispatch`.
- `e2e.yml` keeps `permissions: contents: read` and references no `secrets.*`.
- The aggregate job id stays `e2e-ok`. A push run's check name stays exactly `e2e-ok`, and a dispatch run's check name is exactly `e2e-ok (manual)`.
- `cancel-in-progress: false` is kept.
- `release.yml` is unchanged.
- Docs are in English (the repository is public).

## Review Focus

1. **Expression in a job `name:`.** It must evaluate to a plain string in both events. actionlint checks the syntax only, so the post-merge task 3.1 checks the real names through the API.
2. **The concurrency key change on `main`.** Each push run on a new SHA still gets its own group (`e2e-push-<sha>`), so back-to-back merges still do not wait on each other.
3. **A dispatch on a ref without the trigger.** GitHub rejects it, and `CONTRIBUTING.md` says to rebase first.
4. **Static check that only `e2e.yml` is dispatchable.** Task 1, step 3 covers it.
5. **No other job or step changes behavior.** The diff of `e2e.yml` must touch only `on:`, `concurrency.group`, the `e2e-ok` `name:` and the header comment.

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `e2e.yml` `on:` | modified | `on: { push: { branches: [main] }, workflow_dispatch: {} }`, with no inputs | public (CI interface) |
| `e2e.yml` `concurrency` | modified | `group: e2e-${{ github.event_name }}-${{ github.sha }}`, `cancel-in-progress: false` | internal |
| `e2e.yml` job `e2e-ok` display name | modified | `name: ${{ github.event_name == 'workflow_dispatch' && 'e2e-ok (manual)' || 'e2e-ok' }}`; job id `e2e-ok` unchanged | public (check-run name read by `release.yml` and humans) |
| Check run `e2e-ok (manual)` | new | Check-run name created by dispatched runs, with the same pass/fail rule as `e2e-ok` | public |
| `CONTRIBUTING.md` e2e/release text | modified | Documents `gh workflow run e2e.yml --ref <branch>`, the rebase requirement, and that `e2e-ok (manual)` does not count for releases | public (docs) |

---

## Task 1: Dispatchable e2e workflow — implements 1.1

**Files:**
- Modify: `.github/workflows/e2e.yml` (`on:` at about lines 17-19, `concurrency` at about lines 21-23, `e2e-ok` `name:` at about line 241)

- [ ] **Step 1:** Confirm the baseline. `actionlint .github/workflows/*.yml` should exit 0. `rg -l "workflow_dispatch" .github/workflows` should print nothing.
- [ ] **Step 2:** Apply the three edits from Contracts exactly. Touch nothing else in the jobs.
- [ ] **Step 3:** Verify:
  - `actionlint .github/workflows/*.yml` exits 0.
  - `rg -l "workflow_dispatch" .github/workflows` prints only `.github/workflows/e2e.yml`.
  - `rg -n "secrets\." .github/workflows/e2e.yml` prints nothing.
  - `rg -n "contents: read" .github/workflows/e2e.yml` matches.
  - `rg -nF 'group: e2e-${{ github.event_name }}-${{ github.sha }}' .github/workflows/e2e.yml` matches.
  - `rg -nF "'e2e-ok (manual)' || 'e2e-ok'" .github/workflows/e2e.yml` matches the `e2e-ok` job's `name:` line.
  - `git diff -U0 -- .github/workflows/e2e.yml` touches only `on:`, `concurrency.group` and the `e2e-ok` `name:` line. No hunk falls inside any `steps:`.
  - `git diff --stat` shows only `e2e.yml`.
- [ ] **Step 4:** Commit `ci(e2e): allow manual dispatch with a separate e2e-ok (manual) check`.

## Task 2: Documentation — implements 2.1

**Files:**
- Modify: `.github/workflows/e2e.yml` (header comment, lines 1-13), `CONTRIBUTING.md` ("Code quality gates" paragraph at about lines 52-54, "Releasing" at about lines 163-168)

- [ ] **Step 1:** Header comment. Add this text:

  ```
  # Before merge, run it on a branch with `gh workflow run e2e.yml --ref <branch>`
  # (the branch must contain this trigger). That run reports `e2e-ok (manual)`,
  # which release preflight ignores.
  ```

- [ ] **Step 2:** `CONTRIBUTING.md`:
  - In "Code quality gates", add a sentence with the dispatch command, the rebase-on-`main` requirement, and that the check is `e2e-ok (manual)`.
  - In "Releasing", say that only `e2e-ok` from the push run on `main` counts, and that `e2e-ok (manual)` does not.
- [ ] **Step 3:** Verify:
  - `rg -n "gh workflow run e2e.yml --ref" CONTRIBUTING.md .github/workflows/e2e.yml` gets 2 hits.
  - `rg -n "e2e-ok \(manual\)" CONTRIBUTING.md` gets at least 1 hit.
  - `actionlint` still exits 0.
- [ ] **Step 4:** Commit `docs: document manual e2e runs before merge`.

## Task 3: Post-merge verification — implements 3.1

Runs only after this change is merged to `main`.
- Apply completes Tasks 1 and 2 and leaves 3.1 unticked, with the recorded reason "post-merge".
- Verify records it as an open task with that reason, and it does not block.
- After the PR merges, the parent runs Task 3, ticks 3.1 with the run URLs as evidence, and then archives.
- Every push, force-push or dispatch in this task needs the user's explicit approval at that time.

- [ ] **Step 1:** Push run check. After the merge commit `M` appears on `main`, wait for the e2e run, then run `gh api repos/naicoi92/iroh-tunnel/commits/M/check-runs --jq '.check_runs[].name'`. It must include `e2e-ok` and must not include `e2e-ok (manual)`.
- [ ] **Step 2:** Run `gh workflow run e2e.yml --ref main`. Wait for it with `gh run watch <id> --exit-status`. Then `gh api .../commits/M/check-runs` must also list `e2e-ok (manual)`, and the `e2e-ok` check run must be unchanged: same id and conclusion as in step 1.
- [ ] **Step 3:** With the user's approval, rebase `remake-config-cli` onto `main` and force-push with `--force-with-lease`. Then run `gh workflow run e2e.yml --ref remake-config-cli` and wait for completion. `gh api repos/naicoi92/iroh-tunnel/commits/<rebased head SHA>/check-runs --jq '.check_runs[].name'` must include `e2e-ok (manual)` and must not include `e2e-ok`. Record the run URLs and conclusions.

---

## Scenario → test

**ci-cd-pipeline**

| Scenario | Verified by |
|---|---|
| Dispatch on an unmerged branch | Task 3, steps 2-3 (API check-run names) |
| Dispatch on main | Task 3, step 2 (API shows `e2e-ok (manual)` added and `e2e-ok` unchanged) |
| Manual run fails like a push run | Task 1, step 3 (`git diff -U0` shows no hunk inside the `e2e-ok` steps) |
| Manual and push run on the same commit | Task 1, step 3 (exact concurrency group with `github.event_name`, found with `rg`) |
| No extra privileges | Task 1, step 3 (`contents: read`, no `secrets.`) |
| Contributor guide | Task 2, step 3 |
| Workflow inventory | Task 1, step 3 (no new files: `git diff --stat`) |
| Merge to main | Task 3, step 1 |
| Only e2e is dispatchable | Task 1, step 3 (`rg -l workflow_dispatch`) |
| Commit passed CI; Pre-release tag; Commit without a green gate; E2E red or missing on the tagged commit; Tag off main; Malformed tag | Unchanged `release.yml` (diff shows no change to `release.yml`) |
| Only a manual e2e run exists | Static: `release.yml` filters `check_name=e2e-ok` by exact match, and Task 3, steps 2-3 show that manual runs add no `e2e-ok` |
| Merge runs e2e; Docs-only merge; Back-to-back merges | Task 3, step 1, plus static review of the per-event-and-SHA concurrency group |
