# Apply Receipt

**Change**: `ownership-fail-closed`
**Iteration**: `1`
**Applied at**: `2026-11-17 02:04 UTC`
**Executor**: `subagent-driven-development` (implementer: `implementer` agent; task and final reviewer: `code-reviewer` agent)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.ownership-fail-closed`
- **Worktree branch**: `ownership-fail-closed`
- **Feature branch**: `ownership-fail-closed` (pre-created by the user as this worktree's branch; no separate worktree was created, per user instruction)
- **Base branch**: `main`

---

## Commits

- **Range**: `2d408796ae4fff89c197bcb7fbe04b27c81b09af..b81db52e1f1cd6c2e75280bacc1b0d20a5f9d35d`
- **Count**: `4` (575e913, 3fd39ae, dafc9f5, b81db52); this receipt commit (with tasks/plan progress) follows the range.

---

## Tasks

- **Completed**: `12 of 12` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: none

---

## Contract deviations

- `machine_owner_record_is_world_readable_under_umask_077` (internal test, not a type contract): replaced by the fork-free `machine_owner_record_mode_ignores_the_creation_mode` and `user_owner_record_is_private_even_over_a_wider_temp`; a `umask(077)` child test spawned from the lib test binary inherited concurrent tests' lock fds and made lock tests flaky (7/40). `service/mod.rs` therefore keeps `mod fixtures` private. Recorded in plan.md.

No `public` contract changed; the identity-unknown refusal message matches plan.md Contracts.

---

## Lanes

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| Task 1 | sequential | `2d40879..575e913` | Approved (3 Minor, parked) | n/a |
| Task 2 | sequential | `575e913..dafc9f5` | Approved; Minor 1,2,4 fixed in dafc9f5, Minor 3 parked | n/a |
| Task 3 | sequential | none (gates only) | fmt, clippy `-D warnings`, full `cargo test --all-features --locked` green | n/a |

Final whole-branch review over `2d40879..dafc9f5`: "With fixes" (Important: missing CHANGELOG entry; Minor: `reason =` on `allow(dead_code)`). Fix wave `dafc9f5..b81db52`, scoped re-review Approved. Parked follow-ups: `held_exclusively` I/O error is not a Conflict (pre-existing); no test for a lock freed during the retry; spec `Evidence:` paths to fix at sync/archive; test-utils `machine_dir` error path untested; `in_isolated_home` child tests can inherit lock fds (pre-existing). Limitation: Windows-only branches were reviewed by reading but not compiled on this macOS host; Windows CI is the first build.

---

## Next step

Run `/opsx-verify` in the authoritative worktree.
