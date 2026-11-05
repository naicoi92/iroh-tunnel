---
title: Responsibility-oriented workspace and reusable host operations
summary: Separate tunnel execution, host policy and CLI presentation while preserving existing operator, peer and lifecycle contracts.
---

## Context

This design implements the direction approved in [brainstorm.md](brainstorm.md) and [proposal.md](proposal.md). It is for the implementer and architecture reviewer; the existing eleven specifications remain the behavioral baseline. This document selects boundaries, ownership and migration strategy. Full Rust signatures and acceptance-to-test mappings belong in the later `plan.md` Contracts and test matrix, not in a second artifact here.

Today, `iroh-tunnel` is one Rust package whose library exposes nearly every implementation module. `config.rs` combines TOML documents, validation, key handling and storage; `config_cmd.rs` and `update.rs` accept clap types; `app.rs` contains service policy and prompts; `role_run.rs` prints startup output; and UDP observations construct status-file DTOs. These dependencies make the current library unsuitable as the supported management boundary for another frontend.

There are strengths to retain: the UDP decision model, serialized atomic configuration writes, privileged verified-handle loading, kernel-backed ownership checks, explicit service backends, supervised forwarding drain, and real-binary local-relay tests. The quarantine sink's lifetime beyond a connection is intentional; the missing retained task handle is an ownership concern, not proof of a forwarding defect. Similarly, derived secret-bearing `Debug` is a potential disclosure path, not evidence of an actual leak.

**Authorities:** `docs/GLOSSARY.md`, `docs/udp-integration-contract.md`, `docs/agents/rust-coding-standards.md` (RS-01–RS-16), the living specs, and dependency versions in `Cargo.toml`. Preserve Rust 2021/MSRV 1.91 and the established `anyhow`/`CliError` strategy. The approved internal-workspace exception applies only to this change; no external dependency addition or edit to `openspec/config.yaml` is implied.

## Goals / Non-Goals

**Goals:**

- Use Cargo and Rust visibility to enforce cohesive responsibility boundaries without introducing a framework.
- Let another Rust frontend manage configuration, registered OS services and role status without clap, terminal input or parsing human output.
- Keep registered background services independent of the caller's lifetime; retain foreground CLI execution as a different operation.
- Make secret handling, task ownership, cancellation and per-run state explicit while retaining observable semantics.
- Preserve build, test, installer and release behavior through independently verifiable migration increments.

**Non-Goals:**

- No TUI/Tauri implementation, IPC DTOs, daemon, remote control protocol or general embedded-runtime product API.
- No adoption or termination of arbitrary foreground processes, automatic elevation, new service scope or broader platform support.
- No new forwarding behavior, stricter accepted configuration, correction of known UDP deviations, or blanket rewrite.
- No crate per role/capability/layer, generic repository/transport framework, CQRS, event bus, new dependency or new lint gate.

## Decisions

### 1. Use three packages with one production dependency direction

Keep the existing **root package `iroh-tunnel`** as the CLI/distribution composition root, and add exactly two internal packages under `crates/`:

| Package | Owner and cohesive modules | Must not own |
|---|---|---|
| `iroh-tunnel-engine` at `crates/engine` | Tunnel settings and resolved inputs, protocol/ALPN, endpoint construction, Serve/Access forwarding, pipes, UDP decisions/queues, connection observations and per-run task/drain state | TOML/files, OS registrations, process signals/exit, clap or terminal rendering |
| `iroh-tunnel-host` at `crates/host` | Configuration documents/store/use cases, path resolution, identity persistence, trust and ownership, foreground orchestration, status-file adapters, service operations/backends, updater | clap parsing, terminal questions, tables or process exit |
| `iroh-tunnel` at repository root | Three binaries, CLI argument translation, prompts/editor invocation, rendering, tracing setup, Tokio construction, signals/watchdog and exit mapping; build/release integration | Duplicated persistence, privilege, trust, ownership or forwarding policy |

The production graph is `iroh-tunnel → iroh-tunnel-host → iroh-tunnel-engine`. The CLI does not need a direct engine dependency: host-owned management types and selected runtime bridge types provide its required surface. Test-only dependencies may exercise engine directly without reversing the production graph. Neither lower package depends on the root package, including through dev-dependencies.

