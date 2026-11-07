## Context

`crates/host/src/service/backend/launchd.rs` manages macOS jobs through the modern launchctl vocabulary. Its `load` runs `launchctl bootout <target>` (best-effort, to clear a previous registration) then `launchctl bootstrap <domain> <plist>`. There is no `launchctl enable` anywhere in the module.

launchd keeps a per-domain disable list in `/var/db/com.apple.xpc.launchd/disabled.<uid>.plist`, readable via `launchctl print-disabled <domain>`. A label recorded there cannot be bootstrapped: launchd answers `5: Input/output error`. The flag is keyed by label, not by plist, so it outlives the plist file, every `bootout`, a full `service uninstall`, and a reboot. Nothing in the install path reads or clears it.

The systemd backend does not have this gap. Its `load` runs `daemon-reload`, then `systemctl enable <unit>`, then `restart` — enabling is part of loading. The two backends diverged on whether "load" includes "make runnable", and launchd is the one that omitted it.

`plan_install` in `crates/host/src/service/install.rs` performs every pre-check before a registration is written: absolute-executable, scope decision, platform support, privilege, account resolution and trust for system scope, and config loadability. Its comments state the intent plainly — fail closed before registering anything that would later fail to run. Role liveness is not among those checks. `crate::ownership::ensure_not_running` exists and is called from `verb.rs`, but only for `Verb::Start`.

`ownership::probe` returns `Liveness::Running { tier, owner }` where `Tier` is `User` or `Machine`, each with a human label, and `probe_tiers` checks the user lock then the machine lock. The information install needs is already produced; install simply never asks.

The generated plist declares `PropertyManifest-1.0.dtd` where Apple's definition is `PropertyList-1.0.dtd`. Neither launchd nor `plutil` fetches the DTD, so the file validates and loads; the declaration is simply incorrect.

Constraints this design works within:
- Backend functions run commands through the injected `Native` runner so tests can assert the exact command sequence; no backend step may shell out directly.
- `install` must not write a registration before its checks pass, and a failure must report what was already applied via `AppliedEffect`.
- The role's exit classification is a cross-platform contract shared by `CliError` and the CLI's documented exit codes.
- Behaviour must stay identical on systemd, BusyBox and Windows hosts, except where the install pre-check deliberately applies everywhere.

## Goals / Non-Goals

**Goals:**
- A user-scope install succeeds on a host whose job label was previously disabled, with no manual launchd command.
- An install that cannot produce a working service fails before writing a registration, naming the conflicting scope.
- A failed bootstrap names a command that actually reaches the diagnosis.
- The launchd and systemd load sequences describe the same operation, so the divergence that caused this bug does not recur.

**Non-Goals:**
- Changing the role's exit classification or its liveness detection.
- Stopping, removing or migrating a service in the other scope on the operator's behalf.
- Closing the restart-loop gap for paths that bypass `service install`.
- Cleaning up state files or status files owned by another account.

## Decisions

**Clear the disabled flag as an unconditional step of `load`, before `bootout`.**

`load` gains `launchctl enable <service_target>` as its first command, run through the same `Native` runner as the rest. It is unconditional: no inspection of `print-disabled`, no retry after a failure.

Rationale: `launchctl enable` is idempotent — enabling an already-enabled label succeeds and changes nothing — so a conditional adds a decision point without reducing work. The alternative, clearing the flag only after a bootstrap fails, has to infer the cause from launchd's error, but `5: Input/output error` covers several conditions; distinguishing them means parsing `print-disabled` output, which is a text format with no stability guarantee. More decisively, the retry shape preserves the asymmetry between the two backends — one treating enable as part of loading, the other as error recovery — which is the structural cause of this bug. A rarely-exercised retry path is also the more likely of the two to rot.

Position before `bootout`: the flag concerns the label, `bootout` concerns a registration, and the order makes the sequence read as "make this label runnable, clear what was there, load the new one". Either order works; this one matches how systemd's `load` orders its steps.

Treatment of failure: unlike `bootout`, whose failure is expected on a fresh host and is swallowed, `enable` failing is a real signal — on the system domain it needs root, which `install` has already verified. It therefore goes through `run_launchctl` and propagates, rather than being ignored.

**Refuse an install that conflicts with the *other* scope, not every running role.**

