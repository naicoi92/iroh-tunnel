# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `optimize-ci-cd`
**Iteration**: `1`
**Applied at**: `2026-11-05 15:47`
**Executor**: `subagent-driven-development` (implementer: `superspec.implementer`; per-task and whole-branch review: `superspec.task-reviewer`)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.optimize-ci-cd`
- **Worktree branch**: `optimize-ci-cd`
- **Feature branch**: `spec/optimize-ci-cd`
- **Base branch**: `main`

Step 0 note: the session started on `main` with the change directory untracked. To keep artifacts off `main`, the feature branch `spec/optimize-ci-cd` was created from `main` (worktree `/Users/naicoi/Projects/iroh/iroh-tunnel.spec-optimize-ci-cd`), the change directory was moved there and committed (`2332a0a`), and the worktree branch was created from it.

---

## Commits

- **Range**: `2332a0a..88220e1` (implementation and task ticks; this receipt is committed on top)
- **Count**: `17`

---

## Tasks

- **Completed**: `24 of 24` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: none

Verification at `88220e1` (controller, fresh): actionlint clean; shellcheck clean; PSScriptAnalyzer `-EnableExit` rc 0; `cargo fmt --check` clean; clippy `-D warnings` clean; `cargo test --workspace --all-features --locked`: 498 passed, 0 failed, 4 ignored, 214 s locally; `openspec validate --strict` valid; `.github/workflows` = `cache.yml ci.yml release.yml`; no `workflow_dispatch`; `id-token: write` count 1.

Not verified yet (needs owner permission): plan Task 8 step 5 (the PR run of the new `ci.yml`, incl. the Windows Task Scheduler step actually running) and step 6 (first tag after merge must be `vX.Y.Z-rc.N` and exercises `release.yml` end to end).

Reviews: every task reviewed by `superspec.task-reviewer`; Task 3 and Task 6 needed one fix round each (stale header; image exec bit + post-push smoke, summary step, recovery comment). Final whole-branch review: no Critical/Important; one doc/comment fix wave (`88220e1`) verified by the controller.

Rulings (controller): Task 8 steps 5-6 deferred (need push/tag) — cost if wrong: none; re-reviews of the two comment/workflow fix rounds done by the controller reading the diff — cost if wrong: one extra fix commit.

Deferred minors (can wait): install.sh-only PR runs only `scripts` (by design); redundant `git fetch origin main` in preflight; confirm on first `-rc` run that semver major/minor image tags are not pushed for pre-releases and that `cosign sign-blob --new-bundle-format` still works on cosign v3.1.4; image smoke runs after push; Homebrew token in clone URL; duplicate "release" badge labels.

---

## Contract deviations

- none

---

## Next step

Run `/opsx-verify`.
