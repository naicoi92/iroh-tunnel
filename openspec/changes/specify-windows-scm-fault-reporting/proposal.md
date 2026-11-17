## Why

Issue #223 made the Windows system service fail closed when the Service Control Manager refuses a status report or the service's own bookkeeping fails, and gave those faults fixed service-specific codes. The living spec only says "any other error is a service-specific failure", so a regression back to a silent start-pending service, or a renumbered code operators alert on, would not contradict it.

## What Changes

- Add four requirements to `system-service-management`:
  - "Windows service logs refused status reports" (state and pending checkpoint, within the service log's existing rules);
  - "Windows service retries running and stopped reports" (bounded retries; a running or stopped report refused on every attempt ends with service-specific code 101; pending reports are not retried);
  - "Windows service internal faults end the service with fixed codes" (100 poisoned state, 102 ticker cannot start);
  - "Windows service fault codes take precedence over the exit mapping" (a fault replaces even a clean drain; recovery applies).
- Modify "Explicit lifecycle operations keep composite semantics" in `frontend-independent-management`: failed operations return the effects applied so far, including a backend-required rollback's; add Windows scenarios for a reinstall whose create fails after the old service was deleted, and for a rolled-back install.
- **Clarification of an existing conflict**
  - From: "Partial install reports applied effects" says a failed install performs no compensating rollback, while `system-service-management` "Windows system install replaces and rolls back" requires deleting a service that failed to start.
  - To: no rollback unless a backend requirement demands it; the Windows system install's rollback stays, and its effects are reported.
  - Reason: both requirements are living; the Windows one is the shipped behaviour.
  - Impact: non-breaking; no behaviour changes on any backend.
- No other existing requirement changes meaning. Behaviour is already implemented on `fix/windows-scm-effects` (`c0dcf47`, `d317572`); apply traces tests to the scenarios and adds the missing ones. The requirement set grew from the one sketched in brainstorm.md after spec review; brainstorm.md stays as approved history.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `system-service-management`: adds the fail-closed status-reporting requirements and codes 100–102.
- `frontend-independent-management`: partial-effects requirement scoped to backends without a rollback requirement; Windows system-scope scenarios added.

## Impact

- Code: tests in `src/scm_runtime/session.rs` and `crates/host/src/service/backend/windows_scm.rs`. No production change is expected; the one permitted fix is the refusal log text (`Status::describe` / `warn!`) if a test shows it does not name the state and checkpoint.
- Coverage limits: the dispatcher's process exit, the ticker-spawn skip and the log-open conditions are covered by review only, not by tests.
- Compatibility: non-breaking; codes 100–102 become a documented contract for operators (Windows event log, recovery).
- Security: none.
