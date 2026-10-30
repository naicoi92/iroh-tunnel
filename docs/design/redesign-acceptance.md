# Redesign acceptance and evidence ledger

Planning revision against tracker snapshots dated 2026-10-29 and documentation baseline `02c1d11c69227e9d7acfbe2448af2001471f7428`. This ledger defines executable acceptance procedures, not tests already implemented or execution evidence. User confirms real process E2E and mandatory short arguments. [Traceability](redesign-traceability.md) separates requirement IDs (`S01`–`S72` and confirmed rules) from scenario IDs below. Live issue decisions/native blockers supersede snapshots; no proposal is approved by being cataloged.

## Acceptance contract and evidence format

Before implementation, use the current row in [traceability](redesign-traceability.md#current-story-ownership-and-evidence) and its cases as the AC-to-E2E plan. Each case has a stable explicit anchor, exact source checkbox locator, prerequisites, action, observable expected outcome, owner, platform, evidence and execution status. These are runnable procedures for an acceptance harness/operator, not a claim that a test script exists. Policy-gated envelopes must gain approved detailed assertions before execution; only their already confirmed outcomes are fixed here.

All runtime cases spawn final executable processes and actual clients/targets; service/install/update cases additionally exercise real OS, filesystem and manager boundaries. In-process helpers/unit tests may support fault injection, never substitute for relevant process/platform acceptance. Documentation assertions such as SEC-NODEID-01 inspect the published contract; they do not invent tunnel authorization.

Every execution record must include case ID and parameter variant, requirement IDs, source AC, owner, exact source SHA, lockfile/dependency version, artifact digest, OS/architecture, user/privilege, actual command and arguments, fixture/version, selected network path when relevant, timestamp, expected versus observed result, logs/data/files and independent reviewer disposition. Never record raw secret keys or relay tokens. Record actual connection/path observations where a successful byte exchange alone cannot prove the assertion.

Planning coverage is `MAPPED` (owned procedure/source exists), not implementation or release PASS. Execution is `NOT_RUN` (no accepted run evidence), `BLOCKED` only with stated reason, `PASS` or `FAIL` after observed evidence and independent review. Platform unavailability requires a reason and closest alternative, never PASS. A blocked variant does not block unrelated cases. Changes to source, dependency or artifact invalidate affected evidence. All cases feed [RELEASE-GATE-01](#release-gate-01), final owner #97; native blockers remain authoritative.

## Executable scenario catalog

Shared prerequisites: immutable final artifacts, isolated disposable environments, real peer/target fixtures and evidence format above. Each case adds its own requirements. “Supported matrix” means preserved released platforms plus Windows x64/ARM64; applicable package/service combinations must be enumerated, cross-build and runtime separated, unavailable variants disclosed. Exact short-option letters and policy mechanisms remain unapproved where not decided; short-option support itself is mandatory.

<a id="cli-quick-01"></a>
### Case CLI-QUICK-01

**Requirements:** S06, S10, S14, S15, S18, TC-01, CLI-01, CLI-02. **Owner:** #81; final integration #97. **Planning:** MAPPED.

**Source AC:** [81.AC1](#81-ac1); [81.CL1](#81-cl1). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Final Serve/Access processes, fixed TCP target and client; isolated user paths.

**Action:** Start both roles with quick flags and matching name/peer/local bind; omit protocol. Send distinct request and response bytes. Repeat equivalent long/short quick options.

**Expected observable outcome:** Local client reaches the named fixed target; exact bytes flow both ways; omitted protocol is TCP; long/short forms reach the same target.

**Evidence/platform:** Commands, listener/target addresses, sent/received bytes, role output and selected path. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="cli-error-01"></a>
### Case CLI-ERROR-01

**Requirements:** S11, CLI-02. **Owner:** #81; final integration #97. **Planning:** MAPPED.

**Source AC:** [81.AC2](#81-ac2); [81.CL1](#81-cl1); [81.CL2](#81-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Existing config/key hashes; separate invalid address, port, peer and option fixtures for each role.

**Action:** Invoke each invalid quick command and mixed quick flags with configured subcommand; inspect listeners and files.

**Expected observable outcome:** Commands reject invalid/mixed input with actionable role-specific diagnostics, not silently ignored flags; user config/key material unchanged. No new numeric exit-code contract.

**Evidence/platform:** Exit/output, listener probes, before/after file hashes. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="identity-quick-01"></a>
### Case IDENTITY-QUICK-01

**Requirements:** S12. **Owner:** #81; final integration #97. **Planning:** MAPPED.

**Source AC:** [81.AC2](#81-ac2); [81.CL2](#81-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** No supplied key; separate isolated paths for both quick roles.

**Action:** Start/stop/restart Serve and Access without keys and record public identities and filesystem changes.

**Expected observable outcome:** Both roles use default ephemeral identities observable across restarts, not a replacement persistent identity; preserve existing user config/key material.

**Evidence/platform:** Public IDs, process commands and filesystem diff; redact secrets. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="sec-nodeid-01"></a>
### Case SEC-NODEID-01

**Requirements:** S17, TC-03. **Owner:** #81; final integration #97. **Planning:** MAPPED.

**Source AC:** [81.CL3](#81-cl3). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Final operator/help/security documentation and quick-run artifacts.

**Action:** Inspect NodeId/service-name explanations and target-protection instructions alongside an actual quick-run session.

**Expected observable outcome:** Documentation explicitly says NodeId is public identity, not target authorization; target services enforce their own protection. No secrecy-based access claim or new tunnel authentication scheme.

**Evidence/platform:** Exact documentation/help excerpts and artifact version; documentation assertion, not fabricated ACL test. Platform: Published documentation plus supported runtime quick-run sample.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="cli-independent-01"></a>
### Case CLI-INDEPENDENT-01

**Requirements:** S01, S02, S03, S05, CLI-01, DIST-01. **Owner:** #82; final integration #97. **Planning:** MAPPED.

**Source AC:** [82.AC1](#82-ac1); [82.CL1](#82-cl1). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Each artifact alone in isolated directory/PATH; peer fixture on separate host/environment.

**Action:** Spawn all-in-one Serve and Access separately, then each standalone role, without other role binaries installed; transfer real client/target bytes.

**Expected observable outcome:** Each supported role runs and forwards independently, with no runtime dependency on another installed executable.

**Evidence/platform:** Directory/PATH listing, commands/process IDs, versions and byte transcripts. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="cli-alias-01"></a>
### Case CLI-ALIAS-01

**Requirements:** S04, CLI-01, DIST-01. **Owner:** #82; final integration #97. **Planning:** MAPPED.

**Source AC:** [82.CL1](#82-cl1); [82.CL2](#82-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Installed agreed aliases and isolated role environments with independent peer/target.

**Action:** Invoke it serve, it access, its and ita through their installed aliases and exchange target/client bytes; inspect resolved artifact.

**Expected observable outcome:** Every alias resolves and runs its intended role; alias invocation itself forwards, not merely --help or file existence. Alias mechanism is not prescribed.

**Evidence/platform:** Alias resolution, invoked/resolved digests and process/byte transcripts. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="cli-short-01"></a>
### Case CLI-SHORT-01

**Requirements:** S08, CLI-02. **Owner:** #82; final integration #97. **Planning:** MAPPED.

**Source AC:** [82.AC2](#82-ac2); [82.CL2](#82-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** All three binaries and both all-in-one roles; documented supported option map.

**Action:** For every equivalent conceptual option invoke long and short forms in real forwarding processes, including supplied-key forms and TCP default; probe conflicting options.

**Expected observable outcome:** Equivalent options have consistent, non-conflicting meanings and forwarding behavior across roles; short-option support and real-process forwarding coverage are mandatory. Record implemented letters, do not approve suggested letters as new policy.

**Evidence/platform:** Long/short command matrix, bytes and public IDs; conflict output. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="cli-help-01"></a>
### Case CLI-HELP-01

**Requirements:** S09, S11, CLI-02. **Owner:** #82; final integration #97. **Planning:** MAPPED.

**Source AC:** [82.AC2](#82-ac2); [82.CL2](#82-cl2); [82.CL3](#82-cl3). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** All three final executables and aliases.

**Action:** Capture help/defaults per supported role; invoke unsupported role-specific options.

**Expected observable outcome:** Help exposes supported options, aliases and defaults; role-specific invalid options produce useful diagnostics; no assertion of unapproved exact message/exit code.

**Evidence/platform:** Help snapshots, option matrix, exits and stderr. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="identity-key-01"></a>
### Case IDENTITY-KEY-01

**Requirements:** S13. **Owner:** #82; final integration #97. **Planning:** MAPPED.

**Source AC:** [82.AC1](#82-ac1); [82.CL2](#82-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Secure supplied persistent key fixture for each supported role form.

**Action:** Run forwarding, restart with same supplied key and reconnect actual peer/client.

**Expected observable outcome:** Public identity remains stable across restart and supplied-key quick-run works for all supported role forms.

**Evidence/platform:** Redacted commands, public IDs and bytes; never publish raw keys. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="cfg-run-01"></a>
### Case CFG-RUN-01

**Requirements:** S07, TC-01, CLI-01. **Owner:** #83; final integration #97. **Planning:** MAPPED.

**Source AC:** [83.AC1](#83-ac1). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Existing valid role configs with known public identities and fixed target.

**Action:** Start real configured Serve/Access run commands, transfer bytes, restart and repeat.

**Expected observable outcome:** Configured settings select intended peer/service/target and forwarding works after reload.

**Evidence/platform:** Commands, redacted configs, public IDs and byte transcripts. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="cfg-multiplex-01"></a>
### Case CFG-MULTIPLEX-01

**Requirements:** S20, S21, S22, CFG-03, TCP-01. **Owner:** #83; final integration #97. **Planning:** MAPPED.

**Source AC:** [83.AC2](#83-ac2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Legacy true/false/omitted multiplex configs; observable shared connection and two clients.

**Action:** Load each variant; run overlapping clients; invoke actual config save, inspect output file and restart.

**Expected observable outcome:** Legacy values silently ignored; no setting disables multiplex; save omits field while preserving valid settings/key. Concurrent streams reuse service connection.

**Evidence/platform:** Logs showing no legacy warning, connection IDs/count, bytes, persisted file diff and restart. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="cfg-path-01"></a>
### Case CFG-PATH-01

**Requirements:** S33, S34, S35, CFG-01. **Owner:** #84; final integration #97. **Planning:** MAPPED.

**Source AC:** [84.AC1](#84-ac1); [84.CL1](#84-cl1). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Real OS user profiles, alternate XDG values, Windows known-folder location and explicit app overrides.

**Action:** Run actual config/state consumers with defaults, then overrides; inspect resolved paths/files.

**Expected observable outcome:** Linux/macOS use ~/.config/iroh-tunnel and ~/.local/state/iroh-tunnel ignoring XDG; Windows resolves LocalAppData/iroh-tunnel/config and state; explicit app overrides work.

**Evidence/platform:** Resolved known-folder/path evidence, commands/environment and file/state locations. Platform: Linux, macOS and Windows real account environments.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="cfg-migrate-01"></a>
### Case CFG-MIGRATE-01

**Requirements:** S36, S37, S38, CFG-02. **Owner:** #84; final integration #97. **Planning:** MAPPED.

**Source AC:** [84.AC2](#84-ac2); [84.CL2](#84-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** macOS with verified legacy location; distinct new/legacy identities and protected secret permissions.

**Action:** With new config present make legacy unreadable and run. Then use legacy-only fixture, run migration, restart; race concurrent migrations against destination creation.

**Expected observable outcome:** New config wins without consulting legacy; copy never overwrites destination, preserves source and secret permissions and stable identity, including races. No locking mechanism mandated.

**Evidence/platform:** Source/destination hashes, permission metadata, public IDs, commands/race timing. Platform: macOS real filesystem/permissions.

**Execution:** BLOCKED — Exact legacy path verification and migration synchronization evidence remain open (#84, #98); no-overwrite outcome already confirmed.

<a id="cfg-error-01"></a>
### Case CFG-ERROR-01

**Requirements:** S39, S40, CFG-01, CFG-02. **Owner:** #84; final integration #97. **Planning:** MAPPED.

**Source AC:** [84.AC2](#84-ac2); [84.CL2](#84-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Invalid/unreadable config, unusable home and forced copy-failure fixtures; known original identity.

**Action:** Invoke applicable real configured commands and macOS migration failures; inspect all candidate user paths.

**Expected observable outcome:** Actionable guidance; no silent other-user config fallback or replacement identity after migration failure; source preserved. First-time missing-both behavior not selected.

**Evidence/platform:** Errors/exits, file hashes/permissions and public-identity evidence. Platform: Linux/macOS/Windows config errors; macOS copy failure.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="tcp-compat-01"></a>
### Case TCP-COMPAT-01

**Requirements:** S16, TC-02. **Owner:** #85; final integration #97. **Planning:** MAPPED.

**Source AC:** [85.AC1](#85-ac1); [85.AC2](#85-ac2); [85.AC3](#85-ac3). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Authentic 0.4 artifacts with hashes and final redesigned artifacts.

**Action:** Run 0.4 Access → new Serve and new Access → 0.4 Serve against real TCP target; exercise other supported peer pairs. Review protocol delta for added negotiation.

**Expected observable outcome:** Record exact bytes/outcome in both directions. Any failure produces compatibility trade-off and user decision before breaking release; no new version-negotiation framework. Failure is not silently PASS or unconditional product rejection.

**Evidence/platform:** Old/new versions/digests, commands, pair matrix, logs/bytes and decision link if incompatible. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="tcp-shared-01"></a>
### Case TCP-SHARED-01

**Requirements:** S18, S19, S20, TCP-01. **Owner:** #86; final integration #97. **Planning:** MAPPED.

**Source AC:** [86.AC1](#86-ac1). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Real roles/target, observable service connection identity and at least two simultaneous clients.

**Action:** Send distinct bidirectional streams with overlap and close one client while other transfers.

**Expected observable outcome:** Independent client streams share one reused service connection; byte delivery is correct and remaining client continues.

**Evidence/platform:** Connection ID/count and stream observations, client/target byte records and overlap timings. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="tcp-failure-01"></a>
### Case TCP-FAILURE-01

**Requirements:** TCP-02. **Owner:** #86; final integration #97. **Planning:** MAPPED.

**Source AC:** [86.AC2](#86-ac2); [86.HANDOFF](#86-handoff). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Live healthy channels, fail-capable target, peer kill/restart, measured implementation default retry deadline/backoff.

**Action:** Fail one target channel; kill peer during marked transfer; start clients while unavailable and after recovery.

**Expected observable outcome:** Target failure closes only affected channel; peer loss closes affected channels without replay; new clients retry with backoff until bounded default deadline then fail, or succeed when peer returns.

**Evidence/platform:** Per-channel closures/bytes, no-replay markers, retry timestamps and measured default; no invented numeric limit. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="tcp-drain-01"></a>
### Case TCP-DRAIN-01

**Requirements:** TCP-02. **Owner:** #86; final integration #97. **Planning:** MAPPED.

**Source AC:** [86.AC2](#86-ac2); [86.HANDOFF](#86-handoff). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Active completing and held-open clients; measured implementation drain default.

**Action:** Request shutdown; attempt new client; let one stream finish within drain and keep another beyond it; exercise cancellation/repeated stop.

**Expected observable outcome:** Stop accepting new clients, permit bounded active drain then close remaining channels; record cancellation/repeated-stop/error mapping as technical contract, not invented policy.

**Evidence/platform:** Accept/close timestamps, client outcomes and selected default evidence; do not enshrine illustrative 30s/5s. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="own-role-01"></a>
### Case OWN-ROLE-01

**Requirements:** S41, S42, RO-01, RO-02. **Owner:** #87; final integration #97. **Planning:** MAPPED.

**Source AC:** [87.AC1](#87-ac1); [87.CL1](#87-cl1). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Real all-in-one/role binaries, distinct identities, foreground and user-service fixtures.

**Action:** Barrier-race same-role starts across executable names and path overrides; repeat foreground vs user service and each vs a running system service (`run` and `service start` refused until the system service is stopped); run distinct roles with different identities.

**Expected observable outcome:** At most one process per role among the system service and same-user modes regardless of executable/config/state paths; refusal names the process to stop and nothing is stopped automatically; different roles coexist with distinct identities.

**Evidence/platform:** Race timing/process outcomes, paths, user/scope and simultaneous byte exchanges. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** Ready after #77 closes (final rule recorded); user-service combination also needs the applicable service adapter.

<a id="own-identity-01"></a>
### Case OWN-IDENTITY-01

**Requirements:** RO-02. **Owner:** #87; final integration #97. **Planning:** MAPPED.

**Source AC:** [87.CL2](#87-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Same public identity on independent local endpoints, different roles/scopes/users where applicable.

**Action:** Start first then second with same identity across roles/user-system scope; inspect ownership artifacts/status/logs; repeat distinct keys.

**Expected observable outcome:** Later local duplicate rejected with guidance; no raw secrets exposed; different keys do not create identity conflict. No cross-host enforcement claimed.

**Evidence/platform:** Public IDs only, role/scope/process results, redaction scan and real OS permission evidence. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** Ready after #77 closes; no cross-user coordination is required beyond the system service.

<a id="own-status-01"></a>
### Case OWN-STATUS-01

**Requirements:** S43, RO-01. **Owner:** #87; final integration #97. **Planning:** MAPPED.

**Source AC:** [87.AC2](#87-ac2); [87.CL3](#87-cl3). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Actual distinct running role owners and status commands.

**Action:** Query each role status before/after controlled stop; compare with observed process owner.

**Expected observable outcome:** Status identifies actual role and owner, not unrelated instance. Stale/crash outcomes tested separately under approved policy.

**Evidence/platform:** Status output, process/user identity and state path correlation. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** Ready after #77 closes (final rule recorded).

<a id="own-recovery-01"></a>
### Case OWN-RECOVERY-01

**Requirements:** S44, RO-01, RO-02. **Owner:** #87; final integration #97. **Planning:** MAPPED.

**Source AC:** [87.AC2](#87-ac2); [87.CL3](#87-cl3). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Real processes that can be killed and restarted; the #77 final rule.

**Action:** Kill a running role abruptly and restart it (a leftover file must not block the start); leave stale snapshots and simulate PID reuse; compare live owner state.

**Expected observable outcome:** Observed stale/recovery/cleanup/security outcomes match approved #77 policy; no deletion of unrelated live owner state. Exact liveness tokens or cleanup algorithm not selected here.

**Evidence/platform:** Approved policy link, process/event trace, state ownership and permission diffs. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** Ready after #77 closes; the mechanism is decided in #87.

<a id="udp-data-01"></a>
### Case UDP-DATA-01

**Requirements:** S23, S24, S25, TC-01, TC-02, UDP-01, UDP-02, UDP-03. **Owner:** #88; final integration #97. **Planning:** MAPPED.

**Source AC:** [88.AC1](#88-ac1); [88.CL2](#88-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Approved #78 transport contract, real roles and UDP clients sharing one Access peer plus clients on two Access peers.

**Action:** Send distinct opaque binary/empty/variable datagrams to fixed target; deliberately interleave replies, including equal local flow numbers across peers where observable.

**Expected observable outcome:** Delivered payload and boundaries unchanged; replies reach correct peer and original client, never another active flow. No application parsing or reliability/order/deduplication guarantee.

**Evidence/platform:** Client/target packet payload/length/recipient traces, public peer IDs and path proof. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — UDP design/benchmark/implementation deferred; native #78 awaits #71/#73/#74 contract.

<a id="udp-socket-01"></a>
### Case UDP-SOCKET-01

**Requirements:** S26, S27, S28, S29, UDP-01, UDP-03. **Owner:** #88; final integration #97. **Planning:** MAPPED.

**Source AC:** [88.AC2](#88-ac2); [88.CL1](#88-cl1). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Approved UDP run path; target records simultaneous source endpoints; final user docs.

**Action:** Run multiple flows, inspect Serve upstream sockets and target source endpoints; inspect docs with NAT fixture where applicable.

**Expected observable outcome:** Dedicated upstream socket per active flow; endpoint stable for socket lifetime only. Flows may share Iroh connection, not required to do so. Docs state best-effort loss/reordering/duplication and Serve-side/NAT source, not original client IP or reconnect stability. Runtime flow association is not persisted AccessMapping.

**Evidence/platform:** Socket/packet observations, actual selected connection strategy and exact documentation excerpts. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Native #78 transport/lifecycle contract unresolved and UDP work deferred; confirmed socket baseline not reopened.

<a id="udp-lifecycle-01"></a>
### Case UDP-LIFECYCLE-01

**Requirements:** S31, UDP-02, UDP-03. **Owner:** #89; final integration #97. **Planning:** MAPPED.

**Source AC:** [89.AC1](#89-ac1); [89.AC2](#89-ac2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Approved #71/#78 expiry/reconnect/source-filter/tuple-reuse policy and UDP run path.

**Action:** Induce idle expiry/reconnect, delayed replies after OS tuple reuse and replies from another source endpoint.

**Expected observable outcome:** Matches approved lifecycle/filtering outcomes; residual tuple-reuse ambiguity documented, no absolute isolation claim across all reuse. Detailed expiry/filter outcomes await decision.

**Evidence/platform:** Approved policy, old/new flow and socket trace, recipient observations and risk documentation. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Await #71/#78 lifecycle, source filtering and late-reply policy; UDP deferred.

<a id="udp-limit-01"></a>
### Case UDP-LIMIT-01

**Requirements:** S30, UDP-01, UDP-03. **Owner:** #89; final integration #97. **Planning:** MAPPED.

**Source AC:** [89.AC1](#89-ac1). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Approved #73/#74/#78 transport, payload/resource/cleanup policy; instrumentation.

**Action:** Send around measured size limit; exhaust flows/sockets/queues with churn/slow consumers; if fragmentation selected inject fragment loss/duplicates/malicious reassembly.

**Expected observable outcome:** Documented bounded size/admission/drop/cleanup behavior matches approved values; never present partial datagram as intact. No numeric bound, fragment scheme or drop policy approved by this catalog.

**Evidence/platform:** Approved limits, delivered/drop counters, memory/handles/queues and cleanup observations. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Await #73/#74/#78 payload/transport/resource decisions and measured bounds; UDP deferred.

<a id="udp-path-01"></a>
### Case UDP-PATH-01

**Requirements:** S32, TC-02, UDP-01. **Owner:** #89; final integration #97. **Planning:** MAPPED.

**Source AC:** [89.AC2](#89-ac2); [97.AC2](#97-ac2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Selected final Iroh version, approved UDP contract; controllable direct and forced-relay network.

**Action:** Verify selected direct then forced-relay route; vary payloads/flow counts, loss, delay, congestion and path change; measure end-to-end behavior.

**Expected observable outcome:** Both selected paths evidenced, benchmark methodology and actual latency/jitter/loss/throughput/CPU/memory recorded; best-effort semantics and tuple-reuse limits documented. No fabricated performance threshold.

**Evidence/platform:** Path telemetry/packet evidence, impairment commands and raw benchmark data. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — UDP deferred; #71/#73/#74/#78 and selected-version #101 evidence required for final run.

<a id="sm-scope-01"></a>
### Case SM-SCOPE-01

**Requirements:** S45, S46, SM-01. **Owner:** #90 / #91; final integration #97. **Planning:** MAPPED.

**Source AC:** [90.CL1](#90-cl1); [91.AC1](#91-ac1); [91.CL1](#91-cl1). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Non-elevated interactive user and explicit system request on real supported OS.

**Action:** Invoke interactive installation; select user then system in isolated runs; repeat --system without privilege.

**Expected observable outcome:** User/system choice offered; unprivileged system selection stops with sudo/elevated-terminal rerun guidance preserving intended args/absolute paths, no automatic elevation or silent user fallback.

**Evidence/platform:** Prompt/exit transcript, unchanged registrations, rerun arguments/paths. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="sm-user-01"></a>
### Case SM-USER-01

**Requirements:** S49, S54, SM-02. **Owner:** #90; final integration #97. **Planning:** MAPPED.

**Source AC:** [90.AC1](#90-ac1); [90.CL2](#90-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Applicable real user service adapter, resolved user absolute config/state paths, peer/target.

**Action:** Install/start/status/stop/restart/uninstall; pass bytes while running; restart with changed CWD/home and inspect registration.

**Expected observable outcome:** Lifecycle uses pinned absolute config/state paths and stable configured identity; status correlates actual role. No proposed absent-service/idempotency behavior imposed.

**Evidence/platform:** OS service registration/account, path/identity evidence and byte/status/lifecycle transcripts. Platform: Applicable Linux/macOS user service managers; Windows via SM-WINDOWS-01.

**Execution:** BLOCKED — Native #90 blockers #84/#87 unresolved; Windows mechanism separately gated by #79/#92.

<a id="sm-permission-01"></a>
### Case SM-PERMISSION-01

**Requirements:** S55, SM-03. **Owner:** #90 / #91; final integration #97. **Planning:** MAPPED.

**Source AC:** [90.AC2](#90-ac2); [91.AC2](#91-ac2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Real accounts with denied config/state/registration permissions.

**Action:** Attempt applicable service operations without required access; compare permission/account/scope metadata.

**Expected observable outcome:** Actionable failure; never silently broaden access, change account or change scope.

**Evidence/platform:** Errors, OS account/privilege and before/after ownership/ACL/registration evidence. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="sm-elevated-01"></a>
### Case SM-ELEVATED-01

**Requirements:** S47, S48, SM-01. **Owner:** #91; final integration #97. **Planning:** MAPPED.

**Source AC:** [91.AC1](#91-ac1); [91.CL1](#91-cl1). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Root/elevated Administrator and existing conflicting service/files.

**Action:** Run elevated default install, observe prompts and decline safety confirmation.

**Expected observable outcome:** System default without scope prompt; safety confirmation still present and decline leaves conflicting state unchanged. Pinned-user paths not replaced by elevated home defaults.

**Evidence/platform:** Prompt transcript, selected paths and registration/file diff. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Native #91 blockers #79/#84/#87 unresolved for privileged service composition.

<a id="sm-path-01"></a>
### Case SM-PATH-01

**Requirements:** S49, SM-02. **Owner:** #91; final integration #97. **Planning:** MAPPED.

**Source AC:** [91.AC2](#91-ac2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Approved privileged service adapter/account and installing-user config/state paths.

**Action:** Install system service then change invoking home/CWD; start/restart actual service and transfer client bytes.

**Expected observable outcome:** Actual registered command/runtime use installing user selected absolute config/state paths, not another account home; stable configured identity.

**Evidence/platform:** Real registration/runtime account/paths, public IDs and byte transcript. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Await #79 privileged service trust/account policy plus #84/#87 dependencies.

<a id="sm-retarget-01"></a>
### Case SM-RETARGET-01

**Requirements:** S48, S50, SM-02. **Owner:** #91; final integration #97. **Planning:** MAPPED.

**Source AC:** [91.AC2](#91-ac2); [91.CL2](#91-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Existing other-user service, both user config hashes/permissions, approved privileged adapter.

**Action:** Request retarget; decline then approve in separate runs; inspect registration and both config files.

**Expected observable outcome:** Explicit confirmation required; decline changes neither service nor files; approval retargets to selected absolute paths and preserves both configs.

**Evidence/platform:** Prompt transcript, registration delta, hashes/ownership for both users. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Native #91 #79/#84/#87 dependencies unresolved; approved trust/user resolution needed.

<a id="sm-unknown-01"></a>
### Case SM-UNKNOWN-01

**Requirements:** S51, SM-02. **Owner:** #91; final integration #97. **Planning:** MAPPED.

**Source AC:** [91.CL2](#91-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Ambiguous invoking user (including sudo), interactive and noninteractive sessions.

**Action:** Attempt installation interactively then without terminal; provide explicit paths in guided retry.

**Expected observable outcome:** Interactive selection required; noninteractive fails with actionable path guidance; no silent root-home substitution. Trusted user identification mechanism remains #79 decision.

**Evidence/platform:** Prompt/errors, account/path resolution and absence of unintended registration. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Await #79 trusted user identification and privileged account policy.

<a id="sm-edit-01"></a>
### Case SM-EDIT-01

**Requirements:** S52, SM-05. **Owner:** #91; final integration #97. **Planning:** MAPPED.

**Source AC:** [91.CL3](#91-cl3). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Authorized user-editable managed config and authorized admin restart using approved adapter.

**Action:** Edit valid setting as authorized user, restart as admin and test changed behavior; repeat invalid schema.

**Expected observable outcome:** Valid authorized edit applies, not treated as hostile integrity violation; invalid config reports error. No silent runtime-account/ownership/permission change or immutable/root-only requirement.

**Evidence/platform:** Edit/restart commands, actual service result, errors and account/permission diffs. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Confirmed workflow; actual privileged adapter depends on #79 threat-model/account decisions.

<a id="sm-trust-01"></a>
### Case SM-TRUST-01

**Requirements:** S52, SM-03, SM-05. **Owner:** #91; final integration #97. **Planning:** MAPPED.

**Source AC:** [91.AC2](#91-ac2); [91.CL3](#91-cl3). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Approved #79 unauthorized-writer/path-resolution threat model and real users/service privileges.

**Action:** Attempt unauthorized writes, hostile parent replacement, symlinks and TOCTOU races; inspect actual runtime account and protected files.

**Expected observable outcome:** Unauthorized actions handled per approved model without silent access broadening; authorized edit workflow still works. No specific symlink/integrity algorithm or root runtime chosen here.

**Evidence/platform:** Policy link, attack transcripts and actual OS privileges/ACL/path effects. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Await #79 runtime account, ownership, symlink/integrity and TOCTOU policy.

<a id="sm-windows-01"></a>
### Case SM-WINDOWS-01

**Requirements:** S53. **Owner:** #92; final integration #97. **Planning:** MAPPED.

**Source AC:** [92.AC1](#92-ac1); [92.AC2](#92-ac2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Approved Windows logged-in user background mechanism and actual non-admin session.

**Action:** Install/start/status/stop role, exchange bytes with peer/target; inspect LocalAppData/override paths, account/permissions; test denied access and foreground conflict.

**Expected observable outcome:** Approved mechanism runs while user logged in, intended user paths/permissions used; denied access gives guidance without elevation/account change. Record x64/ARM64 coverage gaps, not PASS for unavailable host.

**Evidence/platform:** Registration, login/account/architecture, bytes and lifecycle/errors. Platform: Real Windows x64 and ARM64 where available.

**Execution:** BLOCKED — Await #79 Windows mechanism and #84/#87 dependencies; no SCM/task adapter assumed.

<a id="install-default-01"></a>
### Case INSTALL-DEFAULT-01

**Requirements:** S56, S57, DIST-01. **Owner:** #93; final integration #97. **Planning:** MAPPED.

**Source AC:** [93.AC1](#93-ac1); [93.CL1](#93-cl1). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Disposable real shell Linux/macOS and Windows PowerShell environments, appropriate privileges.

**Action:** Execute installer default flow, inspect installed binaries/aliases and run each role against target.

**Expected observable outcome:** Default selects all three binaries system-wide; actual intended artifacts and aliases installed and roles forward independently.

**Evidence/platform:** Installer transcript, install paths/digests, alias resolution and role bytes. Platform: Linux/macOS shell and Windows PowerShell.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="install-consent-01"></a>
### Case INSTALL-CONSENT-01

**Requirements:** S58, DIST-02. **Owner:** #93; final integration #97. **Planning:** MAPPED.

**Source AC:** [93.AC2](#93-ac2); [93.CL1](#93-cl1); [93.CL2](#93-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Seed PATH and conflicting file; installer with before-state hashes and permission metadata.

**Action:** Decline then accept PATH and file-replacement prompts in separate runs; test insufficient permission. If installer changes service, execute approved service safety cases.

**Expected observable outcome:** PATH mutation/conflicting replacement require confirmation; decline leaves prior state unchanged. Service-changing path follows #79/#91 policy; binary publication alone implies no service policy.

**Evidence/platform:** Prompts, PATH/file hashes/permissions and conditional service registration evidence. Platform: Supported shell/PowerShell platforms; service branch requires approved #79/#91.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="package-create-01"></a>
### Case PACKAGE-CREATE-01

**Requirements:** S59, DIST-01. **Owner:** #94; final integration #97. **Planning:** MAPPED.

**Source AC:** [94.AC1](#94-ac1); [94.AC2](#94-ac2); [94.CL1](#94-cl1); [94.CL2](#94-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Versioned separate all-in-one/Serve/Access packages for actual supported formats; clean package environments.

**Action:** Build/fetch and inspect each package manifest/content, install each alone, inspect manager ownership/aliases and run supported roles with peer/target. If service files included, test approved service composition.

**Expected observable outcome:** Each package contains intended independent artifact, runs without other role binaries and has correct manager ownership/aliases. Cross-build distinct from runtime; binary-only packages need not wait for service mechanism.

**Evidence/platform:** Immutable package/artifact IDs, manifests/DB query, isolated install/byte transcripts and conditional unit-file account/trust/path evidence. Platform: Actual supported package OS/architecture matrix, including Windows release artifacts where applicable.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="image-create-01"></a>
### Case IMAGE-CREATE-01

**Requirements:** S60, DIST-01. **Owner:** #94; final integration #97. **Planning:** MAPPED.

**Source AC:** [94.AC1](#94-ac1); [94.AC2](#94-ac2); [94.CL1](#94-cl1). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Three versioned all-in-one/Serve/Access images, container runtime and reachable peer/target.

**Action:** Inspect each image/entrypoint; deploy independently and run both all-in-one roles plus respective role images.

**Expected observable outcome:** Each intended standalone role executes and forwards independently; ownership is container image. No promise of Windows container support.

**Evidence/platform:** Image digests/manifests, entrypoints, runtime commands and bytes. Platform: Released amd64/arm64 container matrix; record actual runtime architecture.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="update-select-01"></a>
### Case UPDATE-SELECT-01

**Requirements:** S62, S64, DIST-01, DIST-02. **Owner:** #95; final integration #97. **Planning:** MAPPED.

**Source AC:** [95.CL1](#95-cl1); [95.CL2](#95-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** All three standalone binaries with before hashes; real approved signed stable/versioned releases.

**Action:** Invoke each executable update using default stable then explicit version; exercise installed aliases using approved alias resolution.

**Expected observable outcome:** Only invoked selected executable changes to requested/default stable version; unrelated binaries unchanged. Managed ownership paths remain separate.

**Evidence/platform:** Version selection, commands/resolved artifact IDs, before/after all-binary hashes and usable role run. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Native #95 #80/#82 dependencies; replacement/alias policy and final signed #100 producer artifacts required.

<a id="update-verify-01"></a>
### Case UPDATE-VERIFY-01

**Requirements:** S65, S66, DIST-02. **Owner:** #95; final integration #97. **Planning:** MAPPED.

**Source AC:** [95.AC1](#95-ac1); [95.CL3](#95-cl3). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Actual #100 signed artifacts, approved pinned CI issuer/identity, known installed usable binary.

**Action:** Verify good artifact; independently tamper payload/checksum/signature, wrong issuer/identity, missing material and failed fetch; run updater without bypass.

**Expected observable outcome:** Only valid pinned signature plus checksum verifies; verification failure preserves installed executable, still usable; no silent replacement.

**Evidence/platform:** Immutable source/signing artifact IDs, verifier/update outputs, old-binary hashes and post-failure role bytes. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Native #80 trust/replacement decisions and #100 real producer evidence not complete; fixtures alone cannot close final gate.

<a id="update-bypass-01"></a>
### Case UPDATE-BYPASS-01

**Requirements:** S67, DIST-02. **Owner:** #95; final integration #97. **Planning:** MAPPED.

**Source AC:** [95.AC1](#95-ac1); [95.CL3](#95-cl3). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Approved #80 exact unsafe bypass scope and invalid verification fixture.

**Action:** Invoke actual explicit unsafe bypass and compare normal failure path.

**Expected observable outcome:** Bypass is deliberate and warns; only approved verification checks bypassed. Checksum-bypass semantics not determined here.

**Evidence/platform:** Explicit command/warning, approved scope link and resulting artifact/hash. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Await #80 exact checksum-bypass semantics and replacement policy.

<a id="update-running-01"></a>
### Case UPDATE-RUNNING-01

**Requirements:** S68, DIST-02. **Owner:** #95; final integration #97. **Planning:** MAPPED.

**Source AC:** [95.AC2](#95-ac2); [95.CL3](#95-cl3); [80.NO-STOP](#80-no-stop). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Live marked tunnel transfer during update; final updater on actual OS.

**Action:** Invoke update while tunnel runs; observe process/data and operator guidance.

**Expected observable outcome:** Updater never automatically stops tunnel; guidance supports operator-controlled stop/update/restart. Replacement mechanics evaluated separately.

**Evidence/platform:** Process uptime, byte/timing trace, updater output; preserve evidence if update cannot proceed. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="update-recovery-01"></a>
### Case UPDATE-RECOVERY-01

**Requirements:** S66, S68, DIST-02. **Owner:** #95; final integration #97. **Planning:** MAPPED.

**Source AC:** [95.AC2](#95-ac2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Approved #80 recovery/rotation/partial-failure and Windows running-file replacement contract.

**Action:** Inject replacement failure/partial write; exercise recovery and trust rotation, including Windows locked executable.

**Expected observable outcome:** Observed replacement/recovery/rotation follows approved policy; no automatic tunnel stop. No invented staging, atomicity or rollback guarantee.

**Evidence/platform:** Approved policy, OS filesystem behavior, old/new hashes, running process and recovery transcript. Platform: Actual supported OS, including Windows locked running executable.

**Execution:** BLOCKED — Await #80 platform replacement, rollback/partial failure and signing trust rotation policy.

<a id="package-update-01"></a>
### Case PACKAGE-UPDATE-01

**Requirements:** S59, S63, DIST-01, DIST-02. **Owner:** #96; final integration #97. **Planning:** MAPPED.

**Source AC:** [96.AC1](#96-ac1); [96.CL1](#96-cl1); [96.CL2](#96-cl2); [95.CL2](#95-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Actual role-specific managed packages, unrelated binaries with hashes, approved package detection/alias policy.

**Action:** Invoke actual update entrypoint and aliases for one package; follow corresponding manager delegation/guidance and perform selective package update.

**Expected observable outcome:** Corresponding package manager owns update, not standalone overwrite; selected package version changes, unrelated binaries remain unchanged and role still runs.

**Evidence/platform:** Entry commands/output, package DB/version, before/after hashes for every binary and real target bytes. Platform: Actual supported package managers/platforms.

**Execution:** BLOCKED — Unresolved package detection/alias/partial-failure choices (#96 clarification checkbox 2); #94 packages required.

<a id="image-update-01"></a>
### Case IMAGE-UPDATE-01

**Requirements:** S61, DIST-01, DIST-02. **Owner:** #96; final integration #97. **Planning:** MAPPED.

**Source AC:** [96.AC1](#96-ac1); [96.CL1](#96-cl1); [95.CL2](#95-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Old/new actual role images, running container, peer and target.

**Action:** Invoke applicable in-container update entrypoint; follow pull/redeploy guidance and redeploy selected new image.

**Expected observable outcome:** No self-update or mutable standalone overwrite; image redeployment changes version and role still forwards. Unrelated artifacts unchanged.

**Evidence/platform:** Old/new image digests, container filesystem comparison, update/redeploy output and bytes. Platform: Actual released container runtime/architectures.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="package-legacy-01"></a>
### Case PACKAGE-LEGACY-01

**Requirements:** S69, DIST-01. **Owner:** #96; final integration #97. **Planning:** MAPPED.

**Source AC:** [96.AC2](#96-ac2); [96.CL1](#96-cl1); [96.CL2](#96-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Actual legacy bundled package and approved migration/detection/alias policy; all binary hashes.

**Action:** Invoke update via each applicable owned entrypoint/alias, follow migration guidance to separate package and run role.

**Expected observable outcome:** Migration does not silently update unrelated binaries; only intended manager owns changes, no standalone managed-file overwrite.

**Evidence/platform:** Migration instructions/transcript, package DB and all-binary hashes before/after, role bytes. Platform: Actual legacy package managers/platforms.

**Execution:** BLOCKED — Await #96 unresolved detection, alias and partial-failure policy; real legacy package fixture required.

<a id="cfg-lifecycle-01"></a>
### Case CFG-LIFECYCLE-01

**Requirements:** S72. **Owner:** #99; final integration #97. **Planning:** MAPPED.

**Source AC:** [98.AC1](#98-ac1); [98.AC2](#98-ac2); [98.AC3](#98-ac3); [99.AC1](#99-ac1); [99.AC2](#99-ac2); [99.AC3](#99-ac3). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Approved #98 command-by-command lifecycle/identity policy and #83 configured baseline.

**Action:** Run approved config/setup/mutation/rotation/display commands against actual files; restart processes; execute approved invalid/concurrent/secret cases.

**Expected observable outcome:** Each approved decision implemented with explicit command/scenario trace; confirmed migration/identity safeguards retained; intentional changes documented. Proposed validation/atomicity/rotation/disclosure outcomes not invented.

**Evidence/platform:** Decision inventory, commands, file/public-ID diffs, secret handling and restart/byte evidence. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Native #99 #98/#83 blockers; lifecycle decisions unresolved. Planning envelope requires concrete policy assertions before execution.

<a id="release-sign-01"></a>
### Case RELEASE-SIGN-01

**Requirements:** S65, S72, DIST-02. **Owner:** #100; final integration #97. **Planning:** MAPPED.

**Source AC:** [100.AC1](#100-ac1); [100.AC2](#100-ac2); [100.AC3](#100-ac3); [94.CL3](#94-cl3). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Approved #80 CI trust policy, #82 artifacts and actual release production environment.

**Action:** Produce complete intended artifact/checksum/keyless signature set, verify via integrated #95 verifier; test wrong identity, changed payload and missing/invalid material. Inspect promotion gate.

**Expected observable outcome:** Artifacts bound to approved issuer AND identity verify, invalid material rejected; immutable IDs recorded. Public promotion gated on complete approved artifact set and final #97 evidence.

**Evidence/platform:** Actual producer/verifier commands, immutable artifact/signature/checksum IDs and CI promotion configuration/results. Platform: Supported release artifact matrix; cross-build labeled separately.

**Execution:** BLOCKED — Native #100 #80/#82 dependencies; actual signed production and final #97 acceptance not established.

<a id="dependency-01"></a>
### Case DEPENDENCY-01

**Requirements:** S72, TC-02. **Owner:** #101; final integration #97. **Planning:** MAPPED.

**Source AC:** [101.AC1](#101-ac1); [101.AC2](#101-ac2); [101.AC3](#101-ac3). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Current lockfile, stable-release selection evidence and supported artifact/platform matrix.

**Action:** Record selected stable version and constraints, upgrade/build, run affected TCP/relay/status regression and both 0.4 directions; feed version into UDP benchmark/final gate.

**Expected observable outcome:** No silent behavior/platform loss; final commit/lockfile/artifacts bind compatibility results; later dependency/artifact change triggers affected reruns. Design-time 1.4.1 not unconditional pin.

**Evidence/platform:** Selection evidence, dependency diff/build commands and linked final-version regression results. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="reg-relay-01"></a>
### Case REG-RELAY-01

**Requirements:** Retained functionality (#97 clarification checkbox 2). **Owner:** #97; final integration #97. **Planning:** MAPPED.

**Source AC:** [53.AC1](#53-ac1); [53.AC2](#53-ac2); [53.AC3](#53-ac3); [53.AC4](#53-ac4); [53.AC5](#53-ac5); [53.AC6](#53-ac6); [53.AC7](#53-ac7); [97.CL2](#97-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Final binaries, local authenticated relay including on-demand/non-first relay route, real target/client.

**Action:** Exercise both role relay_token config, reject empty/whitespace tokens; run correct/wrong/missing token and on-demand dial; inspect logs/status and docs examples.

**Expected observable outcome:** Correct token registers/dials and forwards; wrong/missing rejected with clear non-hanging connection failure; no token in logs/status; on-demand behavior and wiring documented.

**Evidence/platform:** Relay auth observations, bytes, invalid-config errors, redaction scan and docs excerpts; #101 final-version rerun. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="reg-relay-02"></a>
### Case REG-RELAY-02

**Requirements:** Retained functionality (#97 clarification checkbox 2). **Owner:** #97; final integration #97. **Planning:** MAPPED.

**Source AC:** [54.AC1](#54-ac1); [54.AC2](#54-ac2); [54.AC3](#54-ac3); [54.AC4](#54-ac4); [54.AC5](#54-ac5); [54.AC6](#54-ac6); [97.CL2](#97-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Two local relays and services, reachable Serve registration, node/default fallback fixtures.

**Action:** Run services concurrently with distinct per-service relay sets; test absent/empty override then empty node list, and invalid non-https service URL.

**Expected observable outcome:** Per-service selected routes work independently; fallback service → node → defaults retained; invalid URL diagnostic names service; docs describe Serve-side registration constraint.

**Evidence/platform:** Actual selected relay/dial addresses and bytes, fallback/validation outputs and docs; retain duplicate-name/conflict validation. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="reg-status-01"></a>
### Case REG-STATUS-01

**Requirements:** Retained functionality (#97 clarification checkbox 2). **Owner:** #97; final integration #97. **Planning:** MAPPED.

**Source AC:** [57.AC2](#57-ac2); [57.AC3](#57-ac3); [57.AC4](#57-ac4); [57.AC5](#57-ac5); [57.AC6](#57-ac6); [58.AC1](#58-ac1); [58.AC2](#58-ac2); [58.AC4](#58-ac4); [58.AC5](#58-ac5); [58.AC6](#58-ac6); [59.AC1](#59-ac1); [59.AC2](#59-ac2); [59.AC3](#59-ac3); [59.AC5](#59-ac5); [59.AC6](#59-ac6); [97.CL2](#97-cl2). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Final roles with active paths, actual path-change controls and isolated status directories.

**Action:** Read role status files/CLI human and --json output; induce relay/direct/fallback/address transitions, then unchanged snapshots; remove status file.

**Expected observable outcome:** Role-scoped status reports peer/service and active/inactive transports, local addresses labeled candidates (not per-path mapping). JSON passthrough, missing-file clear error/nonzero per existing convention. Establish/path-change logs have correct fields, no unchanged-snapshot spam; status refreshes atomically. Docs retain schema/paths/log glossary and path-change release note.

**Evidence/platform:** Status bytes/refresh/path observations, CLI exits and logs, README/CHANGELOG; rerun on #101 final version. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** NOT_RUN — no independently accepted final-artifact run attached.

<a id="release-matrix-01"></a>
### Case RELEASE-MATRIX-01

**Requirements:** S70, S71. **Owner:** #97; final integration #97. **Planning:** MAPPED.

**Source AC:** [97.AC1](#97-ac1); [97.CL2](#97-cl2); [97.CL3](#97-cl3); [97.CL4](#97-cl4). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** Final release candidate, supported-platform inventory and all applicable catalog fixtures.

**Action:** Per actual OS/architecture execute installation → all three binaries/aliases long/short quick/configured tunnel → service lifecycle → standalone/managed update. Include TCP/UDP, ownership/security and retained relay/status cases.

**Expected observable outcome:** Record actual supported combinations and cross-feature outcomes; preserve released Linux/macOS plus Windows x64/ARM64 artifacts and runtime where available. Missing environments/evidence never PASS, release exception needs explicit decision.

**Evidence/platform:** Per-case immutable evidence matrix and composition commands, versions/digests, privilege and observed results. Platform: Linux x86_64/aarch64 glibc; x86_64/aarch64/riscv64 musl; macOS aarch64; released Linux packages and amd64/arm64 images; Windows x64/ARM64.

**Execution:** BLOCKED — Native #97 blockers #84–#96 plus #99/#100/#101 remain open (not #83); full runtime matrix and UDP evidence absent.

<a id="release-gate-01"></a>
### Case RELEASE-GATE-01

**Requirements:** S72. **Owner:** #97; final integration #97. **Planning:** MAPPED.

**Source AC:** [97.AC1](#97-ac1); [97.AC2](#97-ac2); [97.CL1](#97-cl1); [97.CL2](#97-cl2); [97.CL3](#97-cl3); [97.CL4](#97-cl4); [97.CL5](#97-cl5). Exact URLs/checkbox text in [source register](#source-ac-register).

**Prerequisites:** All final-artifact catalog evidence, resolved applicable decisions/native blockers, independent acceptance review.

**Action:** Audit every story/rule → source checkbox → case → final evidence; inspect rerun freshness, actual platforms/paths, compatibility decisions, #99/#100/#101 and release exceptions.

**Expected observable outcome:** No unit-only closure, stale evidence or untested PASS. All 72 stories and 21 confirmed rules trace to owned cases and accepted evidence; UDP/direct/forced-relay/benchmarks remain required; #97 stays blocked until native blockers resolved.

**Evidence/platform:** Signed-off coverage/evidence matrix, actual results and missing coverage, decision/review artifacts; no approval inferred from planning counts. Platform: Supported Linux/macOS/Windows runtime matrix.

**Execution:** BLOCKED — Full release evidence and all native #97 blockers #84–#96, #99/#100/#101 unresolved; this documentation change is planning only.

<a id="source-ac-register"></a>
## Exact source AC register

Locators use the supplied complete issue snapshots (bodies, comments and native blockers). `N.ACx` = checkbox x under issue N body **Acceptance criteria**; `N.CLx` = checkbox x in the linked named clarification comment, also mirrored in issue body. Ordinals are within that section/comment, not the combined issue. GitHub does not supply checkbox-specific anchors: the URL targets the exact section/comment and the ordinal plus quoted text identifies the assertion. Revalidate wording/ordinal if tracker changes. The two decision references are superseding comments, not checkboxes. Retained #53/#54/#57–#59 sources use English descriptions with exact checkbox ordinal because their source text is not English.

| AC key | Exact source locator | Source checkbox text / decision (English descriptions for retained issues) |
|---|---|---|
| <a id="53-ac1"></a>53.AC1 | [#53 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/53#acceptance-criteria) | Both roles accept relay_token; empty/whitespace validation errors. |
| <a id="53-ac2"></a>53.AC2 | [#53 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/53#acceptance-criteria) | Serve registers through shared-token relay. |
| <a id="53-ac3"></a>53.AC3 | [#53 body, Acceptance criteria, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/53#acceptance-criteria) | Access succeeds with correct token, clearly rejected for wrong/missing token. |
| <a id="53-ac4"></a>53.AC4 | [#53 body, Acceptance criteria, checkbox 4](https://github.com/naicoi92/iroh-tunnel/issues/53#acceptance-criteria) | CI-safe E2E using local in-process shared-token relay. |
| <a id="53-ac5"></a>53.AC5 | [#53 body, Acceptance criteria, checkbox 5](https://github.com/naicoi92/iroh-tunnel/issues/53#acceptance-criteria) | Status and logs do not expose token. |
| <a id="53-ac6"></a>53.AC6 | [#53 body, Acceptance criteria, checkbox 6](https://github.com/naicoi92/iroh-tunnel/issues/53#acceptance-criteria) | Verify/document on-demand relay connection token behavior. |
| <a id="53-ac7"></a>53.AC7 | [#53 body, Acceptance criteria, checkbox 7](https://github.com/naicoi92/iroh-tunnel/issues/53#acceptance-criteria) | Update relay_token example/docs. |
| <a id="54-ac1"></a>54.AC1 | [#54 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/54#acceptance-criteria) | Per-service relay_urls override node relay_urls. |
| <a id="54-ac2"></a>54.AC2 | [#54 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/54#acceptance-criteria) | Absent/empty service list falls back node then defaults. |
| <a id="54-ac3"></a>54.AC3 | [#54 body, Acceptance criteria, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/54#acceptance-criteria) | Non-https URL names service in error; retain duplicate-name/conflict validation. |
| <a id="54-ac4"></a>54.AC4 | [#54 body, Acceptance criteria, checkbox 4](https://github.com/naicoi92/iroh-tunnel/issues/54#acceptance-criteria) | Two services carry their own dial relay sets. |
| <a id="54-ac5"></a>54.AC5 | [#54 body, Acceptance criteria, checkbox 5](https://github.com/naicoi92/iroh-tunnel/issues/54#acceptance-criteria) | Two local-relay services tunnel concurrently. |
| <a id="54-ac6"></a>54.AC6 | [#54 body, Acceptance criteria, checkbox 6](https://github.com/naicoi92/iroh-tunnel/issues/54#acceptance-criteria) | Examples/docs include Serve-side relay-registration constraint. |
| <a id="57-ac2"></a>57.AC2 | [#57 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/57#acceptance-criteria) | Serve connections contain peer/services/transports with usage and local bound candidates. |
| <a id="57-ac3"></a>57.AC3 | [#57 body, Acceptance criteria, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/57#acceptance-criteria) | Rename to serve-status.json and update status documentation. |
| <a id="57-ac4"></a>57.AC4 | [#57 body, Acceptance criteria, checkbox 4](https://github.com/naicoi92/iroh-tunnel/issues/57#acceptance-criteria) | README explains status path/schema/transport semantics in English. |
| <a id="57-ac5"></a>57.AC5 | [#57 body, Acceptance criteria, checkbox 5](https://github.com/naicoi92/iroh-tunnel/issues/57#acceptance-criteria) | CHANGELOG notes breaking status path change. |
| <a id="57-ac6"></a>57.AC6 | [#57 body, Acceptance criteria, checkbox 6](https://github.com/naicoi92/iroh-tunnel/issues/57#acceptance-criteria) | Active Serve status transport observed through local relay integration. |
| <a id="58-ac1"></a>58.AC1 | [#58 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/58#acceptance-criteria) | Connection-established INFO includes service, short peer and active transports. |
| <a id="58-ac2"></a>58.AC2 | [#58 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/58#acceptance-criteria) | Path-change log for relay/direct/fallback/address transitions. |
| <a id="58-ac4"></a>58.AC4 | [#58 body, Acceptance criteria, checkbox 4](https://github.com/naicoi92/iroh-tunnel/issues/58#acceptance-criteria) | Document event types and fields in log glossary. |
| <a id="58-ac5"></a>58.AC5 | [#58 body, Acceptance criteria, checkbox 5](https://github.com/naicoi92/iroh-tunnel/issues/58#acceptance-criteria) | Actual Serve/Access relay-to-direct migration E2E output. |
| <a id="58-ac6"></a>58.AC6 | [#58 body, Acceptance criteria, checkbox 6](https://github.com/naicoi92/iroh-tunnel/issues/58#acceptance-criteria) | No log spam when snapshot unchanged. |
| <a id="59-ac1"></a>59.AC1 | [#59 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/59#acceptance-criteria) | Access status written atomically and refreshed with path changes. |
| <a id="59-ac2"></a>59.AC2 | [#59 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/59#acceptance-criteria) | Role status human table and verbatim JSON passthrough. |
| <a id="59-ac3"></a>59.AC3 | [#59 body, Acceptance criteria, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/59#acceptance-criteria) | Missing status gives clear error and existing nonzero exit convention. |
| <a id="59-ac5"></a>59.AC5 | [#59 body, Acceptance criteria, checkbox 5](https://github.com/naicoi92/iroh-tunnel/issues/59#acceptance-criteria) | Active Access status includes per-service peer and active transport. |
| <a id="59-ac6"></a>59.AC6 | [#59 body, Acceptance criteria, checkbox 6](https://github.com/naicoi92/iroh-tunnel/issues/59#acceptance-criteria) | README documents both role paths and status output. |
| <a id="80-no-stop"></a>80.NO-STOP | [#80 partial decision — running tunnel](https://github.com/naicoi92/iroh-tunnel/issues/80#issuecomment-6490074418) | User confirmed the updater must not automatically stop a running tunnel. Give operator guidance to stop/update/restart as needed; platform replacement and rollback details remain open. This supersedes any proposed automatic stop/restart flow. |
| <a id="81-ac1"></a>81.AC1 | [#81 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/81#acceptance-criteria) | Real client exchanges bytes through the tunnel with a target fixture using quick-run commands; TCP is the default. |
| <a id="81-ac2"></a>81.AC2 | [#81 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/81#acceptance-criteria) | Ephemeral identity and invalid input behavior are observable; introduce only the DDD seams needed by this complete path. |
| <a id="81-cl1"></a>81.CL1 | [#81 clarification, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/81#issuecomment-6491341020) | Exercise quick-run long and short options through real Serve and Access processes; equivalent options reach the same fixed target. Reject mixed quick flags and configured subcommands rather than silently ignoring input. |
| <a id="81-cl2"></a>81.CL2 | [#81 clarification, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/81#issuecomment-6491341020) | For both roles, prove default ephemeral identities across restarts and actionable role-specific invalid-input diagnostics. Preserve user configuration and key material on failure. |
| <a id="81-cl3"></a>81.CL3 | [#81 clarification, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/81#issuecomment-6491341020) | Document that NodeId is public identity, not target authorization; target services must enforce their own access protection (story 17, TC-03). This does not introduce a new authentication scheme. |
| <a id="82-ac1"></a>82.AC1 | [#82 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/82#acceptance-criteria) | Each binary runs its supported role without another binary; quick-run and supplied persistent key work. |
| <a id="82-ac2"></a>82.AC2 | [#82 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/82#acceptance-criteria) | Help documents supported options and defaults; short options are consistent and non-conflicting. |
| <a id="82-cl1"></a>82.CL1 | [#82 clarification, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/82#issuecomment-6491341533) | Run all-in-one Serve and Access plus each standalone role as separate real processes without the other binaries installed. Verify the `it`, `its`, `ita` aliases resolve and run their intended roles. |
| <a id="82-cl2"></a>82.CL2 | [#82 clarification, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/82#issuecomment-6491341533) | Exercise equivalent long and short options on each of the three binaries and both all-in-one roles, including TCP defaults and persistent-key restart identity. Help must expose supported options, aliases and defaults; short options must not conflict. |
| <a id="82-cl3"></a>82.CL3 | [#82 clarification, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/82#issuecomment-6491341533) | Verify role-specific invalid options and diagnostics. These are additions to the existing #82 acceptance criteria, not a substitute for process-level forwarding. |
| <a id="83-ac1"></a>83.AC1 | [#83 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/83#acceptance-criteria) | Configured Serve and Access forward TCP through a real tunnel. |
| <a id="83-ac2"></a>83.AC2 | [#83 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/83#acceptance-criteria) | Legacy multiplex values are silently ignored and removed upon saving; no setting disables multiplex. |
| <a id="84-ac1"></a>84.AC1 | [#84 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/84#acceptance-criteria) | Defaults and overrides are verified on applicable platforms, including Windows LocalAppData. |
| <a id="84-ac2"></a>84.AC2 | [#84 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/84#acceptance-criteria) | New config wins; legacy copy never overwrites, preserves source and secret permissions, and failure never creates a substitute identity. |
| <a id="84-cl1"></a>84.CL1 | [#84 clarification, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/84#issuecomment-6491342083) | On applicable real platforms verify macOS/Linux config `~/.config/iroh-tunnel` and state `~/.local/state/iroh-tunnel` ignore XDG overrides; Windows uses the installing user LocalAppData locations; explicit app overrides work. |
| <a id="84-cl2"></a>84.CL2 | [#84 clarification, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/84#issuecomment-6491342083) | Inaccessible or invalid config, unusable home, and migration failures provide actionable guidance without silently using another user’s configuration or replacing identity. Test new-location precedence, no-overwrite migration, source preservation and secret permissions on the applicable platform. |
| <a id="85-ac1"></a>85.AC1 | [#85 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/85#acceptance-criteria) | Test real 0.4 Access → redesigned Serve and redesigned Access → real 0.4 Serve against a TCP target; record exact versions and outcomes. |
| <a id="85-ac2"></a>85.AC2 | [#85 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/85#acceptance-criteria) | If either path fails, document the compatibility trade-off and obtain the user's decision before a breaking release; do not silently declare compatibility. |
| <a id="85-ac3"></a>85.AC3 | [#85 body, Acceptance criteria, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/85#acceptance-criteria) | Do not add a version-negotiation framework for this redesign. Test other supported peer combinations as applicable. |
| <a id="86-ac1"></a>86.AC1 | [#86 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/86#acceptance-criteria) | Real concurrent clients transfer independently using a shared service connection. |
| <a id="86-ac2"></a>86.AC2 | [#86 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/86#acceptance-criteria) | Target failure, peer loss, retry and shutdown follow approved policy; no silent replay. |
| <a id="86-handoff"></a>86.HANDOFF | [#86 TCP decision handoff](https://github.com/naicoi92/iroh-tunnel/issues/86#issuecomment-6490222782) | Confirmed policy: target failure closes only the affected channel; peer loss closes affected channels without replay. New clients retry with backoff under a bounded default deadline. On shutdown, stop accepting new clients, drain active channels for a bounded period, then close remaining channels. Determine and validate numeric defaults, cancellation, repeated-stop and error mapping as technical implementation work; illustrative values in #76 are not approved. Test real client and target behavior end to end. |
| <a id="87-ac1"></a>87.AC1 | [#87 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/87#acceptance-criteria) | Competing all-in-one and role binaries admit at most one owner per role and scope; different roles can coexist. |
| <a id="87-ac2"></a>87.AC2 | [#87 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/87#acceptance-criteria) | Status, crash recovery, stale state and cleanup follow approved ownership policy in real process tests. |
| <a id="87-cl1"></a>87.CL1 | [#87 clarification, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/87#issuecomment-6491342704) | Test same-role/same-scope races across all-in-one and role binaries, including foreground/user service and path overrides. Distinct roles coexist when their identities differ. |
| <a id="87-cl2"></a>87.CL2 | [#87 clarification, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/87#issuecomment-6491342704) | A second local endpoint using the same public identity is rejected even across roles or scopes; ownership artifacts must not expose raw keys. No cross-host intervention is implied. |
| <a id="87-cl3"></a>87.CL3 | [#87 clarification, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/87#issuecomment-6491342704) | Status identifies the actual role and owner; stale state, crash recovery, cleanup and cross-user security follow the approved #77 policy and are verified with real processes on applicable platforms. |
| <a id="88-ac1"></a>88.AC1 | [#88 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/88#acceptance-criteria) | Real clients receive correctly associated interleaved responses; delivered payload and datagram boundaries are unchanged. |
| <a id="88-ac2"></a>88.AC2 | [#88 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/88#acceptance-criteria) | Serve uses a dedicated upstream socket per active flow; shared Iroh connection is allowed and best-effort semantics are documented. |
| <a id="88-cl1"></a>88.CL1 | [#88 clarification, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/88#issuecomment-6491343233) | Document and verify target-visible Serve-side source endpoint behavior, including possible NAT rewriting and stability only for the lifetime of the upstream socket; do not promise original client IP transparency. |
| <a id="88-cl2"></a>88.CL2 | [#88 clarification, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/88#issuecomment-6491343233) | Verify interleaved clients across Access peers as well as clients sharing a peer, preserving opaque datagrams and response association. Expiry, filtering and tuple-reuse policy remain gated by #71/#73/#74/#78. |
| <a id="89-ac1"></a>89.AC1 | [#89 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/89#acceptance-criteria) | Exercise expiry, reconnect, late packets, source filtering, oversized datagrams and resource exhaustion with observable outcomes. |
| <a id="89-ac2"></a>89.AC2 | [#89 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/89#acceptance-criteria) | Verify direct and forced-relay paths and document tuple-reuse residual risks; do not claim absolute isolation. |
| <a id="90-ac1"></a>90.AC1 | [#90 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/90#acceptance-criteria) | Real supported-platform service installation, start, stop, restart, status and uninstall use the selected absolute paths. |
| <a id="90-ac2"></a>90.AC2 | [#90 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/90#acceptance-criteria) | Missing permissions fail with guidance; no silent scope or account change. |
| <a id="90-cl1"></a>90.CL1 | [#90 clarification, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/90#issuecomment-6491349469) | Verify non-elevated interactive installation offers user/system choice. Selecting system without privilege stops with actionable rerun guidance, preserves intended arguments/paths and never auto-elevates or silently falls back. |
| <a id="90-cl2"></a>90.CL2 | [#90 clarification, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/90#issuecomment-6491349469) | Verify real user-service lifecycle and pinned absolute configuration/state paths on each applicable supported OS. Windows mechanism remains gated by #79/#92. |
| <a id="91-ac1"></a>91.AC1 | [#91 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/91#acceptance-criteria) | Non-elevated system request stops with rerun guidance; elevated defaults retain safety confirmations. |
| <a id="91-ac2"></a>91.AC2 | [#91 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/91#acceptance-criteria) | Pinned paths and retargeting preserve both users' configurations; enforce and test approved ownership, symlink and integrity protections. |
| <a id="91-cl1"></a>91.CL1 | [#91 clarification, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/91#issuecomment-6491350008) | Elevated installation defaults to system scope without a scope prompt while retaining safety confirmations. Unprivileged system selection stops without auto-elevation/fallback and preserves rerun arguments and paths. |
| <a id="91-cl2"></a>91.CL2 | [#91 clarification, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/91#issuecomment-6491350008) | Retargeting another user requires confirmation; declining leaves service and both users’ files unchanged. Unknown user requires interactive selection or actionable non-interactive failure, never silent root-home substitution. |
| <a id="91-cl3"></a>91.CL3 | [#91 clarification, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/91#issuecomment-6491350008) | Authorized user edits followed by authorized administrator restart remain supported: validate configuration, report failures and do not silently change runtime account, ownership or permissions. Test approved unauthorized-writer, symlink and trust policy after #79. |
| <a id="92-ac1"></a>92.AC1 | [#92 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/92#acceptance-criteria) | Verify real installation, startup, status and stop on Windows with the approved user-background mechanism. |
| <a id="92-ac2"></a>92.AC2 | [#92 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/92#acceptance-criteria) | Exercise intended user paths and permissions; disclose unavailable Windows architecture coverage. |
| <a id="93-ac1"></a>93.AC1 | [#93 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/93#acceptance-criteria) | Shell and PowerShell install flows default to system-wide selection of all three binaries. |
| <a id="93-ac2"></a>93.AC2 | [#93 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/93#acceptance-criteria) | PATH changes and file conflicts require confirmation; exercise actual installation behavior on supported platforms. |
| <a id="93-cl1"></a>93.CL1 | [#93 clarification, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/93#issuecomment-6491350588) | Exercise default three-binary system-wide selection and actual PATH/conflict confirmation and decline behavior on supported shell and PowerShell platforms. Verify selected artifacts and aliases correspond to the intended binaries. |
| <a id="93-cl2"></a>93.CL2 | [#93 clarification, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/93#issuecomment-6491350588) | If the installer registers or changes a service, apply the approved #79/#91 service trust, identity and pinned-path policy; no service policy is implied merely by publishing binaries. |
| <a id="94-ac1"></a>94.AC1 | [#94 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/94#acceptance-criteria) | Each package and image contains the intended standalone artifact and runs its role independently. |
| <a id="94-ac2"></a>94.AC2 | [#94 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/94#acceptance-criteria) | Verify package and container ownership on supported platforms including Windows release artifacts where applicable. |
| <a id="94-cl1"></a>94.CL1 | [#94 clarification, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/94#issuecomment-6491351057) | Verify separate package and image ownership, contents, aliases and independent execution for each role on the supported release matrix. Distinguish cross-build evidence from real runtime execution. |
| <a id="94-cl2"></a>94.CL2 | [#94 clarification, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/94#issuecomment-6491351057) | If packages include service registration or unit files, verify composition with the approved #79/#91 account, trust and pinned-path policy. Binary-only package work need not wait for a service mechanism. |
| <a id="94-cl3"></a>94.CL3 | [#94 clarification, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/94#issuecomment-6491351057) | Real signed artifact production and promotion are tracked separately by #100; verify final release artifact identities in #97. |
| <a id="95-ac1"></a>95.AC1 | [#95 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/95#acceptance-criteria) | Verify CI identity-pinned keyless signature and checksum; failure preserves the old binary absent an explicit warned bypass. |
| <a id="95-ac2"></a>95.AC2 | [#95 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/95#acceptance-criteria) | Exercise approved running-process, rollback and Windows replacement behavior without silently interrupting a tunnel. |
| <a id="95-cl1"></a>95.CL1 | [#95 clarification, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/95#issuecomment-6491351657) | Verify default latest stable selection and explicit version selection against real update behavior; only the selected executable changes. |
| <a id="95-cl2"></a>95.CL2 | [#95 clarification, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/95#issuecomment-6491351657) | Together with #96, exercise actual update entrypoints for standalone, package-owned and container-owned artifacts and aliases; managed artifacts must never be silently overwritten by the standalone updater. |
| <a id="95-cl3"></a>95.CL3 | [#95 clarification, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/95#issuecomment-6491351657) | Confirm no automatic stopping of a running tunnel. Verification failure preserves the installed executable absent an explicit warned bypass. Replacement, recovery and trust rotation remain gated by #80. Validate actual signed producer artifacts from #100 before release. |
| <a id="96-ac1"></a>96.AC1 | [#96 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/96#acceptance-criteria) | Package-owned artifacts are updated through the corresponding package manager; container update is image redeployment. |
| <a id="96-ac2"></a>96.AC2 | [#96 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/96#acceptance-criteria) | Legacy bundled package migration does not silently update unrelated binaries. |
| <a id="96-cl1"></a>96.CL1 | [#96 clarification, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/96#issuecomment-6491352161) | Exercise package, container and standalone invocation paths together with #95, including aliases and legacy bundled migration: only the intended manager or image redeployment owns managed updates; unrelated binaries remain unchanged. |
| <a id="96-cl2"></a>96.CL2 | [#96 clarification, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/96#issuecomment-6491352161) | Package detection, alias resolution and partial failure behavior require approved decisions where unresolved. Do not implement guessed ownership policy. |
| <a id="97-ac1"></a>97.AC1 | [#97 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/97#acceptance-criteria) | Exercise all three binaries, TCP/UDP, services, installers and updates on supported OS runners, including real Windows x64 and ARM64 where available; disclose gaps. |
| <a id="97-ac2"></a>97.AC2 | [#97 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/97#acceptance-criteria) | Verify direct and forced-relay selected paths and record actual benchmark and E2E evidence; validate the selected Iroh upgrade from the current lockfile and report compatibility. |
| <a id="97-cl1"></a>97.CL1 | [#97 clarification, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/97#issuecomment-6491352677) | Every confirmed #68 story and rule maps to an owned issue, executable acceptance scenario and evidence; unresolved decisions remain explicit blockers for affected work. Review `docs/design/redesign-traceability.md` and `docs/design/redesign-acceptance.md` when available. No implementation issue is accepted on unit tests alone. |
| <a id="97-cl2"></a>97.CL2 | [#97 clarification, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/97#issuecomment-6491352677) | Run final-artifact end-to-end regression for all three binaries and aliases, long/short options, quick/configured modes, TCP and UDP, role/identity ownership, service lifecycle and trust, installers, packages/images, standalone and managed updates, signed artifact verification and cross-feature composition. Preserve released relay authentication, secret redaction, relay selection and status/path reporting unless an explicit approved change says otherwise. |
| <a id="97-cl3"></a>97.CL3 | [#97 clarification, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/97#issuecomment-6491352677) | Bind evidence to final source commit, lockfile, artifact digest, OS/architecture, privilege, command and observed result. Evidence affected by a later source, dependency or artifact change is stale and must be rerun. Separate cross-build, real runtime and privileged-service evidence. |
| <a id="97-cl4"></a>97.CL4 | [#97 clarification, checkbox 4](https://github.com/naicoi92/iroh-tunnel/issues/97#issuecomment-6491352677) | Enumerate preserved released platforms and Windows x64/ARM64 artifacts and actual runtime coverage. Missing environments or evidence are UNVERIFIED/BLOCKED, never PASS. Full redesign retains UDP and verified direct/forced-relay paths; any release exception requires an explicit decision. |
| <a id="97-cl5"></a>97.CL5 | [#97 clarification, checkbox 5](https://github.com/naicoi92/iroh-tunnel/issues/97#issuecomment-6491352677) | Include #99 configuration lifecycle, #100 actual signed release production and #101 selected Iroh upgrade. #97 must remain blocked until these and all existing native blockers are resolved. |
| <a id="98-ac1"></a>98.AC1 | [#98 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/98#acceptance-criteria) | Inventory existing configuration commands, first-run and missing-config behavior, validation and persistence, concurrent edits, identity rotation, and secret display/redaction; trace each to confirmed requirements or clearly mark it as proposed or unresolved. |
| <a id="98-ac2"></a>98.AC2 | [#98 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/98#acceptance-criteria) | Investigate platform/security consequences and present concrete options for unresolved validation, atomicity and conflict handling, setup, key rotation and disclosure; retain existing confirmed migration safeguards. |
| <a id="98-ac3"></a>98.AC3 | [#98 body, Acceptance criteria, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/98#acceptance-criteria) | Record decisions and independently testable scenarios, assign follow-up implementation ownership and dependencies, and preserve unresolved decisions as blockers for only the dependent work. |
| <a id="99-ac1"></a>99.AC1 | [#99 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/99#acceptance-criteria) | Implement each approved configuration and identity lifecycle decision from #98, with explicit traceability to the affected commands, data and acceptance scenarios. |
| <a id="99-ac2"></a>99.AC2 | [#99 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/99#acceptance-criteria) | Exercise real commands and process restart against actual configuration files and identities, including applicable failure, concurrent-edit and secret-handling cases defined by the approved policy. Record commands and evidence; disclose unsupported environments. |
| <a id="99-ac3"></a>99.AC3 | [#99 body, Acceptance criteria, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/99#acceptance-criteria) | Preserve confirmed migration and identity safeguards and document any intentionally changed existing behavior. |
| <a id="100-ac1"></a>100.AC1 | [#100 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/100#acceptance-criteria) | Produce each intended standalone role/all-in-one artifact for the supported release matrix with checksum and keyless signature or verification bundle bound to the approved CI issuer and identity. |
| <a id="100-ac2"></a>100.AC2 | [#100 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/100#acceptance-criteria) | Exercise real artifact production and verification using the verifier integrated by #95, including wrong identity, changed payload and missing or invalid verification material; record immutable artifact identifiers and outcomes. |
| <a id="100-ac3"></a>100.AC3 | [#100 body, Acceptance criteria, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/100#acceptance-criteria) | Gate public promotion on the complete approved artifact set and final #97 evidence. Keep signing rotation and recovery decisions governed by #80; do not invent unapproved bypass or replacement policies. |
| <a id="101-ac1"></a>101.AC1 | [#101 body, Acceptance criteria, checkbox 1](https://github.com/naicoi92/iroh-tunnel/issues/101#acceptance-criteria) | Record selected stable version, selection evidence, source/lockfile and compatibility constraints; implement required API and dependency updates without silently dropping supported behavior or platforms. |
| <a id="101-ac2"></a>101.AC2 | [#101 body, Acceptance criteria, checkbox 2](https://github.com/naicoi92/iroh-tunnel/issues/101#acceptance-criteria) | Run actual TCP/relay/status compatibility and other affected regression scenarios on the selected final version and supported environments; link results to exact commit, lockfile and artifacts. Record failures and unsupported environments. |
| <a id="101-ac3"></a>101.AC3 | [#101 body, Acceptance criteria, checkbox 3](https://github.com/naicoi92/iroh-tunnel/issues/101#acceptance-criteria) | Feed the selected final version into UDP benchmarking and #97 final-artifact verification; rerun affected evidence after any later dependency or artifact changes. |

## Planning coverage and execution inventory

57 anchored procedures map all 72 story requirements and all 21 confirmed rules; many-to-many links appear in the current trace tables. Every case has source AC and owner. 29 cases are NOT_RUN and 28 are BLOCKED with reasons; zero PASS/FAIL evidence is asserted. These counts describe planning only. Gated cases do not approve unresolved policy or add dependencies to independent work. No executable test code is added by this documentation change.

#97 release-critical scope includes configuration lifecycle (#98 decisions/#99 implementation), actual signed release production (#100), selected Iroh upgrade (#101), all existing native blockers and retained regressions. Baseline platform matrix: Linux x86_64/aarch64 glibc and x86_64/aarch64/riscv64 musl; macOS aarch64; existing Linux packages and amd64/arm64 images; Windows x64/ARM64 artifacts and real runtime where available. Record actual package/service applicability, missing environments and explicit exceptions. No benchmark threshold, Windows container promise, automatic tunnel stop or root-only config policy is introduced.

## Historical audit inventory (preserved, not current case IDs or execution results)

The following audit prose and issue-level matrix predate the catalog. Their grouped evidence suggestions, source-line references and historical blockers are not the current keyed mapping or approvals. The catalog/source register and current trace tables resolve navigation; live tracker supersession governs disagreements. In particular #75/#76 are resolved decisions, TCP numeric defaults are technical #86 work, process E2E and short arguments are required, and no #81 worker report is accepted evidence. Historical `NOT_RUN / BLOCKED` phrases mean absence of acceptance, not a case execution status.

### Historical implementation ticket AC → observable evidence matrix #81–97

Every row cites the corresponding issue body and `docs/design/redesign-spec.md` rule ranges. Blockers listed are historical; live native dependencies are authoritative. Each issue AC is covered, including policy-dependent cases without assuming outcome.

### #81 — quick-run TCP; open blockers: none
**AC C** (#81; TC-01, CLI-01–02/TCP-01, spec 5–16, 40–44, 72–76): quick flags start Serve and Access; fixed named target; real client exchanges bidirectional bytes, TCP default; ephemeral identity and invalid input visible. Add only seams needed by path.
**Evidence:** launch real separate Serve/Access commands, TCP echo target and real client without protocol switch; confirm exact bidirectional payload and target address/service mapping. Repeat restart without supplied key; compare displayed public identity to establish ephemeral behavior, verify no unintended config or key persistence. Supply invalid address/port/peer/flag and capture useful diagnostic, failure/exit and no functioning listener. Record default/help. **Gap:** no real-process redesigned quick-run evidence; short flags are required; exact exit-code semantics remain subject to the existing contract. NodeId is public identity, not authorization: document target-side protection, do not imply secrecy grants access.

### #82 — independent binaries and CLI; blocker #81
**AC C** (#82; CLI-01–03, spec 20, 72–76; plan 42–47): all-in-one and Serve/Access standalone binaries each run supported role without other binary installed; quick-run and supplied persistent key; consistent, non-conflicting short options, complete help/defaults; agreed aliases `it`, `its`, `ita`.
**Evidence:** isolated dirs/PATH individually containing only executable under test; test all-in-one Serve and Access plus standalone Serve and Access, pairing each with independent peer fixture; actual target bytes. Restart with same supplied key and verify public identity stable and connection works; no key default separately checked #81. Invoke long/short equivalent commands for **each of three binaries and both all-in-one roles**, compare parsed behavior and equivalent options, invalid/duplicate/conflicting options, role-specific unsupported options, defaults and help. Verify each alias resolves to intended binary without dependence on others. Short flags are mandatory; #81 implementation uses -n, -t, -l, -N and -p and #82 must keep equivalent options consistent. **Gap:** three standalone artifacts and process/option matrix not verified; exact letter allocation and diagnostic contract D.

### #83 — config and legacy multiplex; blocker #81
**AC C** (#83; CFG-03, AC-CFG-04, TCP-01, spec 24–30, 40–44): configured Serve and Access actually forward TCP; legacy `multiplex` values silently ignored, removed on save, no setting disables multiplex.
**Evidence:** real configured roles + echo fixture; load legacy true/false/omitted values, capture absence of legacy warning, run two overlapping independent client streams and observe one shared service connection; invoke real configuration write and inspect persisted file for field removal, valid other fields/key preserved, restart and retest. Include config parse/write errors as observable without assuming unapproved validation rules. **Gap:** current source still models `multiplex` and false selects old connection behavior (`src/config.rs:138–154`, `src/access.rs:641–649`); existing false-behavior test contradicts desired target and must be revised rather than treated as acceptance.

### #84 — paths and macOS migration; blocker #83
**AC C** (#84; CFG-01–02, spec 24–30; plan 63–68): macOS/Linux config `~/.config/iroh-tunnel`, state `~/.local/state/iroh-tunnel`, ignore XDG overrides; Windows LocalAppData config/state; explicit app overrides. macOS new config wins without consulting legacy; otherwise no-overwrite copy, preserve original and secret permissions; copy failure reports and never generates substitute identity.
**Evidence:** real executable processes on Linux/macOS/Windows with distinct HOME/LocalAppData/XDG and explicit override paths, inspect actual file and state paths; on macOS put conflicting public identities in new and legacy config and make legacy unreadable to prove new-first; legacy-only copy preserves source hash, secret access permissions and peer identity across restart. Induce destination failure/unreadable source/permission error and concurrent migration racers, inspect destination/source/key and diagnostic: no overwrite and no replacement identity; do not assert unapproved synchronization mechanism. Bad/unusable home/inaccessible config gets actionable failure. **Gap:** exact legacy path and first-time missing-config policy D; genuine platform ACLs and Windows LocalAppData required. `src/main.rs:83–92` currently resolves via `dirs::config_dir`, not the full target contract.

### #85 — TCP 0.4 interoperability; blocker #81 (#75 closed)
**AC C** (#85, #75 resolution; TC-02, spec 11–16): real 0.4 Access → redesigned Serve and redesigned Access → real 0.4 Serve; real target exchange and exact version/outcome record; test other supported combinations as applicable. Any failed direction: document compatibility trade-off and request user decision before breaking release; no new version-negotiation framework.
**Evidence:** obtain authentic versioned 0.4 executable(s), record version/hash/platform; run independent old/new processes and same TCP target/client for both directions, including multiple channels where applicable; capture bytes, exits/logs and supported peer matrix. If artifacts or platform unavailable explicitly mark untested, not pass. If incompatible, preserve logs and user decision artifact; do not imply incompatibility automatically forbids release or claim compatibility based on protocol source. **Gap:** both direction tests explicitly unperformed in #85/#75.

### #86 — TCP multiplex, failure, shutdown; blocker #81 (#76 closed)
**AC C** (#86 and [handoff](https://github.com/naicoi92/iroh-tunnel/issues/86#issuecomment-6490222782); #76 resolution is authoritative; spec 42–44, 94–95): concurrent clients transfer independently over one service connection; target failure only affected channel; peer loss closes affected channels without replay; new clients retry with backoff bounded by default deadline; shutdown stops new accepts, bounded drain then closes remaining.
**Evidence:** launch processes with observable connection identity/count, 2+ overlapping clients with distinct bidirectional markers; close/fail only one target connection, remaining client continues. Kill peer mid-transfer, assert affected clients terminate and target never sees replayed old markers; new client while unavailable observes repeated backoff and eventual bounded failure, another after peer return succeeds. Stop with one in-flight completing inside drain and one intentionally held beyond drain; verify no new accept, successful drain and forced closure, plus cancellation/repeated stop according to approved technical contract. Measure elapsed behavior; do not enshrine illustrative 30s/5s values. **Gap:** exact retry/drain numeric defaults, cancellation, errors and tests not established; connection sharing needs external observation, not merely successful payloads. Existing test only in-process and ignored.

### #87 — role and identity ownership/status; blockers #77, #82
**AC C** (#87; #77; RO-01–02, spec 32–38): competing all-in-one/role binaries admit at most one process per role among the system service and same-user modes, different roles coexist with distinct identities; foreground/user service same role/user exclude each other independent of path overrides, and `run`/`service start` are refused while the system service runs; duplicate local public identity rejected even across roles/scopes; no raw key in artifacts; status identifies actual role/owner. Crash, stale status and cleanup follow approved policy.
**Evidence:** barrier-race real `it serve` vs `its`, and Access equivalents, assert exactly one proceeds; vary config/state overrides and service vs foreground. Different roles with distinct keys coexist; same key across roles rejects the later process with actionable guidance. Capture actual status owner before/after stop, abrupt kill/restart, stale snapshots, PID reuse and ensure cleanup never deletes other live owner's state; examine locks/status/logs for raw secret; start `run` and `service start` while the system service runs and assert refusal with nothing stopped. **Gap:** a non-elevated user's ability to detect the system service is verified in #87; no cross-host or cross-user check is implied. Different roles coexist only when identity ownership also satisfied.

### #88 — UDP data path; blockers #78, #81
**AC C** (#88; #71, UDP-01–03, spec 46–52): multiple clients to fixed target; correctly associated interleaved responses, unchanged opaque payload and datagram boundaries; dedicated Serve upstream socket per active flow; shared Iroh connection allowed; document best-effort delivery, source endpoint effects.
**Evidence after #78 approval:** real UDP clients A/B send distinct binary/empty/variable-length datagrams, target deliberately interleaves replies and records source endpoints; assert delivered payload and message boundaries, proper recipient, distinct simultaneous upstream sockets and stable endpoint during each socket lifetime. Include two Access processes/NodeIds and same-node clients so peer identity is not mistaken for client identity; confirm actual Serve-side target source and document NAT/reconnect limits. Verify docs do not claim ordering/reliability/deduplication/original-client IP. No need to require shared connection if merely allowed. **Gap:** no approved transport or full UDP run path; reliability under lossy conditions must not be asserted.

### #89 — UDP limits and lifecycle; blockers #78, #88
**AC C** (#89, #71–74, #78; spec 48–52, 96–97): exercise approved expiry, reconnect, late replies, source filtering, oversized datagrams and exhaustion; verify actual direct and forced-relay paths and document tuple-reuse residual risk, not absolute isolation.
**Evidence after decisions:** real endpoints/target induce idle expiry/reconnect, old and new flow IDs, delayed old target responses after OS tuple reuse and different-source replies; check actual approved outcomes and prevent partial datagrams masquerading as complete. Inject oversized messages near measured limits; apply bounded flow/socket/queue pressure, slow consumer and, only if selected, fragment loss/duplicate/reassembly attacks; record output/drop/resource cleanup against approved limits. Repeat verified selected direct and forced relay under loss, congestion and path change; measure CPU/memory/loss/latency. **Gap:** values, source policy, fragmentation, lifecycle and loss/drop diagnostics D; no benchmark, cannot claim cross-reuse perfect isolation or require a particular drop policy prematurely.

### #90 — user service; blockers #84, #87
**AC C** (#90; SM-01–03, spec 54–62): install/start/stop/restart/status/uninstall on supported platform using selected absolute config/state paths; privilege/permission failure gives guidance, no silent scope/account change.
**Evidence:** actual OS service manager as ordinary user, inspect registered command/absolute paths, start and pass real client bytes, query status, stop/restart and uninstall; restart under changed home/CWD to prove pinned locations and persistent identity. Attempt inaccessible state/config/registration, assert actionable error and unchanged permissions/account/scope. Distinguish foreground role conflict under #87. **Gap:** actual supported-OS service environments and paths not established; Windows user mechanism #92/#79 still D; do not assume Unix unit mocks prove platform behavior.

### #91 — system service and privilege boundary; blockers #79, #84, #87
**AC C** (#91; SM-01–05, spec 54–62): unprivileged system request stops with sudo/elevated-terminal guidance, no auto-elevation or fallback. Elevated default system skips scope prompt but keeps safety confirmation. Pin installing user's absolute paths; confirm retarget to different user, preserve both config files. Missing/unknown user → interactive selection or noninteractive guidance/failure. Authorized edit + admin restart allowed; validate and don't silently change permissions/ownership/runtime account. Approved security policy must protect ownership, symlink and integrity.
**Evidence:** on real supported OS use separate normal/admin users; capture non-elevated error and no system/user registration; elevated install default/prompt transcript, actual registration paths and account, run target exchange. Set up existing other-user service, decline then approve retarget and compare registration plus both file hashes/ownership; test unknown-user interactive and noninteractive paths. Edit managed config as authorized user, restart as admin, confirm changes applied; invalid schema fails with clear error and unchanged ownership. Under #79 approved threat model simulate unprivileged unauthorized write, hostile parent-directory replacement/symlink and TOCTOU races; verify safe outcomes and actual service privilege. **Gap:** trust and runtime account policy open; no choice of root-only files, symlink algorithm or integrity mechanism approved.

### #92 — Windows logged-in user background; blockers #79, #84, #87
**AC C** (#92; SM-04, #68 story 53): real Windows installation, startup, status and stop using approved logged-in user mechanism; intended user paths/permissions, disclose architecture coverage.
**Evidence after #79:** actual Windows login under non-admin user, install chosen background mechanism, inspect registration/account and absolute LocalAppData/override paths, launch and exchange data with real peer/target, status and stop; attempt denied access and confirm guidance/no elevation/account change, same-role foreground conflict #87. Run x64 and ARM64 where available and record physical/virtual architecture and gaps; no assumption mechanism is SCM, scheduled task or any other specific adapter. **Gap:** mechanism unresolved; mock/compilation is not Windows lifecycle proof.

### #93 — shell/PowerShell installers; blocker #82
**AC C** (#93; DIST-01/AC-DIST-01,05, spec 64–70): shell and PowerShell default system-wide selection of all three binaries, confirmation before PATH mutation or file conflicts; actual installation on supported OS.
**Evidence:** clean disposable real Linux/macOS shell and Windows PowerShell environments with appropriate privilege; exercise default prompts and verify all 3 executable artifacts/aliases and each independent role. Seed conflicting file and PATH; decline then accept each prompt, compare prior files/PATH and installed versions; verify failure on insufficient permission does not silently change scope. Capture transcript and environment cleanup. **Gap:** real platform installers and consent paths not verified; exact noninteractive policy not decided.

### #94 — packages, images and Windows artifacts; blocker #82
**AC C** (#94; DIST-01, spec 66–70): separate packages and three all-in-one/Serve/Access images contain intended standalone artifact and independently run role; package/container ownership; Windows releases where applicable.
**Evidence:** build/fetch each versioned package and image; inspect manifest/content, install each alone into clean platform, run its role with peer and real client/target, verify no runtime dependency on other role binaries. Exercise all-in-one both roles, standalone roles, image entrypoints, package-manager ownership/query and container immutable ownership; examine Windows x64/ARM64 artifacts and launch on real hosts where available, state applicability and missing architecture explicitly. **Gap:** no package/image/platform evidence asserted; do not assume all packaging formats exist or Windows container support is promised.

### #95 — verified standalone update; blockers #80, #82
**AC C** (#95; DIST-02, spec 66–70): invoked standalone executable only, latest stable default and explicit version; CI issuer/identity-pinned keyless signature plus checksum; failed verification retains old executable absent explicit warned bypass; no automatic running-tunnel stop; approved recovery/Windows replacement.
**Evidence:** test each invoked binary against controlled signed release with known issuer/identity, stable vs explicit version and compare hashes of all three; independently tamper binary/checksum/signature, use wrong issuer and identity, simulate failed fetch, assert old executable intact and usable with no silent replacement. Exercise explicit bypass, capture warning and verify exact allowed scope once #80 decides it. Keep a real tunnel transferring during updater; assert no automatic stop, capture guidance and approved replacement/recovery behavior. Inject replacement/partial-write failure and test rollback per approved policy on real OS incl Windows locked executable, test trust rotation when specified. **Gap:** exact bypass/rollback/rotation/Windows behavior open; staging/atomicity not approved; avoid fabricating identities, release source or checksum-bypass rule.

### #96 — managed ownership; blocker #94
**AC C** (#96; DIST-01, spec 66–70): package-owned artifacts update through corresponding manager; containers update by image redeploy; legacy bundled package migration guidance prevents silent updates of unrelated binaries.
**Evidence:** install actual role-specific packages, invoke update and observe correct manager guidance/delegation, package DB/version and unchanged unrelated artifacts; verify no independent overwrite of managed binary. Run container and invoke applicable update path: no self-mutation, actionable pull/redeploy guidance, then redeploy new image and verify role + bytes. Install actual legacy bundle, follow documented migration, capture package-manager changes and compare all binary hashes; no silent unrelated update. **Gap:** detection and legacy alias mechanics open; proper platform packages/container runtime must be available; mocks alone insufficient.

### #97 — cross-platform complete release; blockers #84–96 excluding #83 (native list in snapshot)
**AC C** (#97; #68 stories 70–72, plan 93–109): release candidate end-to-end all three binaries, TCP/UDP, services, installers and updates across supported OS; real Windows x64/ARM64 where available with gaps disclosed; verified selected direct/forced relay, real benchmark/E2E; validate selected Iroh upgrade from current lockfile and report compatibility.
**Evidence:** produce traceable per-OS/arch matrix linking #81–96 evidence, artifact checksums/versions, real execution and pass/fail/untested for every required combination; Linux/macOS/Windows installation → quick/config role tunnel → service lifecycle → independently verified update, actual security/permission scenarios and both TCP directions with 0.4. Exercise UDP across direct/forced-relay with path proof, adversarial flow/lifecycle/resource cases and measured latency/jitter/loss/throughput/CPU/memory under varied payload/flows/impairment, noting method and actual limits. Read current `Cargo.lock`, recheck stable target/availability, build and run selected Iroh version, record protocol/0.4 and path compatibility; do not assume 1.4.1 remains stable/latest. Windows x64/ARM64 actual evidence where available, unavailable environment disclosed with closest alternative and not marked passed. Security, UDP deferral and decision gates remain release-critical. **Gap:** #97 readiness explicitly says no full E2E or benchmarks; successful subset cannot close full release gate. Do not invent quantitative performance thresholds.

## Cross-cutting gaps and integration inventory

1. **Baseline reality, not E2E proof:** `Cargo.toml:9–15` declares only all-in-one binary; `src/main.rs:52–80` dispatches configured role commands and `src/cli.rs:37–76` requires role subcommand. Source snapshot is not proof no files exist elsewhere; it establishes declared entrypoint and visible interface. `src/pipe.rs:26–33,136–179` labels UDP codec/pipe as awaiting handlers; payload framing code alone is not generic real UDP forwarding. `src/config.rs:138–154` and `src/access.rs:641–649` retain legacy multiplex toggle. Distribution, role ownership, migration, service trust and Windows behavior need real-path audit; no sweeping claim of absence merely from filenames.
2. **Existing prior art not acceptance:** `tests/serve_access_tunnel.rs:1–33,65–66,357–361,400–404,431–432` contains in-process real-TCP fixtures but public relay cases ignored, including legacy `multiplex=false` behavior contrary to target; `tests/access_status_file.rs:1–21,48–49` uses in-process roles/local relay and pure renderer rather than spawning CLI. No tests were executed in this recon; existence does not establish current success. Update/replace old assumptions while retaining regression coverage that remains valid.
3. **Security/evidence trace:** NodeId not authorization (#68 TC-03); test service target protection/documentation, not a fabricated tunnel ACL. Never leak raw keys in locks/status/logs; local identity uniqueness and service config trust are different security boundaries (#77/#79). User-editable config and authorized restart cannot be rejected as hostile by default. Signatures need pinned issuer AND identity plus checksum, deliberate warned bypass only; review checksum bypass semantics separately (#80).
4. **Persistence/concurrency:** distinguish quick ephemeral vs provided stable identity; save/reload and migration source/destination/permissions; perform barrier races for config migration and role acquisition, no-overwrite guaranteed but concurrent config mutation general behavior remains proposal (spec 99–105). Status stale/PID-reuse exact response D. Service path pinning survives changed working directory/user home and restart. Update negative paths preserve hashes and unrelated artifacts.
5. **UDP/release sequencing:** #78 explicitly defers UDP design/benchmark/implementation until independent non-UDP work, not permission to omit #97. #70/#72 close research, not policy or runtime proof. Per-flow mapping confirms active-flow routing, not absolute safety after OS tuple reuse. Direct/relay selection requires observable path evidence; public relay use is not automatically forced relay. No numbers were approved.
6. **Source ambiguity:** #75 original issue wording and older comments requesting wire negotiation are superseded by later resolution; #76 older proposed isolation superseded by confirmed lifecycle; #68 shared comment/spec line 44 contains stale `AC-TCP-04` proposal wording despite TCP-02 confirmation; #68 body 'open gates' lists old wire/retry wording despite later supersession. Prefer explicit later decisions. #85/#86 audit comments may still list #75/#76 as open, while the audit snapshot recorded both as CLOSED; distinguish historical comments from state.
7. **Evidence packaging:** retain commands, actual executable and Iroh versions, test identities without secrets, hashes, platform/architecture, OS users, privileges, paths, observed selected network route, service/installer transcripts, failure artifacts, benchmark methodology/results, unresolved policy review and user decision. Record unavailable platforms or 0.4 artifacts as untested. No benchmark, compatibility, Windows release or actual full-E2E completion asserted by this report.

## Historical retained-functionality inventory

Owner #97, with dependency upgrade evidence from #101. Execute these against final binaries and record actual commands, artifact digests, platform and observed outcomes. These scenarios preserve existing contracts; they do not establish new authorization policy.

- **Relay authentication:** Real relay accepts the correct token, rejects wrong or missing tokens, including on-demand dialing; token is absent from logs and status. Source: [#53](https://github.com/naicoi92/iroh-tunnel/issues/53).
- **Relay selection:** Two services use distinct per-service relay routes; exercise service → node → default fallback and observe selected relay. Source: [#54](https://github.com/naicoi92/iroh-tunnel/issues/54).
- **Status/path reporting:** Exercise actual role status and path transitions, active/inactive transport reporting, candidate local addresses, JSON output and missing-status error; verify logs reflect path changes. Sources: [#57](https://github.com/naicoi92/iroh-tunnel/issues/57), [#58](https://github.com/naicoi92/iroh-tunnel/issues/58), [#59](https://github.com/naicoi92/iroh-tunnel/issues/59).

## Additional owners

- **#98 / #99:** decide and implement approved configuration and identity lifecycle, including actual commands, persistence, failure, concurrency and secret handling as policy requires.
- **#100:** produce and verify actual signed and checksummed artifacts; gate publication on complete accepted evidence.
- **#101:** select, implement and validate the approved Iroh upgrade, including affected regression on the final version.

## Evidence and release status

Each scenario records requirement and issue, command, exact source SHA, lockfile and artifact digest, OS/architecture, user/privilege, fixtures, actual selected network path where relevant, expected versus observed result, logs and timestamp. Status is `NOT_RUN`, `BLOCKED`, `PASS`, or `FAIL`; unavailable environments are `BLOCKED` or `NOT_RUN`, never `PASS`. Changes affecting the binary, dependency, configuration or artifact invalidate affected prior evidence. Separate cross-build from runtime and privileged-platform evidence.

Final #97 verification must cover three binaries and aliases, quick/configured CLI and short flags, TCP/UDP, retained relay authentication and status behavior, ownership, services, installers, packages/images, updater ownership and actual signed artifacts, plus direct and forced-relay paths. UDP remains required despite deferred sequencing.

Baseline release matrix to preserve and validate: Linux x86_64/aarch64 glibc and x86_64/aarch64/riscv64 musl; macOS aarch64; existing Linux package and amd64/arm64 container coverage; plus Windows x64 and ARM64 artifacts and real runtime coverage where available. Record actual supported package/service-manager combinations and any explicit exception; cross-compilation alone does not demonstrate runtime support.

All redesigned behavior remains `NOT_RUN` or `BLOCKED` in this ledger until independently bound evidence is accepted. #81 worker reported passing tests at `bc943290d8393658851995fa99592c54eb071c2d`, but independent review is BLOCK; no acceptance is asserted.
