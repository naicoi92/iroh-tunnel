# Redesign capability specifications — review draft

**Status:** capability map approved; the capability rules and acceptance criteria below are a review draft, not implementation approval. **Confirmed** means explicitly agreed during the interview; **Proposed** is a candidate design; **Deferred** needs a decision or evidence. See [capability map](capability-map.md), [decision record](project-redesign.md), and [diagrams](detailed-design.html). No database is implied. [Dependency graph, UML and story map](structural-views.html) provide capability and proposed structural views; the story-map release cut is not an approved rollout commitment.

## `tunnel-contract` — publication and mapping

**Actors / input:** operator defines Serve service name, target address and protocol; Access defines matching name/protocol, local bind and remote NodeId. TCP is the default. Both peers use Iroh identities.

| Rule | Status | Behavior |
|---|---|---|
| TC-01 | Confirmed | Serve exposes a configured fixed target; Access exposes a local endpoint for that remote service. |
| TC-02 | Confirmed | TCP and UDP are supported. Do not add a version-negotiation framework in this redesign. Seek TCP interoperability with 0.4 in both directions; if tests fail, present the trade-off to the user before choosing a breaking release. |
| TC-03 | Confirmed | A peer knowing NodeId and service name can attempt access; NodeId is public identity, not authorization. Target services protect themselves. |
| TC-04 | Proposed | Reject mismatched service name/protocol explicitly; discovery and diagnostics remain under review. |

**Path:** validate inputs → start role → Access resolves peer → establish compatible tunnel → route traffic to configured target. **Failures:** invalid input or unreachable target is reported; 0.4 interoperability is tested rather than assumed. Numeric retry defaults remain open. **Ownership:** service definitions belong to role configuration; remote references are not cross-host foreign keys. **Acceptance:** AC-TC-01 matching mapping forwards to fixed target; AC-TC-02 test TCP with 0.4 in both directions and present any incompatibility for user decision; AC-TC-03 no claim that NodeId grants authorization.

## `configuration` — config and identity

**Actors / trigger:** operator uses role config commands or configured run; quick-run flags are separate. **Inputs/defaults:** role, config path override, service definitions and identity. Quick run uses ephemeral identity unless supplied a persistent key.