The check joins the existing pre-check sequence in `plan_install`, so it runs before any write and its failure carries no applied effects. It refuses only when the role is held at the lock tier belonging to the scope the install will *not* register in: `Tier::Machine` for a user install, `Tier::User` for a system install. A role held at the tier matching the registering scope does not refuse the install.

**The tier compared is the effective scope, not the requested one.** On a Linux host without systemd, the BusyBox backend has no per-user services: a user-scope request installs the same system-wide `/etc/init.d` script, with a note saying so. Its running service therefore holds the machine-wide tier, and comparing against the *requested* scope would refuse what is really a same-scope re-install — exactly the case the tier rule exists to protect. `plan_install` resolves the effective scope first, from the same condition `backend_install` uses to pick BusyBox over systemd, and compares the tier of that. Every other backend's effective scope equals the requested one.

That condition is a host probe with no seam — it reads `/run/systemd/system` and `/proc/1/exe` — so the decision is split: a pure function maps a requested scope plus a "this host has only system-wide services" flag to the effective scope, and its caller computes the flag. The rule is then testable on any host, which the backend probe itself is not.

Rationale for the asymmetry: re-installing over your own running service is a supported operation, and the specification already requires it to work — the systemd unit is restarted so the new registration takes effect, and a retarget to the same account succeeds. A blanket "refuse whenever the role runs" would break that, and would also change Windows behaviour, where only one scope exists and a cross-scope conflict is impossible. Refusing on tier mismatch confines the new refusal to exactly the situation that produces an unrunnable job.

The check asks about one tier, which neither existing entry point does: `ownership::ensure_not_running` refuses on either tier — correct for `service start`, too broad here — and `ownership::probe` returns only the first tier found, so a caller cannot ask about the other. A tier-scoped probe is added beside them. It returns the existing `Liveness`, not an owner record, because a held lock whose `<role>.owner.json` is missing or unreadable carries no record, and reading that absence as "not running" would permit exactly the install this check exists to refuse.

The refusal reuses `CliError::Conflict`, so the message still names the holding tier and the stop hint, and it additionally names the command that removes the service in the conflicting scope. One consequence: that variant's `Display` prefixes `start refused: `, which misreads on an install. The prefix belongs to the call sites that mean a start, so it moves there. This changes a user-visible string on every conflict path, so it needs the user's agreement before it is made; the specification does not require it, and the misleading prefix is survivable if they prefer to keep it.

Position within the sequence: after the scope decision and privilege checks, before the config and trust checks. A caller who asked for a scope they cannot use should hear about the privilege first, since that is a property of their invocation rather than of the machine's current state. So an unprivileged `--system` install reports the permission error even while the role runs.

Alternatives rejected during brainstorming: accepting the install with a warning leaves two registrations and the restart loop; prompting interactively adds an `InstallNeed` variant and a CLI prompt for a case whose answer is almost always "remove the other one".

**A foreground run in the same tier is not refused.** A foreground run holds the user tier, which a user install matches, so the install proceeds and the job will exit as "already running". Distinguishing a foreground process from the scope's own service within one tier means comparing the owner's pid against the pid the service manager reports, which is a new mechanism for a state the operator created knowingly and can see. It is left as a known limitation alongside the launchd restart gap.

**A system install probes the invoking user's tier, not root's.** The per-user lock directory is derived from a home directory, and a system install runs as root while registering a service that reads the invoking user's files. The tier probe must therefore resolve the per-user lock of the account the install resolved — the same account `system::resolve_target_account` produces for the pin, whose home comes from the passwd database rather than from `HOME` — rather than root's own. This matches the existing requirement that a system install never uses root's home.

The inverse case is accepted: a BusyBox user-scope request run as root registers system-wide, so the tier it compares is the per-user one, resolved from the calling process. There is no invoking-user account to resolve on that path, and the check is a guard against a cross-scope conflict rather than a general liveness audit.

**Keep the role's exit classification unchanged, and record the containment gap.**

The systemd unit carries `RestartPreventExitStatus=6`, exempting the role's "already running" status from restart. launchd's `KeepAlive` dictionary has no equivalent: `SuccessfulExit=false` means "respawn unless the last exit was clean", with no way to name an exception, and `ThrottleInterval` only slows a loop rather than stopping it. So on macOS a refused start is respawned indefinitely.