Keep root `Cargo.toml` as both package and workspace manifest, root `build.rs`, root binary targets and root process/distribution `tests/`. Add members and set `resolver = "2"`; explicitly include all three packages in `default-members`. Keep versions synchronized, inherit edition/MSRV/license/version from workspace metadata, and share dependency declarations without enabling test features there. Internal packages are not separately published as a new supported SDK. Keep the three binary names and root `install.sh`/`install.ps1`, Dockerfiles, packaging and release entrypoints.

Within each package, group by responsibility, for example engine `runtime`, `tcp`, `udp`, `observation`, and host `config`, `service`, `status`, `runtime`, `update`. **File convention (user decision):** inside `crates/engine` and `crates/host`, each leaf file owns exactly one public operation (one crate-exported `pub fn`/`pub async fn`) together with its private helpers and its `#[cfg(test)] mod tests`; methods on that operation's own request/outcome types stay with it. Shared types live in a sibling `types.rs`, private infrastructure (store, lock, native runner) in its own file, and `mod.rs` only declares and re-exports. Private helpers are not split into separate files, and the convention does not impose domain/application/infrastructure directories. A private two-role strategy can remain where it genuinely removes duplication.

**Why this split:** A management caller links host policy without linking clap, prompts or CLI process behavior. Engine cannot reach back into persistence or service code. Keeping the root application package avoids needless disruption to package selection, `CARGO_BIN_EXE_*`, build version injection and installer-relative fixtures. Shared types alone do not justify a fourth domain crate; updater and service backends do not justify more crates in this change.

**Alternatives rejected:** retaining one package was rejected in brainstorming; two packages combining engine with host would leave the runtime-to-file coupling unenforced; a virtual root moving the CLI under `crates/` would create avoidable distribution/test churn; an engine/host/CLI plus contracts crate would enlarge the public surface without an independent responsibility.

### 2. Treat DDD boundaries as modules and invariants, not crate labels

Serve and Access participate in one tunnel domain, not two bounded contexts. Keep `FlowId`, `Admission`, `ClientFlows`, `PeerFlows`, session-incarnation rules, caps, quarantine accounting, injected time inputs and framing together in engine. Iroh and sockets are concrete adapters around those decisions inside that same package; there is no generic transport interface.

Host modules own three related policy areas: configuration/identity and safe mutation; host registration/privilege/trust; and role ownership/status interpretation. Updater ownership and verified replacement remain a cohesive supporting module. Their cooperation is use-case orchestration, not a reason to introduce aggregate repositories or a universal manager object.

Use private fields and semantic enums for role, protocol, scope, consent, disclosure and operation state. Introduce newtypes only when they protect an existing invariant. Store paths as `PathBuf`, borrow as `&Path`, and use `Duration` for effective intervals. Do not replace hostnames with `SocketAddr`, parse every NodeId earlier, or enforce stricter URL/name normalization simply because a stronger type is available.

**Validation stages are compatibility:** a config currently checked for NodeId length at load must still load under that rule; full endpoint-id parsing remains at the corresponding runtime stage. Preserve unknown-field acceptance, defaults, legacy `multiplex` handling/removal, service order and existing error classification. Engine inputs distinguish raw-compatible settings from values resolved for execution; their constructors must not silently move failure earlier. Authority conflicts stop the affected migration rather than being resolved by a newtype.

### 3. Separate configuration format, pure decisions and storage operations

Host `config` owns serde/TOML DTOs, `toml_edit` transformations, format-compatible validation, redaction and the concrete locked store. Engine owns execution settings, protocol and effective UDP limits; host maps documents into these inputs. DTOs have no filesystem methods, and engine settings have no TOML representation or persistence behavior. Preserve raw text/`DocumentMut` where required instead of round-tripping all commands through typed serde DTOs.

Expose concrete host operations corresponding to the existing paths: resolve/path, add, remove, list, show, generate/rotate identity, and edit-copy/commit. They accept non-clap requests and return data/outcomes, not printed messages. No generic `ConfigRepository` is needed: use real isolated files for store tests and pure functions for transformations.

| Concern | Host decision and outcome | CLI responsibility |
|---|---|---|
| Add/remove | Validate and mutate under the existing lock; return affected service/path | Map arguments and print existing messages |
| List/show | Preserve tolerant list behavior and raw/reformatted show behavior; redact by default | Render the existing table or returned text |
| Identity | Return public NodeId; distinguish first generation from rotation requiring explicit consent; recheck under lock | Preserve prompt/`--yes`/non-TTY refusal and warning text |
| Edit | Prepare owner-only sibling copy and original bytes; compare current file under lock, validate and atomically commit or preserve `.rejected` | Run `$EDITOR` using the existing editor integration, outside the lock |
| Random key | Generate through an explicit secret-returning operation without saving | Preserve the intentional key stdout contract |

