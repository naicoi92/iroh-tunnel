## Why

On Windows, `service stop` in the user tier runs `schtasks /End`, which kills the role without the graceful drain and leaves its status snapshot behind. The spike in #185 measured that Task Scheduler sends `WM_CLOSE` but kills the process about 1 s later, too soon for the 3 s drain. The spec and `docs/services.md` do not say so, and "User service lifecycle" even claims the status file is removed after stop. Recording the limitation makes spec, docs and tests match real behaviour and points users to the system tier for a graceful stop.

## What Changes

**Windows user-tier stop**
- From: unspecified; docs say only "`stop` ends the task".
- To: a requirement states the process is ended without the graceful drain, its status snapshot and owner record may remain (reported stale by `serve status`/`access status`, never blocking a start once the process has exited), no restart-on-failure follows, and the system tier is the graceful-stop path.
- Reason: measured behaviour in #185; the decision rule there selects documenting over implementing.
- Impact: non-breaking; documentation and specification only.

**User service lifecycle**
- From: "Full lifecycle" says the status file is removed after stop for every user service.
- To: removal after stop is required for systemd and launchd user services; the Windows logon task follows the new requirement.
- Reason: the old scenario was false on Windows.
- Impact: non-breaking; no runtime change.

**Docs and test comment**
- `docs/services.md` (Windows user tier) states the hard termination and the system-tier alternative.
- `tests/service_user_windows.rs`: the module comment cites the new requirement; the lifecycle test asserts the stale warning when the snapshot remains and starts again with leftovers in place instead of deleting them; the `/End` drain outcome stays reported, not asserted.

Non-goals: no runtime change, no `WM_CLOSE` handling, no new stop command or control channel.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `system-service-management`: adds "Windows user-tier stop terminates without draining"; narrows the "Full lifecycle" scenario of "User service lifecycle" to systemd and launchd.

## Impact

- Files: `openspec/specs/system-service-management/spec.md` (on archive), `docs/services.md`, `tests/service_user_windows.rs` (integration test only).
- No code, API, CLI, dependency, compatibility or security impact. Users who need a graceful stop on Windows must use the system tier (elevated install).
