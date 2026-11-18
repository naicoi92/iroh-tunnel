## Design Summary

Specify the Windows system service's status-reporting behaviour that issue #223 added in code (commits `c0dcf47`, `d317572` on `fix/windows-scm-effects`), so the living spec is the contract operators and future changes rely on.

Today `system-service-management` says only that a non-clean run ends as a "service-specific failure" ("Windows service recovery and exit mapping") and that `Running` is never reported before the startup checks ("Windows service reports running only after startup checks"). It says nothing about what happens when the Service Control Manager refuses a status report, or when the service's own bookkeeping fails. The implementation now:

- logs every refused status report with its state and checkpoint;
- retries `Running` and `Stopped` a bounded number of times;
- when `Running` keeps being refused, drains the role and stops with a service-specific failure instead of staying start-pending;
- treats a poisoned internal state or a failure to start the pending-checkpoint ticker as fatal, ending the service with a service-specific failure instead of continuing or aborting the process;
- uses fixed service-specific codes for these faults: 100 (internal state poisoned), 101 (status report refused), 102 (status ticker could not start).

Effect recording on Windows system-scope operations is already required by "Partial install reports applied effects" (`frontend-independent-management`); this change adds only a Windows scenario for it (reinstall whose create fails after the old service was deleted lists the deletion).

Goals: the spec states the fail-closed reporting policy and the three codes as a contract. Non-goals: changing the existing exit mapping (exit 6 stays clean, other errors stay service-specific), recovery actions, or any non-Windows backend. Anti-goal: a spec that lets a service sit in start-pending silently.

## Alternatives Considered

### Alternative A: Contract with fixed codes
- **Approach**: New requirement plus scenarios naming codes 100, 101, 102.
- **Pros**: Operators can alert on exact codes in the event log; tests pin them.
- **Cons**: Renumbering later is a spec change.
- **Why not chosen**: Chosen.

### Alternative B: Contract without codes
- **Approach**: Same requirement, but only "a service-specific code distinct from the CLI exit codes".
- **Pros**: Codes can change freely.
- **Cons**: Operators get no stable value to monitor; the codes are observable anyway.
- **Why not chosen**: The user preferred a stable, monitorable contract.

### Alternative C: Rely on the existing generic wording
- **Approach**: Add nothing; "any other error SHALL be reported as a service-specific failure" is read as covering it.
- **Pros**: No spec work.
- **Cons**: Retry and fail-closed behaviour stay unspecified; a regression to silent start-pending would not contradict the spec.
- **Why not chosen**: Leaves the #223 guarantees untestable against the spec.

## Agreed Approach

Alternative A. Add one requirement to `system-service-management`, "Windows service status reports fail closed", with scenarios for a refused `Running` (retried, then failure 101), a refused `Stopped` (retried), a poisoned internal state (100) and a ticker that cannot start (102). Add one Windows scenario under the existing "Partial install reports applied effects" requirement. No existing requirement's meaning changes; the new requirement narrows "any other error" for these specific faults.

## Key Decisions

- Codes 100, 101, 102 are part of the contract (user decision).
- The policy is "retry `Running` and `Stopped`, then fail"; the exact retry count and delay stay implementation details, not spec.
- A recorded fault overrides a clean exit: even a drained run that exits 0 reports the fault code.
- Pending (start/stop-pending) reports are not retried individually; the next tick repeats them.
- Implementation already exists; apply only traces tests to scenarios and adds the missing test (a stop control arriving while `Running` is being retried).

## Open Questions

None.