An edit attempt owns scratch cleanup and original-version data; it holds no writer lock while the editor is open. Comparison remains against the original bytes/absence, not a newly invented hash or mtime policy. Invalid or conflicting edits retain the current rejection-file behavior and actionable path. Failed/cancelled attempts must not report a save or overwrite the live file.

All secrets use redacted `Debug`, including enclosing requests, DTOs, edit attempts and returned documents. Explicit reveal/random-key operations can return secrets because the existing commands require them; normal list/status/outcome values cannot expose them accidentally. Do not change TOML storage format, key encoding or introduce a secret-storage/zeroization dependency. Preserve adoption of a concurrently persisted key, owner-only modes, symlink policy, writer locking and atomic rename/fsync ordering.

**Alternative rejected:** passing clap-independent copies of the full CLI action enum into another dispatcher would move the coupling without defining useful use cases. Returning only pre-rendered tables would force future frontends to parse presentation.

### 4. Make service operations explicit, policy-enforcing and process-independent

Expose host operations for existing install/uninstall/start/stop/restart/status with a typed role, chosen scope, resolved paths and operation-specific decisions. They call the same platform backend selection and all current privilege, account, pin/trust, installability and ownership policies. Callers cannot invoke raw backend dispatch as the supported management API.

Scope and system-target resolution produce structured needs such as scope selection, invoking-user input or retarget confirmation when information is insufficient. CLI translates these into the existing prompts/defaults and rerun guidance. A non-terminal caller supplies the decision or receives an actionable outcome; host never reads stdin. Execution rechecks privilege, target ownership/trust and relevant registration facts before mutation. A prepared target is not an authorization token and does not excuse stale validation.

Retarget consent identifies the old and proposed pin/registration, not a general `yes` bypass; changes between inspection and execution require another decision or refusal. Keep existing safe write boundaries rather than inventing transactional rollback across the service manager. If registration succeeds but enable/start fails, return the failure with what was applied; do not pretend the operation was atomic or add rollback with different semantics.

**Tunnel executable identity is explicit.** Host installation takes the all-in-one tunnel executable path to register. It must never fall back to the management application's `current_exe()`. CLI preserves the current resolution rule (`iroh-tunnel` on `PATH`, otherwise its own absolute executable); the existing standalone-binary refusal stays in CLI. Another frontend must locate/supply the actual all-in-one binary. This prevents registering a TUI/Tauri application itself as the tunnel without adding an executable-discovery product.

Keep platform facts distinct:

- Linux selects systemd versus BusyBox/SysV at runtime. Preserve BusyBox's root requirement, system-wide script and explanatory handling of user scope, rather than inventing per-user support.
- macOS keeps launchd domains, labels, restart/stop behavior and registered paths.
- Windows keeps the logged-in user's Task Scheduler task, user-only scope, locale-independent ownership probe, XML/security and quoting rules; do not substitute a Windows Service or machine scope.
- Unsupported combinations remain unsupported. No self-elevation, account switching or silent fallback is added.

**Status is structured without inventing truth.** A service report separates registration/backend facts, kernel-backed role liveness, available owner metadata and backend diagnostics. Manager/task state is not proof that a tunnel runs; a live foreground role is not proof that a registration launched it. Represent unavailable/indeterminate fields explicitly, including no owner record. Supplemental fact-gathering must not turn an otherwise successful native CLI status call into a new failure; retain its existing required checks and record optional failures as unavailable facts. Do not parse localized `schtasks`/`launchctl`/`systemctl status` prose into portable truth or add arbitrary process controls.

Use a narrow child-process adapter with explicit output handling. Non-terminal calls capture stdout/stderr and process status without writing to the caller's terminal. CLI requests inherited native streams for commands whose existing behavior forwards native output, retaining TTY/pager behavior and backend-specific nonzero-status rules (for example stopped systemd status is not a CLI failure). Return captured diagnostic bytes and status where applicable, plus semantic effects/notes for CLI rendering. Notes that precede a native command use a narrow synchronous observer at that stage; collecting everything only on success would lose ordering and partial-failure diagnostics. The default non-terminal observer collects data without terminal output. Never normalize native diagnostics into an inaccurate universal running/stopped line. Keep stdout/stderr routing and message ordering in process-level regression tests.

