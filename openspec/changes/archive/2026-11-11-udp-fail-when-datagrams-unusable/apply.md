# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `udp-fail-when-datagrams-unusable`
**Iteration**: `1`
**Applied at**: `2026-11-11 01:31` (UTC)
**Executor**: `subagent-driven-development` (implementer: general-purpose; task, final and re-reviewer: code-reviewer; thinking from task Difficulty, no model override)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.udp-fail-when-datagrams-unusable`
- **Worktree branch**: `udp-fail-when-datagrams-unusable`
- **Feature branch**: `spec/udp-fail-when-datagrams-unusable` (created from `main` before step 0, because apply started on `main`; the scaffold commit `ec070c4` is on it, `main` is untouched)
- **Base branch**: `main`

---

## Commits

- **Range**: `ec070c4..65d5f28` (implementation, task ticks and the final-review fix wave; this receipt commit follows)
- **Count**: `17`

---

## Tasks

- **Completed**: `25 of 25` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: none

---

## Contract deviations

none. Plan Contracts kept their shapes. Additions inside the declared private scope, without a change to any listed shape:

- `run_access.rs`: private helpers `UdpSession` and `reject_unusable` that split `udp_forward_loop` (approved by the production review).
- `UdpTestHooks` (`#[cfg(test)]`): two counters added for test observation, `waiting_dropped` (task 4 fix round 1) and `readers_alive` with a `ReaderAlive` drop guard (task 4 fix round 2). Both compile out of non-test builds.
- `ServiceDialer::cached_conn_id` (`#[cfg(test)]` read-only observer, allowed by the plan's Global Constraints).

---

## Lanes

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| 1 | sequential | `ec070c4..01ef70d` | approved, no findings | n/a (sequential) |
| 2 | sequential | `1106d5e..dcd2845` | approved, 1 minor deferred | n/a (sequential) |
| 5 | sequential | `513d407..ff8c251` | approved | n/a (sequential) |
| 3 | sequential | `167292b..b58f41d` | approved after 1 fix round (test lifetime) | n/a (sequential) |
| 4 | sequential | `ce39562..8a8cb93` | production approved; tests approved after 2 fix rounds. The first single review ended with `stopReason: length` and no report; the user chose to re-review as two narrow reviews | n/a (sequential) |
| 6 | sequential | `27e953a..6d689b3` | approved after 1 fix round (matchers, quick mode, listener addresses) | n/a (sequential) |
| final | sequential | `f1de4e1..65d5f28` | whole-branch review: part A (production and docs) ready to merge; part B (tests) fix first, then one fix wave; the scoped re-review addressed 6 of 7 findings, and the residual items are parked below | n/a (sequential) |

Order run: T1, T2, T5, T3, T4, T6, all sequential in one worktree (respects Blocked by; avoids parallel cargo target dirs).

---

## Verification evidence (controller-run)

- Full gate at `f1de4e1` all exit 0: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, `RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps --locked`, `cargo test --all-features --locked`, `cargo +1.91 test --workspace --all-features --locked` (76 test binaries ok).
- After the fix wave `65d5f28`: the fixer ran the full gate green, except one unexplained failure of `adoption_check_closes_and_backs_off` on the first, cold `+1.91` run. The controller then ran the 1.91 `access::run_access::tests::` group 25 times in isolation and the full 1.91 workspace 6 times: 31 of 31 green.

## Parked findings (rulings)

- Test fixtures on the assertion-failure path abort their tasks on drop but do not await them. A failing test is already failing, the test runtime cancels the tasks, and child processes are killed and reaped synchronously. No production impact.
- `adoption_check_closes_and_backs_off` timing hazard: the test waits for the remote `a.closed()` before it observes the back-off window, so a close-delivery stall of more than about 900 ms could fail its "at least 600 ms left" check. Suggested hardening: observe the window and the processed hooks from local state first, assert the remote close afterwards, or add a controllable clock seam. Follow-up.
- Declined: tightening the existing `tests/udp_tunnel.rs` restart test (it predates this change, outside scope); an automated source assertion for the `limit_shrink` rustdoc (the plan approved a grep gate).

---

## Next step

Run `/opsx-verify udp-fail-when-datagrams-unusable`.
