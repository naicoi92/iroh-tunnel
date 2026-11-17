## Why

Two role-ownership checks fail open (issue #221). An owner record that cannot be read or written is silently ignored, so Serve and Access can run with the same key. On Windows, a failure to resolve `%ProgramData%` silently disables the machine tier, so the system service and a user process of one role can run together. Both defeat ownership guarantees the specs already promise; they must fail closed.

## What Changes

**Identity check across roles**
- From: owner record write errors are logged and ignored; an unreadable or corrupt record of a running other role is treated as "no record" and the start proceeds.
- To: a start publishes its owner record (at claim and with its node id) or is refused, naming the record path. A running other role whose record exists but cannot be read or parsed refuses the start with the conflict status (exit 6), naming the record path. An absent record, or one without a node id yet, still lets the start proceed (within a tier both sides publish before inspecting, so a same-key pair is refused on one side). At the machine tier only an exclusively held lock's record is consulted, so a crashed system service's leftover never blocks.
- Reason: closes the shared-key hole without false refusals during concurrent starts, shutdown, or shared machine-tier holders.
- Impact: non-breaking for healthy hosts; a host whose state directory cannot hold the owner record now refuses to start instead of running unchecked.

**Windows machine tier resolution**
- From: a `%ProgramData%` resolution failure means "no machine tier"; claim, probe, status and install check only the user tier.
- To: the failure is an error with context in claim, probe, status and install. Only platforms without a machine tier have none. A missing machine lock file still means "no machine-tier owner". Machine log directory selection stays best-effort.
- Reason: a silently disabled tier lets the system service and a user process of one role run together.
- Impact: non-breaking on healthy Windows hosts; a host where the known folder cannot be resolved now refuses to start.

**Non-goals:** detecting a same-key user process of one role from a later system service of the other role across accounts (pre-existing limitation, follow-up issue); a lock keyed by public identity; changing exit codes, lock file layout or the owner record format; status reporting semantics beyond propagating the resolution error.

**Security:** strictly tightens enforcement; the owner record still contains no secret material.

## Capabilities

### New Capabilities

None.

### Modified Capabilities
- `role-ownership-shutdown`: "Same public identity rejected across roles" gains the unreadable-record refusal and mandatory publish; "Owner record is informational and public only" makes a successful publish a start precondition; "Machine-tier lock between system service and user processes" makes a Windows resolution failure an error.

## Impact

- Code: `crates/host/src/ownership.rs` (owner record read/write, `RoleClaim::publish_identity`, `claim`, `probe`, `machine_dir`, `LockDirs::resolve`), `crates/host/src/service/install.rs` and `crates/host/src/service/log_file.rs` (callers of `machine_dir`).
- Tests: unit tests in `ownership.rs`, process-level test in `tests/role_ownership.rs`.
- No new dependency, no public CLI or config change.