**Service lifetime:** the OS registration owns the background process. Install currently enables/starts and uninstall stops/disables/removes; preserve these composite semantics. Dropping a request, result, host object or UI must not issue stop/uninstall or signal the role. Only an explicit operation with those existing semantics changes service lifetime. `start` still performs its current ownership refusal before the manager call; `restart` retains backend stop/start behavior. No lifecycle lease, kill-on-drop child or embedded role handle is returned.

**Alternatives rejected:** controlling arbitrary PIDs or reusing test shutdown handles violates scope and ownership; a daemon/control socket is unnecessary; a trait per backend adds no value when compile-time platform modules and Linux runtime selection already suffice.

### 5. Use finite host operations, with explicit blocking and cancellation behavior

Configuration/store and existing OS-service operations remain synchronous finite operations. Frontends call them on an owned blocking worker, not a Tokio/UI thread; the CLI does the same when dispatching from its async runtime. Foreground execution and HTTP update remain async. Do not build a nested runtime or assume a global executor inside host.

Use narrow seams only for actual effects requiring control: native command execution/output, account/environment facts already injected in tests, status persistence and runtime shutdown. Prefer concrete structs, functions and closures; preserve an existing verifier trait where multiple real/test implementations already use it. No general plugin registry or `Arc<Mutex<Host>>` is required. Capture per-operation facts without mutating process environment, and revalidate security-sensitive facts at commit.

Cancellation before the first side effect leaves live configuration and registrations unchanged; preparing an edit copy is itself a tracked side effect with scratch cleanup. Once blocking storage or a service-manager command has begun, dropping an awaiter is **not rollback or cancellation**. The spawning owner must retain/observe completion; this work never sends an implicit compensating stop. Operations return confirmed effects and failures, not a promise of transactional cancellation. Closing a UI may leave an already issued OS operation completing, but cannot couple the service lifetime to that UI. Do not add new service-command timeouts or retries that alter current semantics in this refactor.

### 6. Keep runtime ownership while reversing the status dependency

Host foreground orchestration retains the production startup sequence: claim the role before configuration mutation, load through the applicable trust path, resolve/persist identity off async workers, check/publish a configured public identity before endpoint bind, run the role, and hold the claim through shutdown and cleanup. Quick mode remains ephemeral unless a key is explicitly supplied and never creates a config. Do not expose an ownership-bypass option in the supported host entrypoint.

Engine receives execution inputs, a per-run drain context and narrow output hooks. It owns endpoints, listeners, connection caches, streams, UDP readers/writers/sinks and the complete task tree. Runtime startup emits a typed ready description for CLI rendering instead of `println!`/`RoleStrategy::print_services`. The callback runs at the existing startup stage before forwarding admission; its completion/error behavior and CLI I/O compatibility must be pinned in Contracts, with owned startup cleanup rather than detachment on failure. Retain tracing diagnostics/log contracts rather than turning every log into a new event framework.

**Status port, not a reverse package edge:** engine builds native observation values (service order, stream counters, UDP counters, peer paths and current limits), not host JSON DTOs. Keep the initial write-before-admission ordering, current approximately five-second/change-only flush loop, initial-success seeding and retry-on-write-failure behavior. Observation equality must preserve the old rendered-value change detection, not include a fresh sampling timestamp or omit a schema-relevant field. Supply the existing async-save seam as an injected callback whose host implementation maps observations to the unchanged file schema and awaits its blocking write. There is no unbounded event queue, new polling daemon or engine dependency on host.

Keep the flush task owned by the run supervisor. Across success and error paths, the completion information identifies whether a writer has quiesced; host never guesses this from `Result::is_ok()`. On graceful stop, seal admission, share one drain deadline, join forwarding descendants, cooperatively stop/join status writes, close the endpoint and only then remove the owned status file when safe. Preserve the current five-second flush-join bound and the stale-file fallback: aborting the async wrapper cannot prove an in-flight blocking save stopped, so a timed-out/failed join does not authorize removal. The process-level forced-exit path retains its existing best-effort own-file cleanup contract; do not claim it has graceful quiescence guarantees.

