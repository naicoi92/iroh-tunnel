# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `status-owner-not-running`
**Iteration**: `1`
**Applied at**: `2026-11-09 17:41` (UTC)
**Executor**: `subagent-driven-development` (implementer: general-purpose, thinking low; task and final reviewer: code-reviewer)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.status-owner-not-running`
- **Worktree branch**: `status-owner-not-running`
- **Feature branch**: `spec/status-owner-not-running` (created from `main` before step 0; the artifacts were moved there so they are not committed on `main`)
- **Base branch**: `main`

---

## Commits

- **Range**: `975144b..06aac2b` (implementation and task ticks; this receipt commit follows)
- **Count**: `2`

---

## Tasks

- **Completed**: `5 of 5` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: none

---

## Contract deviations

none

---

## Lanes

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| `1` | `sequential` | `975144b..f66c48f` | task review: approved (spec compliant, no findings); final whole-branch review `975144b..06aac2b`: ready to merge, no findings | n/a (sequential) |

Verification run fresh by the controller at `06aac2b` (macOS): `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, `cargo test --all-features --locked` (all binaries pass, including `cli_golden` with no golden changes), and `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` all pass. Windows was not run locally; CI covers it.

---

## Next step

Run `/opsx-verify`.