Refusing the conflicting install removes this from the path operators take. It remains reachable by invoking `launchctl bootstrap` directly, or by installing the system service after the user agent is already loaded. Closing it would mean exiting cleanly when the role refuses to start, which changes a contract shared by Linux and Windows — and makes "refused to start" indistinguishable from "ran and finished" to every caller — to contain a symptom specific to how one platform reads exit statuses. The gap is documented instead.

**Name an inspectable command in the load-failure note.**

`ServiceNote::LaunchdBootstrapHint` is rendered in `src/render.rs` as a hint to run `launchctl print <target>` or `launchctl bootstrap <domain> <plist>`. For the disabled-label cause, `launchctl print` answers that the service cannot be found in the domain — true, and a dead end, because it describes the consequence rather than the cause. The note's text must instead name what reaches the diagnosis for the causes an operator can act on, `launchctl print-disabled <domain>` among them.

Whether the note's fields change is left to implementation: `domain` and `plist` are already carried, and `print-disabled` needs only `domain`. The rendering lives in `src/render.rs` with the other notes and its test is beside the existing one.

**Correct the document-type declaration to `PropertyList-1.0.dtd`.**

No behaviour depends on it. It is corrected because it is wrong, and because a wrong declaration invites someone to conclude the plist is malformed while looking for a different bug — which is close to what happened here.

## Risks / Trade-offs

**`launchctl enable` on the system domain requires root → mitigated by ordering.** `plan_install` verifies privilege before the backend runs, so a non-privileged system install never reaches `load`. The command therefore only runs with the privilege it needs, and its failure is a genuine error worth surfacing.

**Clearing a disable an administrator set deliberately → accepted.** An administrator who disabled this label and then runs `service install` is asking for the service to be installed and running; the install is the more recent and more specific instruction. The same reasoning already governs `systemctl enable` on Linux.

**The install pre-check applies on every platform, not just macOS → accepted, and intended.** A cross-scope conflict was never going to produce a working service on any platform, so a macOS-only check would be a second divergence of the kind this change exists to remove. Because the check fires only on tier mismatch, it is unreachable on Windows, where only the user scope exists, and it leaves every same-scope re-install path on Linux and macOS as it was.

**An operator relying on install succeeding into a broken state → acceptable.** The previous "success" produced a job that could not run. A script that installs both scopes in sequence will now fail at the second install, which is the correct report.

**The restart-loop gap stays open for direct launchd use → documented.** Recorded in the modified spec so the asymmetry with systemd's restart containment is stated rather than discovered again. The spec states it as a property of the rendered plist, which a test can assert, rather than as launchd runtime behaviour, which no repository test can observe.

**A foreground run in the same tier still reaches the restart loop on macOS → documented.** The tier-mismatch check does not see it, for the reason given in the decisions. It is the same residual gap as direct `launchctl bootstrap` use, and is recorded the same way.

**Two behaviours cannot be tested in the repository.** A test that genuinely verified the disabled-label path would have to disable a label on the host running it, and no test can observe launchd respawning a job. The command sequence and the rendered plist are assertable through the `Native` fake, which is where the regression guards belong; the two end-to-end behaviours are verified by hand and recorded in this change's `verify.md`.

**The new tests must not reach for the machine's own state.** The per-user lock directory derives from `HOME` and the machine-wide one from a fixed path or a test-only environment variable gated behind the `test-utils` feature, which the crate's own `--lib` test run does not enable. Setting an environment variable is also process-global while cargo runs tests in parallel. So both tiers' lock directories are produced by an injected resolver, threaded through the install's private planning functions and called only at the conflict check, with the account home where one is known; production passes the real resolution and the tests pass temporary directories. The public `install` signature and the install request are unchanged.

This is not only for the new tests. Without the injection, the existing install tests would start probing the developer's real machine-wide lock path, so a developer with the system service running — precisely the state this change is about — would see unrelated tests fail, and the result would differ between a crate-level run and a workspace build that unifies the test-only feature.

## Migration Plan

No migration. No persisted format, configuration file or CLI surface changes, so an upgraded binary needs no operator action.

A host already in the broken state is repaired by re-running `service install`, which now clears the flag. A host carrying both a loaded user agent and a running system service is not repaired automatically: the next install refuses and names the service to remove, which is the intended report rather than a migration.

Rollback is reverting the binary. The one lasting effect on a machine is a cleared disable flag, which is the correct state for an installed service and is not undone by rolling back.

## Open Questions

None.