Quarantine sinks move under a **role-lifetime owner**, not back under the connection's join scope: they must outlive the session incarnation as sinks but not outlive role shutdown. Retain and reap their handles, observe failures and join/abort remaining sink tasks against the existing role stop deadline. Slot release remains guarded on every exit. Preserve quarantine duration, caps, late-reply behavior and wire deviations; do not wait a full quarantine period per connection at disconnect.

Replace the process-global first-wins `DRAIN_DEADLINE` with one fresh context for each foreground invocation, shared by its engine task tree and CLI watchdog observer. Keep the same drain/watchdog budgets, repeated-signal policy and forced exit codes. Signal installation, runtime shutdown timeout, final error rendering and `process::exit` belong to CLI. Management-only calls install no signal handlers and cannot trigger or inherit another run's drain state. A running foreground future still owns its own resources; this internal bridge is not a supported multi-role embedded-runtime API.

The production CLI awaits orderly completion unless its existing forced-exit policy fires. Unexpected future drop takes a best-effort nonblocking abort/close path through owned guards; it cannot async-join descendants or cancel started blocking writes in `Drop`. No ordinary destructor claims graceful cleanup or unconditionally removes a status file. Keep the forced process-exit exception explicit and test it separately from cooperative stop; another management frontend is not given this foreground lifetime to own.

**Alternatives rejected:** moving status schemas into engine preserves accidental coupling; a fire-and-forget observation channel loses initial-write/join guarantees; lifting all task supervisors into host duplicates forwarding ownership. The callback is justified by the existing save seam and concrete host persistence adapter, not hypothetical extensibility.

### 7. Separate status reads and updater use cases from rendering

Host role-status reads return the exact validated raw body when present, a typed snapshot, independently probed liveness/owner data and a freshness condition. Distinguish live-without-snapshot, absent-and-not-running, unreadable/invalid file, stale pid, missing owner metadata and current snapshot. Reading status must not generate keys, create a role or start a service.

CLI owns the existing tables, stale warnings, live-without-file JSON object, error text and broken-pipe tolerance. `status --json` continues forwarding the validated original bytes with the existing line behavior, not serializing the typed value. Host owns schema/serializer/writer compatibility, change detection inputs and pid-checked cleanup; engine observations are not a replacement public JSON schema. Future frontends can refresh the same read operation; no new subscription mechanism is promised.

Move updater implementation and ownership classification into host `update`, replacing `UpdateArgs` with operation inputs and outcomes. CLI keeps invoked-executable selection, flags, warnings and presentation. Preserve the existing download/signature/checksum bounds, verifier, package-manager/container delegation, explicit overwrite and insecure policies, version selection and atomic/platform-specific replacement. The updater's target executable is explicit too: host must not discover and overwrite the UI executable. Preserve the update user-agent/product version when code moves packages. Updater reuse is not service restart; running tunnels are not stopped automatically.

### 8. Expose narrow Rust contracts and preserve error classification

The supported management boundary is the host crate's documented config/service/status operations. Expose only the request/result/value types those callers need; keep OS command construction, trust handles, lock internals and raw backend dispatch private. Engine's host-facing runtime inputs, output hooks and per-run drain observation are Rust cross-crate contracts needed by this implementation, not a promise of a general SDK. Existing in-process network tests move to their owning package or an explicitly gated test-support seam; no testing escape hatch becomes a production management API.

Continue using `anyhow::Result` for application orchestration, per RS-06. Move `CliError` and preserve its source/downcast identity through host context so the root CLI still maps config/permission/Iroh/service/conflict to 2/3/4/5/6, unclassified failure to 1 and success to 0. Engine does not import a CLI error type; host classifies at the same existing operation boundaries, not every engine failure as exit 4. Add typed decisions/outcomes where callers need to act without parsing a message; do not replace every error with a new universal enum or flatten contextual errors to strings.

Every widened `pub` item gets a caller, ownership/cancellation/error rustdoc and an old-to-new migration entry where relevant. Public values use redacted `Debug`; action alternatives use semantic enums instead of ambiguous booleans. Exact fields, signatures, conversion rules and error/outcome mappings must be recorded in `plan.md` Contracts before implementation. There is no Tauri serialization/IPC commitment in this change.

### 9. Preserve test discovery, feature safety and distribution identity

