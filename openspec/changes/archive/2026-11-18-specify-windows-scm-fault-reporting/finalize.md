# Finalize Receipt

**Change**: `specify-windows-scm-fault-reporting`
**Finalized at**: `2026-11-17T23:48Z`
**Outcome**: `pr-created`

> Deviation from the git-side closeout: apply ran on the feature branch itself (user-approved), so there was no worktree branch to merge and steps 3–7 did not apply. Push and PR creation ran only after the user's explicit permission in chat, as the project `AGENTS.md` requires. The push used HTTPS with the `gh` credential helper because the default SSH key belongs to another account (AGENTS.md rule).

---

## Branch state

- **Branch**: `fix/windows-scm-effects`
- **Base branch**: `main`
- **Final state**: `pr-open`
- **PR URL**: `https://github.com/naicoi92/iroh-tunnel/pull/255`

---

## Workspace

- **Worktree**: `/Users/naicoi/Projects/iroh/iroh-tunnel.fix-windows-scm-effects` (Worktrunk-managed)
- **Cleanup**: `preserved (harness-owned)` — removed later with `wt remove` after merge, per AGENTS.md

---

## PR comment

- **PR comment status**: `posted` (https://github.com/naicoi92/iroh-tunnel/pull/255#issuecomment-6964541678)

---

## Code review

- **Code review**: `approved`
- **Reviewer tier**: human, via `plannotator review --gate` (plannotator-cli), decision `approved`, message "Code review completed — no changes requested."
- **Commit range reviewed**: `5809ac7..0ec4bf4`

---

## Tests

- **Baseline status at finish**: `passing` (`cargo test --all-features --locked` in the worktree: exit 0, 1068 passed, 0 failed)

---

## Next step

Wait for PR review. Resolve the `CHANGELOG.md` conflict with `main` (#248 added an entry in the same section; keep both). Confirm the Windows CI job `service_system_windows` passes (verify.md WARNING). After approval, run `/opsx-archive` on this feature branch, push the archive commits to update the PR, then merge the PR.
