## Why

On macOS, `service install --user` fails with the opaque `Bootstrap failed: 5: Input/output error` whenever launchd still holds the job label in its persistent per-user disable list. That flag survives bootout, uninstall and reboot, so the install can never recover on its own and the operator is left with no path forward. Clearing it exposes a second defect: an install that conflicts with a running service in the other scope is still accepted, and the resulting job exits as "already running" — a status macOS reads as a crash, so launchd respawns it indefinitely.

## What Changes

**launchd job loading**
- From: a load boots out any previous job, then bootstraps the plist.
- To: a load first clears the job label's disabled flag in the target domain, then boots out any previous job, then bootstraps the plist.
- Reason: launchd refuses to bootstrap a disabled label, and no step ever cleared that flag. The Linux backend already enables its unit as part of the same operation; the macOS backend did not.
- Impact: non-breaking. A host that was never in the disabled state loads exactly as before, because clearing the flag is idempotent.

**Install pre-checks**
- From: install refuses an unsupported scope, a missing privilege, an unresolvable account and an unusable config, but not a role already running in the other scope.
- To: install additionally refuses, before any registration is written, when the role is held at the lock tier of the scope that was *not* requested, naming that scope and the command that removes its service.
- Reason: the accepted install produces a job that cannot run, and on macOS that job restarts forever. `service start` already refuses a running role; install refused nothing.
- Impact: non-breaking for a host with no cross-scope conflict. Because the refusal fires only on a tier mismatch, re-installing over the same scope's own running service stays supported, and the check is unreachable on Windows, which has only the user scope. An install that previously "succeeded" into a broken state now fails with a conflict error and changes nothing.

**launchd load failure guidance**
- From: a failed bootstrap suggests inspecting the service with a command that reports the service as simply absent.
- To: a failed bootstrap names what an operator can actually inspect to reach the diagnosis.
- Reason: the suggested command is a dead end for the most common cause of the failure, so the guidance actively misleads.
- Impact: non-breaking; message text only.

Also corrected: the generated plist declares a document type URL that does not name Apple's property-list definition. Nothing fetches it, so behaviour is unchanged, but the declaration is wrong.

The restart-loop containment gap between the platforms is recorded as a known limitation rather than closed. The Linux unit prevents a restart on the role's "already running" status; launchd's restart condition cannot express an exit-status exception. Closing it would require changing the role's exit classification, which is a cross-platform contract and not the cause of either defect.

## Capabilities

### New Capabilities

None. Both defects sit inside behaviour the existing service-management capability already describes.

### Modified Capabilities

- `system-service-management`: the launchd registration requirement gains the enable step in the load sequence, the corrected document-type declaration, and the load command order; the launchd verb requirement gains accurate load-failure guidance; the existing `service start` refusal requirement is renamed and extended to cover a cross-scope install conflict; the shared restart-policy requirement records launchd's inability to exempt an exit status, which the systemd unit does express; the systemd lifecycle requirement states explicitly that a same-scope re-install still restarts a running unit, so the new refusal is not read as covering it.

## Impact

Affected code, all within the host crate's service module:
- `crates/host/src/service/backend/launchd.rs` — the load sequence, the plist's document-type declaration, and the note shown when a load fails.
- `crates/host/src/service/install.rs` — the pre-check sequence that runs before a registration is written.
- `crates/host/src/service/native.rs` — the note carrying load-failure guidance, if its fields change.

No new dependencies. No change to the CLI's arguments or to any configuration file. The role's exit classification, the ownership and locking contracts, and the systemd, BusyBox and Windows backends are untouched. Behaviour changes are confined to macOS, except the install pre-check, which is written for every platform while firing only on a cross-scope tier mismatch.

Spec bookkeeping: the modified requirements carry refreshed `Evidence:` lines. The baseline's evidence for the `service start` refusal cited `src/app.rs:333-345`, which predates the extraction of the host crate; the paths are corrected rather than dropped.