Keep root real-binary/installer/release integration tests with the package defining their binary targets. Move private engine tests to engine and host store/trust/service/status tests to host. Move in-process network tests according to the entrypoint they verify; do not keep production APIs public merely to avoid adapting a test. Put shared local-relay helpers in the owning tests/support modules, not an extra support crate. Update fixture-relative `include_*` paths where owners move; retain root-relative installer and release fixtures where they already fit.

Forward the existing `test-utils` feature explicitly from root through host to engine. No default/normal dependency or shared workspace declaration may enable it. Remove or restructure the existing root self-dev-dependency so extraction creates no package cycle; keep local-relay test enablement explicit and preserve opt-in/ignored cases. Engine's TLS override and test environment hooks remain feature-gated. Check both the normal release feature graph and the test feature graph; never use `--all-features` binaries against public relays or as production artifacts.

Use workspace-wide variants of existing gates to avoid accidentally testing only the root package:

```sh
cargo +1.91 fmt --all -- --check
cargo +1.91 clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo +1.91 test --workspace --all-features --locked
```

Also verify default-feature compilation, selected package boundaries, binary target discovery and release builds without test features. These are migration checks, not new lint policy. Inventory discovered test names/ignored cases before and after moves: counts are a loss detector, not the only proof, because intentional splitting can change totals. Use requirement-linked tests and actual CLI/config/status/network/shutdown flows to prove behavior; fake native-command tests alone cannot prove a platform lifecycle.

Update source/test/manifests path filters in CI/snapshots/cache warming, Docker build contexts, package selection, docs/rustdoc links, fixtures and release scripts in the same migration increment that requires them. Keep root `APP_VERSION` behavior and consistent product version for all three binaries/packages. In particular, replace build-workflow assumptions about a literal root `[package].version` before inheriting workspace version, and include member manifests/source in Docker dependency/build stages. Preserve target output paths, executable aliases, archive/checksum/signature names, package identities, installer URLs and existing release-smoke/native platform entrypoints. Do not move an externally referenced root file merely to make the tree look uniform.

## Risks / Trade-offs

| Risk | Mitigation |
|---|---|
| Workspace extraction increases cross-crate public surface and build coordination | Exactly two internal packages; justify each export; one-way graph and a migration map; keep cohesive modules private |
| New runtime types reject inputs earlier or normalize externally visible values | Pin current acceptance/failure stages and golden formats first; convert only at the existing stage; stop on conflicts |
| Status inversion changes readiness ordering, flush retry or cleanup races | Retain engine-owned callback loop and initial barrier; explicit quiescence on every completion path; slow-write/process shutdown tests |
| UI executable is registered or overwritten by ambient `current_exe()` | Require explicit tunnel/update executable in host; keep CLI resolution at the executable boundary |
| Structured service reporting invents liveness or loses native diagnostics | Separate registration, ownership and diagnostics; preserve inherited CLI mode; allow unknown facts; never parse localized prose for truth |
| Closing a UI drops an in-flight blocking operation | Document non-transactional cancellation; operation owner observes completion; no stop-on-drop or automatic rollback |
| Quarantine handles accumulate or connection cleanup shortens sink lifetime | Role-owned reaping supervisor and guarded slot release; tests cover disconnect, role stop and cap accounting |
| Error conversion changes an exit code or leaks a secret | Preserve host `CliError` through context; CLI binary assertions; redacted nested Debug/error/output tests |
| Feature unification leaks insecure TLS into release or creates a dev-dependency cycle | Inspect normal/test feature graphs separately, no default test features, release build verification and safe local relays only |
| Tests disappear under Cargo discovery or CI path filters | Inventory test names/targets, explicit workspace gates, root binary tests and updated filters/fixtures |
| Platform/service/artifact verification is unavailable locally | Keep cfg compile and fixture tests, run actual opt-in native lifecycle/release checks when available, record blocked coverage without claiming E2E success |
| Configured service state is not transactional across registration and manager actions | Preserve current order and failure semantics; report confirmed partial effects rather than adding unsafe compensations |

## Migration Plan

Implement this as ordered, reviewable increments inside the same change; the later tasks/plan will assign exact files and tests.

