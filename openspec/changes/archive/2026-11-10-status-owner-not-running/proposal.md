## Why

Host service status reports a role that is not running as owner `Unavailable("<role> is not running")`. That folds a normal state into the same variant as "owner record unreadable", so a frontend must compare strings to tell them apart. The field must be settled before a TUI or Tauri frontend consumes it (issue #165).

## What Changes

**Owner fact of a role that is not running**
- From: `ServiceStatusReport.owner: Fact<OwnerRecord>`; a role that is not running reports `Fact::Unavailable("<role> is not running")`.
- To: `ServiceStatusReport.owner: Fact<Option<OwnerRecord>>`; a role that is not running reports `Fact::Known(None)`. A running role with a readable record reports `Known(Some(record))`; a running role whose record is missing or unreadable, or a failed liveness probe, still reports `Unavailable`.
- Reason: a not-running role has no owner by definition; that is a known fact, not a missing one.
- Impact: breaking for the `iroh_tunnel_host` public type `ServiceStatusReport`, which has no consumer outside the host crate's own tests today. CLI output is unchanged.

Non-goals: no change to liveness probing, the owner record file, CLI rendering or exit codes.

## Capabilities

### New Capabilities

None.

### Modified Capabilities
- `frontend-independent-management`: the requirement "Service status separates facts without inventing truth" gains a scenario stating that a role that is not running reports its owner as known and absent, not unavailable.

## Impact

- Code: `crates/host/src/service/types.rs` (`ServiceStatusReport.owner` type), `crates/host/src/service/status.rs` (owner derivation and unit tests).
- API: public field type change in the host crate; no CLI, config or on-disk format change.
- Security: none; the owner record already carries only public data (pid, public node id, start time).
- Compatibility: CLI goldens in `tests/cli_golden.rs` must pass unchanged.
