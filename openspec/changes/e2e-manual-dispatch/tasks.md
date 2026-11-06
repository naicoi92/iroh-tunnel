<!--
File ownership: group 1 owns .github/workflows/e2e.yml (trigger, concurrency, job name); group 2 owns CONTRIBUTING.md and the e2e.yml header comment (after group 1).
Sequential: 2 depends on 1; 3 depends on 1-2 and on the change being merged to main.
Platforms: workflow logic is OS-independent; the matrix (linux/macos/windows) is unchanged.
Traceability: CICD = ci-cd-pipeline delta spec.
-->

## 1. Workflow (.github/workflows/e2e.yml)

- [x] 1.1 Add `workflow_dispatch:` (no inputs) to `on:`, change the concurrency group to `e2e-${{ github.event_name }}-${{ github.sha }}` (keep `cancel-in-progress: false`), and make the aggregate job's display name `${{ github.event_name == 'workflow_dispatch' && 'e2e-ok (manual)' || 'e2e-ok' }}` (job id stays `e2e-ok`). Permissions stay `contents: read`; no `secrets.*`. Verify with `actionlint .github/workflows/*.yml` and by grepping that only `e2e.yml` declares `workflow_dispatch`. (CICD: Manual end-to-end runs on any ref — Dispatch on an unmerged branch, Dispatch on main, Manual run fails like a push run, Manual and push run on the same commit, No extra privileges; Three single-purpose pipeline stages — Only e2e is dispatchable, Workflow inventory; End-to-end stage on main)

## 2. Documentation

- [x] 2.1 Update the `e2e.yml` header comment and `CONTRIBUTING.md` (code quality gates and releasing): the dispatch command `gh workflow run e2e.yml --ref <branch>`, the branch must contain the trigger (rebase on `main`), and only `e2e-ok` from a push run on `main` counts for releases; `e2e-ok (manual)` does not. (CICD: Manual end-to-end runs are documented — Contributor guide; Release preflight requires a passed test gate — Only a manual e2e run exists)

## 3. Post-merge verification

- [ ] 3.1 (post-merge; stays open through apply/verify with reason "post-merge") After merge: check that the push run on the merge commit reports a check named `e2e-ok`; dispatch on `main` and check `e2e-ok` is unchanged; then dispatch on a branch rebased onto that commit (`gh workflow run e2e.yml --ref remake-config-cli`) and check via `gh api repos/naicoi92/iroh-tunnel/commits/<sha>/check-runs` that it reports `e2e-ok (manual)` and creates no `e2e-ok`. Record the run URLs. (CICD: Dispatch on an unmerged branch; End-to-end stage on main — Merge runs e2e; Release preflight — Only a manual e2e run exists by exact-name filter in release.yml)
