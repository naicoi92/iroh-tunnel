# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `specify-windows-scm-fault-reporting`
**Iteration**: `1`
**Applied at**: `2026-11-17T12:35Z`
**Executor**: `subagent-driven-development` (implementer: `implementer` agent; task and final reviewer: `code-reviewer` agent)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.fix-windows-scm-effects`
- **Worktree branch**: `fix/windows-scm-effects` (user-approved deviation: no separate `specify-windows-scm-fault-reporting` worktree; the session must not change worktree, and the change only adds tests on top of the #223 fix)
- **Feature branch**: `fix/windows-scm-effects`
- **Base branch**: `main`

---

## Commits

- **Range**: `5809ac7..353b427` (implementation `5809ac7..7703f1f`, receipt `353b427`; this range note amended in the following commit)
- **Count**: `7`

---

## Tasks

- **Completed**: `10 of 10`
- **Remaining**: none

---

## Contract deviations

- none

---

## Lanes

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| Task 1 | `sequential` | `5809ac7..2c8a80c` | clean after fix round 1 (Important: assertion inside `thread::scope` hung instead of failing) | n/a (sequential) |
| Task 2 | `sequential` | `b718255..6006153` | clean | n/a (sequential) |
| Task 3 | `sequential` | none (verification only; tick commit `7703f1f`) | n/a | n/a |

Final whole-branch review over `5809ac7..7703f1f`: Approve with notes, no Critical/Important; deferred minors kept (doc wording on `gated_on_first_running`, stricter log/error-message assertions).

## Verification run (controller)

- `openspec validate specify-windows-scm-fault-reporting --strict`: valid
- `cargo fmt --all -- --check`: clean
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: clean
- `cargo xwin clippy --workspace --all-targets --all-features --locked --target x86_64-pc-windows-msvc -- -D warnings`: clean (Windows cross-check; native Windows clippy not runnable here)
- `cargo test --all-features --locked` (RUBY = Homebrew portable Ruby): 1068 passed, 0 failed
- Every test named in plan.md "Scenario → test" exists.

## Coverage limits (task 3.3)

- Review only, no test: the dispatcher's process exit after faults 100/101, the ticker-spawn skip of `run_to_code` (102), the service-log open/quiet/untrusted conditions.
- Indirect: "Fault triggers recovery" via `failure_plan_from_default_policy` plus `tests/service_system_windows.rs::crash_is_restarted` (exit 1 stands in for 101), Windows CI only.
- `#[cfg(unix)]`: `start_is_refused_before_manager_when_role_runs`, `partial_install_reports_written_registration` (returns early on Linux without systemd).
- Windows e2e `service_system_windows` not run here; left to Windows CI.

---

## Next step

Run `/opsx-verify`.