| Rule | Status | Behavior |
|---|---|---|
| CFG-01 | Confirmed | macOS/Linux defaults are `~/.config/iroh-tunnel` and `~/.local/state/iroh-tunnel`, ignoring XDG directory overrides. Windows uses LocalAppData config/state directories. Explicit app overrides remain. |
| CFG-02 | Confirmed | If new macOS config exists, use it without looking at legacy. If absent and legacy exists, copy without overwriting, preserve original and secret permissions; copy failure is reported without creating a replacement identity. |
| CFG-03 | Confirmed | Ignore legacy `multiplex` silently; remove it on subsequent config writes. |
| CFG-04 | Confirmed (implemented in #99) | Every config mutation (`add`, `remove`, `keygen`, `edit`, and the key persisted by `run`) validates the resulting document before persisting, writes it atomically (temp file in the same directory, fsync, rename) with owner-only permissions, and serializes writers with an advisory lock. `edit` validates after the editor exits and rejects an invalid result. Every config command accepts `--config`. Lock timeout is 10 seconds; the Windows ACL equivalent is UNVERIFIED (#99). Decision record: `issue-98-config-identity-decisions.md` D4, D5. |
| CFG-05 | Confirmed (implemented: `keygen random` in #98, rest in #99) | `config keygen` asks for confirmation before replacing an existing key (`--yes` skips; non-interactive without it fails and does not rotate). `config show` redacts `secret_key` and `relay_token` unless `--reveal`. `run`, `list` and `show` with no configuration fail with one guidance error and create no file or identity. `config keygen random` prints a fresh secret key and node ID and writes nothing. Still deferred: exact legacy path, migration synchronization. Decision record: D1, D2, D3, D6. |

**Path:** resolve explicit path or default → choose existing new config / safely copy legacy when applicable → load and validate → manage or run. **Branches:** missing both configurations is a guidance error for read commands (CFG-05); unreadable file, copy failure or unusable home returns actionable error. **Persistence:** config and persistent identity survive process restarts; ephemeral quick-run identity does not require a config write. **Concurrency:** no overwrite on concurrent migration; config writers serialize through an advisory lock (CFG-04); lock mechanism per platform is deferred to #99. **Acceptance:** AC-CFG-01 new config wins; AC-CFG-02 copy never replaces destination and preserves source; AC-CFG-03 failure does not generate identity; AC-CFG-04 writing drops old multiplex field.

## `runtime-ownership` — instance and status

**Actors / input:** either binary for a role starts foreground or as service, with resolved user/scope and state path. **Invariant RO-01 (Confirmed; revised by #77):** a role runs as one process at a time. At start, the process checks for a running process of the same role; if one is detected, the start is refused with a reason, regardless of how either process was started and regardless of binary name (`it serve` and `its` must not bypass each other). Serve and Access are separate roles.

**RO-02 (Confirmed; revised by #77):** a non-elevated user must be able to detect a running system service of the role and then may not run that role. Independent endpoints on the same host must not concurrently use the same public identity, even across roles; reject the later process when this is detected. Detecting a copied key used by another OS user is out of scope; report a node-id conflict only if it can be detected simply. Do not implement cross-host intervention. Do not store raw secrets in ownership artifacts.

**RO-03 (Confirmed, #77):** keep the 0.4.2 lifecycle: a foreground run stops with Ctrl+C and a service stops with its service stop command; add no stop command and no permission model. The only new behavior is that a start is refused, with a reason, while a process of the same role is detected. A system service installs and runs as root/Administrator, and whoever installs it owns it (SM-02 retarget confirmation still applies). While the system service of a role runs, `run` and `service start` for that role are refused for any OS user; to use a user service or a foreground run, the operator first stops the system service. Nothing is stopped automatically. A leftover file after a crash must not block the next start. The detection mechanism is an implementation detail of #87.

**Path (Proposed):** resolve role → acquire ownership → start role → publish status snapshot → stop and clean owned state. **Failure:** conflict refuses second instance; unusable state path or permissions fails with guidance. **Deferred to #87 (implementation detail):** lock implementation, stale PID detection and crash cleanup under RO-03. **Output:** runtime state and per-role status; do not delete another process's status. **Acceptance:** AC-RO-01 competing binary/role pair cannot both run for the same role (system service, or same-user user service or foreground); AC-RO-02 different roles may run when they use different public identities (same identity across roles is rejected, RO-02); AC-RO-03 status identifies role and owner and does not silently report an unrelated instance. Stale-status display follows AC-RO-05 and AC-RO-06; the mechanism is decided in #87.

## `tcp-forwarding` — multiplexed TCP

**Actors / input:** TCP clients, Access, Serve and fixed target. **Rule TCP-01 (Confirmed):** multiplex is always active; one reused service connection carries a separate bidirectional stream per local TCP connection. **Rule TCP-02 (Confirmed):** target failure closes only the affected channel; peer loss closes affected channels without replaying bytes. New clients may retry with backoff until a default bounded deadline. On shutdown, stop accepting new clients, drain active channels for a bounded period, then close remaining channels. Numeric defaults and configurability remain open.

**Path:** Access listens → client connects → obtain/reuse compatible peer connection → open channel → Serve connects fixed target → relay bidirectional bytes → close channel. **Failures:** target connection failure affects that channel; peer loss affects its shared channels without replay. New clients retry with backoff under a bounded default deadline. Shutdown stops accepting clients, then drains active channels for a bounded period. Numeric defaults, exact client-visible errors and flow-control limits remain open. **Concurrency:** independent client stream ownership; shared connection lifecycle must not cross role/scope ownership boundaries. **Acceptance:** AC-TCP-01 two clients transfer independently over reused connection; AC-TCP-02 incompatible peer is rejected; AC-TCP-03 no `multiplex` setting disables reuse. AC-TCP-04 isolated channel failure is proposed pending confirmation.

## `udp-forwarding` — datagram sessions

**Actors / input:** multiple UDP clients, local listener, Access, Serve and fixed UDP target. **UDP-01 (Confirmed):** preserve datagram boundaries; best-effort delivery permits loss, reordering and duplication. **UDP-02 (Confirmed):** automatically distinguish client sources and route responses to the corresponding client.

**UDP-03 (Confirmed):** Access associates a client endpoint with a runtime UDP flow; Serve uses a dedicated upstream UDP socket per active flow. The application payload remains opaque and unchanged; framing inside the tunnel is permitted. Flows may share an Iroh connection. A socket keeps its local endpoint for its lifetime, not necessarily across expiry or reconnect; the target generally sees the Serve-side endpoint, not the original client IP. This runtime association is distinct from persisted `AccessMapping` configuration.

**Path:** receive datagram and source → associate logical client flow → convey to Serve → forward to fixed target → route response to same flow and source. **Failures/resources (Deferred):** QUIC DATAGRAM versus other transport, fragmentation, payload limit, mapping lifetime, congestion/drop reporting, memory and abuse limits; benchmark Iroh 1.4.1 direct and relay paths before selection. No reliable ordered fallback is silently assumed. **Acceptance:** AC-UDP-01 concurrent clients receive their own responses; AC-UDP-02 datagram boundaries survive; AC-UDP-03 no guarantee of ordered/reliable delivery; AC-UDP-04 resource limits and MTU behavior are specified and tested before release.

## `service-management` — background operations

**Actors / input:** installing user, optional `--system`, role, config/state paths, OS service adapter. **SM-01 (Confirmed):** non-elevated user chooses user/system; root or elevated Administrator defaults system without scope prompt. If system is chosen without privilege, stop and instruct rerun using sudo or an elevated Windows terminal; no automatic elevation/fallback. Preserve intended paths and arguments.

**SM-05 (Confirmed):** the user may edit managed configuration, and an authorized administrator may restart the system service with sudo to apply it. Validate the configuration and report errors; never silently change ownership, permissions or runtime account. Do not treat an authorized edit as an integrity violation. Protection against unauthorized writers and unsafe path resolution remains open.

**SM-02 (Confirmed):** pin absolute config/state paths of the installing user, even for system service. Confirm before retargeting an existing service belonging to another user; keep both config files. Unknown user → interactive selection; non-interactive → fail with guidance. **SM-03 (Confirmed):** missing permissions produce actionable failure, never silently broaden access.

**SM-04 (Proposed, #79; awaiting owner review on #79):** RO-03 sets the system service runtime account to root/Administrator; the user may edit managed config and an authorized administrator restarts to apply it (SM-05).

- **Trusted principals.** root, plus the owner UID of the pinned config directory recorded at install time (the installing user, or the user chosen at the SM-02 retarget/unknown-user prompt; `SUDO_USER` is only an input to that choice). A different owner is refused.
- **Check (Linux/macOS system scope).** Descend from `/` with no-follow, handle-relative opens (`openat`-style), checking owner and mode of each directory handle; open the config with no-follow and check the opened handle; parse only from that handle. Refuse (fail closed) when any untrusted principal can write to the file or a parent, naming path, owner and mode. Never chown/chmod. Group-writable is refused by default.
- **When.** At every start and restart; any re-read re-runs the check. There is no hot reload.
- **Other root-written user paths.** The same descent and checks apply to the state directory, status snapshot, lock and key paths, because the root service writes and deletes there (RO-03). If a path is deliberately trusted instead, the reason is recorded here.
- **Prior art.** sshd `StrictModes` and sudoers fail closed on owner/mode, but neither is documented to walk every parent directory; this walk is new design.
- **Windows scope.** The Windows user-background task does not cross a privilege boundary, so the DACL, reparse-point (symlink, junction, mount point) and `GetFinalPathNameByHandleW` rules are deferred together with Windows system scope (**Deferred**); `--system` on Windows keeps reporting unsupported.

*Windows user-background (Proposed):* a Task Scheduler task for the current user (logon trigger, interactive token) with the shared `RestartPolicy`, managed by `schtasks` run/end/query and the existing status snapshot. That a non-admin user can create, query and delete such a task without a password is unverified and is the first measurement in #92. Per RO-03 stop is `schtasks /End`; if measurement on real Windows shows it terminates without drain, record the result as a known limitation rather than adding a stop command or control channel. Console window: prefer no visible window; the task `Hidden` setting only hides the task, and hiding the console may prevent Ctrl events, so the trade-off is decided by measurement in #92. Prior art: `docs/design/issue-79-prior-art-research.md`.

**Operations:** install resolves scope and paths → checks privilege and conflicts → asks necessary confirmation → registers adapter. Start/stop/restart/status/uninstall operate on role/scope; exact absent-service and idempotency behavior is proposed, not confirmed. **Acceptance:** AC-SM-01 unprivileged system install stops with rerun guidance; AC-SM-02 installed service uses the selected absolute paths; AC-SM-03 retarget requires confirmation and preserves files; AC-SM-04 trust-boundary policy and Windows user mechanism are reviewed before release; config and state trust is exercised on real Linux and macOS accounts with an unauthorized writer, parent-directory replacement, symlink and race attempts, and admin-group and sticky-bit parent directories; on real Windows, install/start/status/stop/uninstall, `/End` drain behavior and the recorded limitation, and non-admin task creation are measured.

## `distribution` — install, update and releases

**Actors / input:** release CI, operator, installer, standalone updater, package manager and container runtime. **DIST-01 (Confirmed):** three standalone binaries and three role/all-in-one images; installers default to all three system-wide. Update each binary independently; separate packages allow package-manager updates. Containers are updated by replacing the image.

**DIST-02 (Confirmed):** stable latest is default, explicit version allowed. Verify keyless CI identity/signature and checksum; failure preserves old binary. A deliberate unsafe bypass warns; exact checksum-bypass semantics are deferred. Confirm before PATH changes or conflicting file replacement. Do not automatically stop a running tunnel; give operator guidance. Replacement and rollback remain open. **DIST-03 (Deferred):** signing identity rotation, rollback, alias update behavior, atomic replacement on Windows, package detection and partial failures.

**Decision table:** standalone → download/verify → handle running process with consent/guidance → replace selected binary; package-owned → delegate to corresponding package; container → instruct pull/redeploy. Legacy bundled package → guide migration rather than update unrelated binaries. **Acceptance:** AC-DIST-01 default installer selects all three; AC-DIST-02 update only changes selected binary/package; AC-DIST-03 failed verification retains old binary unless explicit bypass; AC-DIST-04 container path does not self-update; AC-DIST-05 system-wide install confirms conflicts and PATH changes.

## `cli-experience` — discoverable role interface

**Actors / input:** user invokes `it serve`, `it access`, `its`, or `ita`, with quick-run flags or separate configured `run`. **CLI-01 (Confirmed):** role binaries are independently runnable and all-in-one has both roles. **CLI-02 (Confirmed):** same conceptual options have consistent non-conflicting short arguments and complete help; TCP is default. **CLI-03 (Deferred):** exact argument mapping, diagnostic messages, short-option conflicts and exit-code contract require design.

**Path:** parse → select role/mode → validate → dispatch to appropriate capability; `update` acts on the invoked binary. **Failure:** invalid inputs show meaningful diagnostics/help; no hidden persistent write for ephemeral quick run (proposed pending review). **Acceptance:** AC-CLI-01 equivalent role forms dispatch consistently; AC-CLI-02 each binary runs independently; AC-CLI-03 help lists args and defaults; AC-CLI-04 `--system` unprivileged branch gives elevation guidance.

## Scenario decision tables — proposals for review

These scenarios do not change the confirmed rules above. Proposed outcomes require user review before implementation.

### Ownership and status

| Scenario | Proposed outcome | Acceptance candidate |
|---|---|---|
| `it serve` and `its` start for the same role (system service running, or same user) | Second process fails with a reason, independent of executable name. | AC-RO-04: concurrent starts admit at most one owner. |
| Process crashes and leaves status | Treat the snapshot as potentially stale; do not silently claim live. A leftover file must not block the next start (RO-03). | AC-RO-05: crash snapshot is never presented as verified live without a successful ownership/liveness check. |
| OS reuses PID | PID alone is insufficient proof; include a process incarnation/ownership token or equivalent mechanism. | AC-RO-06: reused PID cannot authorize deletion or ownership takeover of another process's state. |

### Forwarding failures and concurrency

| Scenario | Proposed outcome | Acceptance candidate |
|---|---|---|
| TCP target unavailable | Confirmed: fail the affected channel; keep unrelated healthy channels. Diagnostic details remain open. | AC-TCP-05: one target failure does not close unrelated healthy channels. |
| Peer disappears with multiple TCP streams | Confirmed: affected channels fail without replay; new clients retry with backoff bounded by a default deadline. Numeric defaults remain open. | AC-TCP-06: clients get closure/error and no duplicated replay is claimed. |
| UDP responses for A/B arrive interleaved | Route each datagram using its logical flow association; never cross-deliver. | AC-UDP-05: interleaved A/B responses reach the original client. |
| UDP flow expires or datagram exceeds supported size | Drop/report according to a bounded policy; timeout, limits and reporting are deferred pending benchmarks. | AC-UDP-06: behavior is documented and bounded before release; no partial datagram is delivered. |

### Configuration and service lifecycle

| Scenario | Proposed outcome | Acceptance candidate |
|---|---|---|
| Invalid config mutation | Reject before persisting; keep prior valid file and identity. | AC-CFG-05: validation failure leaves config unchanged. |
| Concurrent config writes | Serialize through an advisory lock; no corruption or silent lost update (CFG-04). | AC-CFG-06: two writers do not produce corrupted or silently lost changes. |
| Key rotation or config display | Rotation changes peer identity and needs confirmation (`--yes` to skip); `show` redacts secrets unless `--reveal` (CFG-05). | AC-CFG-07a/07b: approved policy is tested for secret disclosure and rotation before release. |
| Install service already targeting another user's config | Confirm retarget; decline leaves service and both config files unchanged. | AC-SM-05: declining confirmation produces no retarget. |
| Missing privilege / unknown invoking user | Stop with elevation instructions; when user is unknown, select interactively or fail with explicit paths in non-interactive mode. | AC-SM-06: no silent root-home substitution. |

### Update and artifact ownership

| Scenario | Proposed outcome | Acceptance candidate |
|---|---|---|
| Standalone binary is running | Confirmed: do not automatically stop the tunnel; guide the operator. Replacement and rollback mechanics remain open. | AC-DIST-06: no unconfirmed interruption. |
| Package or container owns binary | Delegate only corresponding package, or instruct image pull/redeploy; do not independently overwrite managed file. | AC-DIST-07: no package/container ownership bypass. |
| Alias points at target binary | Preserve alias relationship while replacing only selected binary; alias mechanism differs by OS and remains to be specified. | AC-DIST-08: an approved alias strategy leaves unrelated binaries unchanged. |
| Signature/checksum verification fails | Preserve installed executable unless user invokes explicit warned bypass; checksum bypass semantics remain open. | AC-DIST-09: verification failure cannot replace a binary silently. |

## Cross-capability release gates

1. Resolve and approve the per-capability specifications, security rules and data-model cardinalities needed by each dependent implementation. Unrelated open decisions do not block independent slices; unresolved release requirements remain release gates.
2. E2E verification across TCP, UDP, config migration, service lifecycle, installers and updates, including actual Windows runners; tests must trace to acceptance criteria.
3. Benchmark UDP and establish memory, payload, timeout and loss behavior; document protocol compatibility and upgrade guidance.
4. Verify secret protection and the privileged service/config trust boundary; define crash cleanup, rollback and signing trust rotation.
5. Maintain current versus target distinction in [architecture delta](detailed-design.html#delta); Kanban cards are planned work, not completed implementation.
