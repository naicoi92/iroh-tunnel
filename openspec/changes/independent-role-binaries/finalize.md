# Finalize Receipt

**Change**: `independent-role-binaries`
**Finalized at**: `2026-11-16 14:41 UTC`
**Outcome**: `pr-updated`

## Branch state

- **Branch**: `feat/independent-role-binaries`
- **Base branch**: `main`
- **Final state**: `pr-updated` (feature branch publication; no PR created)
- **PR URL**: N/A

## Workspace

- **Worktree**: `/root/projects/iroh-tunnel.independent-role-binaries`
- **Cleanup**: `preserved (harness-owned)`
- **Worktree branch**: retained because the preserved worktree uses it.

## PR comment

- **PR comment status**: `skipped (no PR)`

## Code review

- **Code review**: `approved`
- **Reviewer tier**: human, plain channel
- **Commit range reviewed**: `9745aeb2a7a858986b13c560fd78277f6e6c932a..dde19e051867db5371206f6c455b41712aeb8373`

## Tests

- **Baseline status at finish**: `passing`
- Before and after fast-forward: `unshare -m sh -c 'mount -t tmpfs tmpfs /run && cargo test --all-features --locked'` with isolated HOME/XDG/state and local Ruby: 954 passed, 0 failed, 4 ignored on each run.
- Native Windows/macOS and real service-manager lifecycle remain unverified; see `verify.md`.

## Next step

Review this receipt, then run `/opsx-archive` on the feature branch. Push the archive commit only with explicit authorization; create or merge a PR only with explicit authorization.
