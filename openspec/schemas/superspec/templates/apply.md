# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `<change-name>`
**Iteration**: `1`
**Applied at**: `YYYY-MM-DD HH:mm`
**Executor**: `subagent-driven-development` | `executing-plans`

---

## Workspace

- **Worktree path**: `<absolute worktree path>`
- **Worktree branch**: `<change-name>`
- **Feature branch**: `<branch checked out before apply step 0>`
- **Base branch**: `<integration branch, typically main>`

---

## Commits

- **Range**: `<from-sha>..<to-sha>` (or `none` if nothing committed yet)
- **Count**: `<n>`

---

## Tasks

- **Completed**: `X of Y` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: `<list ids of unfinished tasks, or "none">`

---

## Contract deviations

> Contracts changed during apply, each with its reason. Cumulative:
> keep entries from earlier iterations. Write `none` if there are none.

- `<contract name>`: `<what changed and why>`

---

## Lanes

> One row per plan task. Lane branch is `<change-name>--task-<N>` or
> `sequential`. The Commit range above covers the lane merges too.

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| `<task N>` | `<change-name>--task-<N>` or `sequential` | `<from-sha>..<to-sha>` | `<verdict>` | `<sha>` |

---

## Next step

`<e.g., "Run /opsx-verify" or "Re-run apply to address <issue>">`
