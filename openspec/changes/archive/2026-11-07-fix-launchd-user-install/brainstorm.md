## Design Summary

`iroh-tunnel <role> service install --user` fails on macOS with `Bootstrap failed: 5: Input/output error` whenever launchd holds the job label in its persistent per-user disable list. The install writes a valid plist, boots out any previous job, then asks launchd to bootstrap it — but never clears the disabled flag, so launchd refuses the job and the operator sees an opaque I/O error with no path forward. The disabled flag survives bootout, uninstall and reboot, so the install can never recover on its own.

Clearing that flag exposes a second defect. When a service for the same role is already registered and running in the other scope, install still proceeds: the newly bootstrapped job starts, the role refuses to run a second instance, and the job exits with the role's "already running" status. On macOS that exit is indistinguishable from a crash, so launchd respawns it indefinitely. The equivalent Linux unit already prevents this; the macOS job has no comparable containment.

This change makes a user-scope install on macOS succeed when it should, and refuse cleanly when it must not proceed, so the operator never ends up with an opaque failure or a job that restarts forever. The scope covers both defects together: fixing only the disabled flag would hand the operator a restart loop on their very next install, which is the symptom that prompted the work.

Goals:
- A user-scope install succeeds on a host where the job label was previously disabled, without the operator running any launchd command by hand.
- An install that cannot lead to a working service is refused before anything is written or registered.
- A failed load tells the operator what to inspect, and the suggested command actually produces the diagnosis.

Non-goals:
- Changing how the role decides it is already running, or the status it exits with. That contract is shared across all three platforms and is not the cause of either defect.
- Reconciling the two scopes automatically. An install never stops, uninstalls or migrates a service in the other scope.
- Cleaning up the state directory or status file left behind by a service that ran under a different account.

Anti-goal: an install that reports success while leaving behind a job that cannot run, or that restarts forever.

## Alternatives Considered

### Alternative A: Clear the disabled flag unconditionally before loading, and refuse an install that conflicts with the other scope

- **Approach**: The macOS backend clears the job label's disabled flag as an ordinary step of loading, in the same position the Linux backend already enables its unit. Separately, install gains the same "role already running" pre-check that `service start` performs, and refuses with a conflict error naming the other scope before writing anything.
- **Pros**: Both defects are addressed where they originate. The load sequence becomes symmetric with the Linux backend, so the two platforms no longer diverge on whether loading includes enabling. The refusal reuses the role-liveness check and the conflict-error style that already exist, so there is no new vocabulary for operators to learn. Install keeps its existing discipline of failing before it writes.
- **Cons**: Does not stop a restart loop that an operator could still create by invoking launchd directly, or by installing the two scopes in the other order. That residual gap has to be documented rather than closed.
- **Why not chosen**: This is the agreed approach.

### Alternative B: Clear the disabled flag only after a load fails, as a retry

- **Approach**: Keep the current load sequence. When the bootstrap fails, inspect launchd's disabled list; if the label appears there, clear it and bootstrap once more.
- **Pros**: Touches the successful path not at all, so a host that was never in the disabled state behaves exactly as before.
- **Cons**: Diagnoses by inference. The same I/O error covers several causes, so the retry either guesses or has to parse launchd's disabled-list output, which is a text format with no stability guarantee. It also leaves the two platform backends asymmetric: one treats enabling as part of loading, the other as error recovery, which is how the gap arose in the first place. A retry path is exercised far more rarely than the path it guards, so it is the more likely of the two to rot.
- **Why not chosen**: More machinery, less certainty, and it preserves the asymmetry between the platforms that produced the bug.

### Alternative C: Refuse the install and tell the operator to clear the flag themselves

- **Approach**: Detect the disabled state during the install's pre-checks and refuse, with instructions for the operator to clear it and re-run.
- **Pros**: Never changes machine-wide launchd state on the operator's behalf, which is defensible for a flag an administrator may have set deliberately.
- **Cons**: Makes the operator do by hand what the Linux path does for them, for a flag they most likely never set knowingly. It is also the wrong read of the flag's meaning: a disable recorded for an earlier install of this same service is stale bookkeeping, not an administrator's standing decision about a service they are installing right now.
- **Why not chosen**: An install is already an explicit instruction to make the service runnable; asking for a second confirmation of that instruction is friction with no safety benefit.

### Alternative D: Make the role exit cleanly when it refuses to start, so the job is not respawned

- **Approach**: When the role finds itself already running, exit with a status macOS reads as a clean exit, so the job's restart condition does not match and launchd leaves it stopped.
- **Pros**: Closes the restart-loop gap completely, including the paths a refusal at install cannot reach.
- **Cons**: Changes the role's exit classification, which is a cross-platform contract the project maintains deliberately and which callers and tests depend on. It alters Linux and Windows behaviour to contain a symptom that only appears on macOS, and it makes "refused to start" indistinguishable from "ran and finished" to every other caller.
- **Why not chosen**: The cost lands far outside the defect. Refusing the install removes the loop from the path operators actually take, and the residual gap is better documented than paid for with the exit contract.

## Agreed Approach

Alternative A. Clearing the disabled flag as part of loading puts the fix at the point where the two platform backends diverged, and it makes the macOS load sequence mean the same thing as the Linux one: make this service runnable, then run it. Refusing a conflicting install reuses a check and an error shape the project already has, and keeps install's existing promise that it fails before it writes rather than after.

The two defects ship together because they are the same operator experience. Clearing the flag is what makes the install succeed; refusing the conflict is what makes that success mean something. Shipping only the first would turn an opaque error into a job that restarts forever, which is worse.

The restart-loop gap that remains — reachable by invoking launchd directly, or by installing the scopes in the other order — is recorded as a known limitation of the platform rather than closed, because the only ways to close it cost more than the gap.

## Key Decisions

- **Both defects are one change.** Scope covers the disabled flag and the cross-scope conflict. Fixing the flag alone would hand the operator the restart loop on their next install.
- **Clearing the disabled flag is part of loading, not error recovery.** It happens on every load, in the position the Linux backend already occupies, so the two backends describe the same sequence. It needs no inspection of launchd's state and no parsing of its output.
- **A conflicting install is refused, not reconciled.** Install refuses when the role is already running in the other scope, before writing a registration, and names the other scope in the error so the operator knows which service to remove. Nothing in the other scope is stopped, removed or migrated automatically — consistent with the project's existing rule that nothing is stopped on the operator's behalf.
- **The refusal reuses the existing role-liveness check**, the same one `service start` performs, rather than introducing a second way to ask whether a role is running.
- **The role's exit classification is unchanged.** It is a cross-platform contract, and the defect is a macOS-only consequence of how that platform reads exit statuses.
- **The residual restart-loop gap is documented, not closed.** It is a platform limitation, recorded in the design where the two platforms' restart containment is compared.
- **A failed load must point at a command that works.** The current guidance names a command that reports the service as simply absent, which is a dead end; the diagnosis must name what an operator can actually inspect. (Carried as a decision rather than an afterthought because it is what turns the underlying failure legible.)
- **No cleanup of operator machine state.** Files left by a service that ran under another account are out of scope; the operator was told what to run and chose to handle it themselves.

## Open Questions

None. The two design choices with real alternatives — how the conflicting install behaves, and whether to close the restart-loop gap — were settled during brainstorming.
