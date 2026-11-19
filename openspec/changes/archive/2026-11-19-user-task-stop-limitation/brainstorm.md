## Design Summary

On Windows, `service stop` for a user-tier role runs `schtasks /End`, which hard-terminates the role: no drain, and the status snapshot is left behind. Issue #185 asked whether the role could turn Task Scheduler's `WM_CLOSE` into a graceful stop. The spike (https://github.com/naicoi92/iroh-tunnel/issues/185#issuecomment-7007922981) answered it:

- A hidden top-level window receives `WM_CLOSE`; a message-only window does not.
- Task Scheduler calls `TerminateProcess` about 1 s later on x64 and ARM64, which is below the shared 3 s drain budget (role-ownership-shutdown, "Single drain deadline of three seconds") plus the shutdown path.
- `/End` does not trigger restart-on-failure.

Under the issue's decision rule, the behaviour is recorded as a known limitation instead of implemented. The change is documentation and specification only; no runtime behaviour changes.

What changes:

- `system-service-management` gains a requirement stating that a Windows user-tier stop ends the process without the graceful drain and may leave the status snapshot behind (reported stale by `status`), that it does not trigger restart-on-failure, and that the system tier is the path for a graceful stop.
- The existing requirement "User service lifecycle" alters a specified requirement: its "Full lifecycle" scenario says the status file "is removed after stop", which is false for the Windows logon task. The scenario is narrowed so that removal-after-stop applies to systemd and launchd user services, and the Windows case points to the new requirement.
- `docs/services.md`, Windows user tier: "`stop` ends the task" becomes an explicit statement that the process is terminated without draining, that the snapshot may stay behind, and that the system tier is the option for a graceful stop.
- `tests/service_user_windows.rs` keeps reporting the `/End` drain outcome as evidence (no new gate), and its module comment cites the new requirement instead of the unrecorded "SM-04" note. When the snapshot remains after stop, the lifecycle test checks the stale warning and starts again with the leftovers in place instead of deleting them by hand.

Goals: the spec, docs and test agree with real behaviour; users needing a graceful stop are pointed to the system tier.
Non-goals: any runtime change to the user-tier role or task registration; a new stop command or control channel (RO-03).
Anti-goals: promising a drain the OS will not allow; a test that fails if a future Windows version waits longer.

## Alternatives Considered

### Alternative A: Handle `WM_CLOSE` with a hidden window and do a partial cleanup
- **Approach**: create a hidden top-level window in the user-tier role; on `WM_CLOSE`, stop admission and remove the status snapshot within the ~1 s window, without a full drain.
- **Pros**: the snapshot would usually be removed; `status` would not need the stale warning.
- **Cons**: a second, shorter stop contract that diverges from the shared drain contract; adds Win32 window and message-loop code to the runtime for a partial result; races the 1 s kill, so cleanup is not guaranteed.
- **Why not chosen**: the issue's decision rule requires enough time to drain; it is not there, and a partial contract adds complexity for a best-effort gain.

### Alternative B: Register the task with `AllowHardTerminate=false`
- **Approach**: stop Task Scheduler from calling `TerminateProcess`, so the role, after getting `WM_CLOSE`, could drain fully.
- **Pros**: full drain.
- **Cons**: a role that does not exit (no window, or stuck) is never stopped by `/End`; `restart` would hit its 15 s timeout; it also requires the window code from Alternative A.
- **Why not chosen**: trades a clean but abrupt stop for a stop that may never happen.

### Alternative C: Record the limitation (chosen)
- **Approach**: spec requirement, docs and the Windows lifecycle test state the hard termination and point to the system tier.
- **Pros**: no runtime risk; matches measured behaviour; small.
- **Cons**: the user tier still leaves a stale snapshot after stop.
- **Why not chosen**: chosen.

## Agreed Approach

Alternative C, as directed by the decision rule in #185 and approved in chat after the spike results. The system tier already drains on stop ("Windows service stop drains"), so users have a graceful path today.

## Key Decisions

- The spike measurements are recorded in #185, not in the repository; the spike code was removed with its branch.
- The new requirement states observable behaviour (process ends, snapshot may remain, `status` reports it stale, no restart) rather than the ~1 s timing, which is Task Scheduler's and may change.
- The Windows `/End` outcome stays reported, not asserted, in `tests/service_user_windows.rs`.

## Open Questions

None.
