# Apply Receipt

**Change**: `busybox-script-hardening`
**Iteration**: `1`
**Applied at**: `2026-11-17 11:00`
**Executor**: `subagent-driven-development` (implementer agent per task; code-reviewer agent for task, re-review and final whole-branch review seats)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.busybox-script-hardening`
- **Worktree branch**: `busybox-script-hardening`
- **Feature branch**: `busybox-script-hardening` (the user-created worktree branch was used directly as instructed; no separate apply worktree was created)
- **Base branch**: `main` (merge-base `82d7d92`)

---

## Commits

- **Range**: `b35bde1..HEAD` (implementation `b35bde1..cb39a08` plus this receipt commit; `b35bde1` is the artifact scaffold commit)
- **Count**: `10` before this receipt

---

## Tasks

- **Completed**: `13 of 13` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: none

---

## Contract deviations

- none (spec wording of "BusyBox install refuses non-UTF-8 paths" narrowed to "before the init script is written or any service command runs", commit `561e099`; no contract shape changed)

---

## Lanes

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| 1 | sequential | `b35bde1..fcde8b4` | spec ✅, quality approved (3 Minor) | n/a |
| 2 | sequential | `fcde8b4..ab53e5e` (+ spec `561e099`) | spec ✅, approved (1 Important ruled, Minors) | n/a |
| 3 | sequential | `561e099..903add0` | changes needed → fix round 1 → re-review ✅ | n/a |
| 4 | sequential | `cfa478e..3b2da9d` | controller review ✅ | n/a |
| final fix wave | sequential | `3b2da9d..cb39a08` | final review "With fixes" → re-review "Ready to merge: Yes" | n/a |

---

## Verification evidence

- macOS: `cargo fmt --check` clean; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` clean; `cargo test --all-features --locked`: all pass except `tests/update.rs::update_does_not_stop_a_running_process_of_the_old_binary` ("tunnel did not start"), which fails identically at base `82d7d92` (pre-existing, unrelated).
- Linux docker (rust:1.91-bookworm, dash): clippy clean; `service::backend` 87 passed.
- Linux privileged E2E (`IROH_TUNNEL_E2E_PRIVILEGED=1`, `tests/service_system.rs`): 11 passed.
- BusyBox ash (throwaway container, reported by implementer and final reviewer): busybox_script exec tests pass; injection, wrong-process signalling, legacy fallback, ` (deleted)`, SIGKILL after grace, non-root status 4, zombie checked on a real /proc.

## Parked findings (follow-up candidates)

- New-format identity compares only `/proc/<pid>/exe`; when serve and access run the same Direct binary, a stale serve PID reused by the access daemon would be treated as ours. Hardening: also match `cmdline` when readable.
- `uninstall` ignores a failing `stop` (unverifiable identity) silently apart from the script's stderr; a ServiceNote warning would need a new CLI rendering (#225 overlap).

---

## Next step

Run `/opsx-verify`.
