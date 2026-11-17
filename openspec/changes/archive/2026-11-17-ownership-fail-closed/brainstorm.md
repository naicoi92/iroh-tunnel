## Design Summary

Issue #221: two role-ownership checks fail **open** when an input cannot be read. This change makes both fail **closed** without introducing false refusals in normal operation.

1. **Same public identity across roles** (living spec `role-ownership-shutdown`, Requirement "Same public identity rejected across roles"). Today the owner record is written best-effort (write errors are only logged) and any read or JSON error of the other role's record is treated as "no record". A holder whose record cannot be read is skipped, so Serve and Access can run with one key.
2. **Windows machine tier** (Requirement "Machine-tier lock between system service and user processes"). Today a failure to resolve `%ProgramData%` turns into "no machine tier", so claim and probe silently check only the user tier and system/user conflict detection is disabled.

Behaviour after the change:

- **Publishing is mandatory.** A starting role must successfully publish its owner record (at claim time, and again with its public node id) in every tier it holds exclusively. A failed publish refuses the start, naming the record path.
- **Reading the other role's record is three-way:**
  - record readable and well-formed: compare node ids as today;
  - record **exists but cannot be read or parsed**: refuse the start with the conflict status (exit 6), naming the record path — the identity of a running holder is unknown, so it is treated as a possible clash;
  - record **absent**, or present with **no node id yet**: the other role has not published its identity; the start proceeds. Within one tier this is safe because both sides publish before they inspect the other, so of two same-key starts at least one sees the other's identity and is refused.
- **Only an exclusive holder's record counts.** At the machine tier a record is consulted only when the other role's lock is held exclusively (by a system holder, the only one that writes a machine record). Locks held only shared by user processes carry no machine record, so a stale or corrupt machine record left by a crashed system service never blocks a start.
- **Known limitation (unchanged, out of scope):** a user-session process of one role and a later system service of the other role with the same key are not detected, because the system start cannot see per-user records of other accounts. The reverse order is refused. Tracked as a follow-up.
- **Windows machine tier resolution failure is an error**, not "no machine tier". `None` remains only for platforms without a machine tier. Claim, probe, status and install paths propagate the error with context. Choosing the machine log directory stays best-effort (it only picks where logs go and does not decide ownership).
- **Preserved:** a machine lock file that does not exist yet still means "no machine-tier owner" (Requirement "Non-elevated start does not create a machine lock").

This alters the specified behaviour of: "Same public identity rejected across roles" (adds the unreadable-record refusal and mandatory publish), "Owner record is informational and public only" (publishing becomes a start precondition; liveness is still the lock, never the record), and "Machine-tier lock between system service and user processes" (resolution failure is fatal on Windows).

## Alternatives Considered

### Alternative A: Symmetric fail-closed (chosen)
- **Approach**: mandatory publish before probing; refuse only on a record that exists but is unreadable or corrupt; absent or not-yet-identified records are "not published yet".
- **Pros**: closes the fail-open hole; no false refusal during concurrent starts, while the other role is stopping (record removed before the lock is released), or when the other role is a shared machine-tier holder that never writes a machine record.
- **Cons**: relies on the publish-before-inspect ordering for absent records; must be stated and tested.
- **Why chosen**: the only option that fixes #221 without breaking legitimate start orders.

### Alternative B: Absolute fail-closed
- **Approach**: any held lock without a readable record carrying a node id refuses the start (optionally after a short retry).
- **Pros**: simplest rule.
- **Cons**: false refusals in normal operation — a role starting while the other has claimed but not yet bound (ephemeral key), while the other is shutting down, and a system service starting while any user runs the other role (shared machine holders never publish a machine record).
- **Why not chosen**: turns a security fix into an availability regression.

### Alternative C: Lock keyed by public identity
- **Approach**: each role also takes an exclusive kernel lock named after its public identity.
- **Pros**: kernel-enforced; no JSON parsing in the decision.
- **Cons**: a non-elevated user cannot create a lock in the machine directory, so it cannot cover the user/system cross-tier case; lock files are never unlinked, so one file accumulates per key ever used.
- **Why not chosen**: does not cover the cross-tier case the requirement names, and adds unbounded files.

## Agreed Approach

Alternative A, confirmed by the user, together with the Windows `machine_dir` change: resolution failure propagates as an error with context through claim, probe, status and install; the absent-lock-file exception is kept; machine log directory selection stays best-effort.

## Key Decisions

- Unreadable or corrupt record of a **running** other role → refuse with `CliError::Conflict` (exit 6), message names the record path and states the identity could not be determined.
- A record of a role that is **not running** (lock not held), or at the machine tier not held exclusively, is never read for the identity decision, so leftover garbage from a crash never blocks a start.
- User-then-system same-key detection across accounts stays out of scope (follow-up issue).
- Absent record or record without node id → proceed (other role has not published yet).
- Owner record publish failure (claim or identity publish) → start refused with an error naming the path; liveness is still decided only by the lock.
- Windows `%ProgramData%` resolution failure → error in claim/probe/status/install; tests inject the resolver so the behaviour is tested on every OS.
- Machine log directory selection remains best-effort.
- No new dependency, no change to exit codes or the lock file layout.

## Open Questions

None.