1. **Pin contracts before moving owners.** Inventory public Rust callers, discovered tests, feature paths, binary/build entrypoints and CI/distribution path assumptions. Add requirement-linked characterization tests for affected presentation/exit/acceptance/lifecycle seams. Record the starting source/API map and baseline limits.
2. **Establish boundaries in the current package.** Separate clap translation, prompts/rendering and editor invocation from config/service/update use cases; separate DTO/store/execution data; introduce the per-run drain context and status callbacks without changing wire/files. Verify each seam through its actual CLI/network path before extraction.
3. **Extract engine.** Move cohesive forwarding/protocol/endpoint/UDP/observation/task owners and their tests. Adapt host-facing orchestration and status persistence to the callback boundary; retain role-owned quarantine sinks. Enforce no engine dependency on root/host and no file-schema knowledge.
4. **Extract host.** Move config/store/identity/trust/ownership/status/service/updater and their tests; promote only documented use cases, not raw backend or test bypasses. Preserve CLI output adapters and the existing root package/binary names. Ensure each intermediate commit builds and has no reverse dependency.
5. **Complete workspace integration and migration documentation.** Update manifests/lockfile, default members, feature forwarding, fixtures, filters, Docker/packaging and documentation with their corresponding moves. The map must identify at least the destinations below and all caller-visible API changes.
6. **Verify the completed dependency and behavioral contracts.** Run workspace/default-feature/MSRV gates and real-binary local-relay/config/status/ownership/shutdown coverage. Exercise actual supported service managers and Windows updater workflows where available; keep signed-release/public-relay checks explicitly blocked unless their safe harness/artifacts are available. Compare test inventory and normal release features, not just a green root test command.

| Current owner/path | Destination and API migration direction |
|---|---|
| `src/config.rs` | Host `config` DTO/validation/identity plus engine execution settings; replace DTO load/save methods with host operations |
| `src/config_store.rs`, `src/config_cmd.rs` | Host `config` store/use cases; CLI prompts/editor/rendering remain root; replace clap inputs with concrete requests |
| `src/service/**`, service policy in `src/app.rs` | Host `service` policy/backends/trust; root maps prompts/output and supplies tunnel binary; raw backend functions become private |
| `src/status.rs`, `src/status_cmd.rs` | Host `status` schema/store/read interpretation; root renderers; return data and preserve raw JSON rather than printing in host |
| `src/role_run.rs`, `src/ownership.rs`, `src/paths.rs` | Host foreground/ownership/path policy; engine receives execution inputs/stop context and emits ready/observation values |
| `src/serve.rs`, `src/access.rs` | Engine role loops/task owners, host production startup facade, root startup rendering; test bypass entrypoints move to owned tests/test-support |
| `src/endpoint.rs`, `src/proto.rs`, `src/pipe.rs`, `src/udp.rs`, `src/conn_path.rs` | Engine transport/domain/observation modules; host maps native observations to file DTOs |
| `src/shutdown.rs` | Engine run-local drain/task mechanics; CLI signal/watchdog/exit policy via host bridge; remove process-global first-wins state |
| `src/update.rs`, `src/update_owner.rs`, `src/error.rs` | Host update/ownership/error modules; root flags/presentation/version/target selection; preserve downcast classification |
| `src/cli.rs`, `src/app.rs`, three binary entrypoints, `build.rs` | Root CLI/composition/build; remove duplicate policy, retain binary identities and process contract |
| Root `tests/` and fixtures | Keep real-binary/distribution tests root; relocate in-process/private tests with their owners and update fixture paths explicitly |

**Deploy/rollback:** no user data, TOML, status schema, service registration format or wire migration is required. Existing registrations continue launching the same binary command; existing running services are not automatically restarted by this refactor. Roll back a failed implementation increment by reverting its source/manifests/caller/test/path changes together. A deployed rollback uses the prior compatible release and the existing explicit update/restart procedures, not a new automated service stop or data rewrite. Rust callers use the documented map or revert the consuming change with the implementation; a compatibility facade is optional during migration, not a reason to retain unsafe broad exports permanently.

## Open Questions

No product-level decision remains open within the approved scope. The three-package graph, management-versus-foreground distinction, explicit executable identity, status callback ownership and per-run lifecycle strategy are this design's reviewable decisions.

Exact signatures/fields and the exhaustive API migration/test matrix are deliberately deferred to `plan.md` Contracts; specs will define the two new capability contracts without modifying the existing eleven. These are scheduled artifacts, not authorization to code now. Native Linux/Windows/macOS service-manager availability and signed-release fixtures are verification prerequisites to record, not assumptions of coverage. Any discovery requiring a new dependency, changed operator/peer behavior, stronger validation, expanded support or control protocol returns for an explicit decision before implementation.
