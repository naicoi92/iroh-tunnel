# Apply Receipt

**Change**: `independent-role-binaries`  
**Iteration**: `1`  
**Applied at**: `2026-11-16 10:37 UTC`  
**Executor**: `subagent-driven-development`

## Workspace

- **Worktree path**: `/root/projects/iroh-tunnel.independent-role-binaries`
- **Worktree branch**: `independent-role-binaries`
- **Feature branch**: `feat/independent-role-binaries`
- **Base branch**: `main`

## Commits

- **Range**: `9745aeb2a7a858986b13c560fd78277f6e6c932a..e865f02eb80536d67631aac97e05be5e8f1b80f1`
- **Count**: `16` implementation and planning-progress commits; this receipt commit follows the range.

## Tasks

- **Completed**: `9 of 9` checkboxes in `tasks.md`
- **Remaining**: none

## Contract deviations

- **Lifecycle invocation APIs**: user approved additional invocation-aware public lifecycle functions while retaining the original combined-compatible wrappers. Their signatures are recorded in `plan.md`.
- **InstallRequest migration**: the approved `invocation` field is source-breaking for existing struct literals; add `invocation: ServiceInvocation::Combined` to preserve combined behavior.
- **Homebrew aliases**: user approved standard Homebrew artifact conflict and removal semantics, including possible removal of a replacement symlink and replacement of dangling links; documented in `docs/install.md`.
- **Spec alignment**: user approved modifications to proposal and delta specs to resolve existing combined-only service and Unix trust guidance contradictions.

## Lanes

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| Task 1 | sequential | `9745aeb..73f5272` | pass after fixes; native Windows/macOS unavailable | n/a |
| Task 2 | sequential | `2ced812..a861c74` | pass with native Windows limitation | n/a |
| Task 3 | sequential | `6ae7f58..4b3441f` | pass with approved Homebrew semantics and native macOS limitation | n/a |
| Task 4 | sequential | `1612631..a58eb7c` | pass after documentation fix | n/a |

Whole-branch review identified macOS test portability, PowerShell guidance and specification inconsistencies. Fixes `7e237a7..c5afc8d` and revised specs received a scoped pass; native macOS, native Windows and real service-manager lifecycle remain unverified. Linux test runs initially encountered missing Ruby, host role locks, and transient port/lock contention; isolated reruns were reported passing. Living specs are not yet synced.

## Next step

Run `/opsx-verify` in the authoritative worktree.
