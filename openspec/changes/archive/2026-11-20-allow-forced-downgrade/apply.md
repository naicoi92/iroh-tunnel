# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `allow-forced-downgrade`
**Iteration**: `1`
**Applied at**: `2026-11-20 10:30`
**Executor**: `subagent-driven-development` (implementer: `superspec.implementer`; task reviewer: `superspec.task-reviewer`)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.allow-forced-downgrade`
- **Worktree branch**: `allow-forced-downgrade`
- **Feature branch**: `spec/allow-forced-downgrade`
- **Base branch**: `main`

---

## Commits

- **Range**: `7566c97..HEAD` (implementation through `bd4209a`, plus this receipt commit)
- **Count**: `8` before this receipt (2 feat, 2 test, 1 docs, 3 tasks.md ticks)

---

## Tasks

- **Completed**: `12 of 12` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: none

---

## Contract deviations

none

---

## Lanes

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| Task 1 | `sequential` | `7566c97..6a6ed05` | spec ✅, quality approved (0 findings) | n/a (sequential) |
| Task 2 | `sequential` | `3fc7e40..3fa6b69` | spec ✅, quality approved (1 minor, fixed in `3fa6b69`) | n/a (sequential) |
| Task 3 | `sequential` | `0e91a1d..630b3a1` | approved within the final whole-branch review | n/a (sequential) |

Final whole-branch review over `7566c97..630b3a1`: fix first (2 important coverage gaps, 1 minor). One fix wave, `630b3a1..bd4209a`, then a scoped re-review: clean.

Fresh controller verification at `bd4209a`: `cargo fmt --check` clean; `cargo clippy --all-targets --locked -- -D warnings` clean; `cargo test --locked` passed `update` 28, `update_ownership` 11, `cli_usage` 21 and `cli_golden` 18; host `update` unit tests (`--features test-utils`) passed 38.

Known environment notes (pre-existing, not caused by this change):
- `cargo test -p iroh-tunnel-host` needs `--features test-utils`.
- `homebrew_casks` needs `RUBY` set to Homebrew portable Ruby 3.1.
- `tests/update_zip_windows.rs` is Windows-only and was not compiled here.

---

## Next step

Run `/opsx-verify`.
