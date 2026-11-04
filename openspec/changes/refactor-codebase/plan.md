# Responsibility-Oriented Workspace Refactor Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` (`workspace-boundaries`, `frontend-independent-management`) and `design.md`; task IDs from `tasks.md`.

**Goal:** Split `iroh-tunnel` into a three-package workspace (`iroh-tunnel → iroh-tunnel-host → iroh-tunnel-engine`) and expose non-terminal host operations for config, OS services, role status and update, while leaving every observable contract in the eleven existing specs unchanged.

**Architecture:** First establish boundaries inside the current package: per-run drain context, owned tasks, typed ready/observation values, and host-style operations that take requests and return outcomes. Then move cohesive modules into `crates/engine` and `crates/host`. The root package keeps clap, prompts, rendering, signals, watchdog and exit mapping. Every step stays green against the baseline characterization from Tasks 1–3.

**Tech Stack:** Rust 2021, MSRV 1.91, Cargo workspace (resolver 2), Tokio, Iroh 1.4.1, clap 4.6, serde/serde_json, toml/toml_edit, tracing, anyhow + `CliError` (thiserror), reqwest/rustls, Sigstore verifier. No new external crates.

## Global Constraints

- Toolchain: `cargo +1.91`; `--locked` for clippy and test; edition 2021; `rust-version = "1.91"` inherited by all three packages.
- No new external crate in `Cargo.lock` (WB-3). Internal packages have `publish = false`.
- Production graph: root → host → engine only; no reverse edge, including dev-dependencies (WB-2).
- `test-utils` is forwarded explicitly root → host → engine → `iroh/test-utils`; it is never default and never in `[workspace.dependencies]` (WB-18). Never run `--all-features` binaries against public relays (RS-13/RS-15).
- Exit codes: 0 success, 1 unclassified, 2 Config, 3 Permission, 4 Iroh, 5 Service, 6 Conflict; forced exit 130 (repeated signal after grace) and 1 (watchdog) (WB-15, WB-8).
- Budgets unchanged: `DRAIN_GRACE = 3s`, `EXIT_WATCHDOG = 10s`, `STATUS_FLUSH_INTERVAL = 5s`, flush-join bound 5s, Quarantine duration `udp::QUARANTINE`.
- CLI stdout/stderr text, ordering and the `status --json` byte passthrough are byte-identical to the Task 2 golden files.
- Host never reads stdin, prints, installs signal handlers or calls `process::exit`. Engine never touches files, OS services, signals, `process::exit` or stdout/stderr.
- Secret-bearing types use a hand-written redacting `Debug` that prints `[redacted]`. The TOML format and base64 key encoding are unchanged.
- Repository standards RS-01–RS-16 in `docs/agents/rust-coding-standards.md` override generic advice. Use TDD per task, and commit at the end of each task.
- File convention (design.md Decision 1): in `crates/engine` and `crates/host`, each leaf file owns exactly one crate-exported `pub fn`/`pub async fn` plus its private helpers and `#[cfg(test)] mod tests`. Shared types go in `types.rs`, private infrastructure in its own file, and `mod.rs` only declares and re-exports. Tasks 4–18 edit the existing `src/` files; Tasks 19–20 land each operation in its own file.
- Keep the glossary terms from `docs/GLOSSARY.md` (Serve, Access, Flow, Session incarnation, Quarantine, Flow cap, Datagram limit). Known UDP deviations stay as they are.

## Review Focus

Five inputs or conditions that the specs imply but no scenario directly exercises. Each has a test in its owning task:

1. **Non-UTF-8 config/state paths** passed through the new `PathBuf` request types must be handled exactly as the CLI handles them today. → `add_service_accepts_non_utf8_config_path` (Task 10, unix only).
2. **Editing a config that does not exist yet**, where another writer creates it before commit, must be rejected as a conflict, not overwritten. → `edit_of_missing_config_conflicts_if_created_meanwhile` (Task 12).
3. **A failing ready hook**: the role must close its endpoint and release its ownership claim, and must not leave tasks detached. → `ready_hook_failure_closes_endpoint_and_releases_claim` (Task 8).
4. **A legacy registration without a pin** must keep today's retarget policy rather than being treated as the same user. → `retarget_from_unpinned_registration_matches_existing_policy` (Task 14).
5. **A stop signal before the ready report**: the claim is released, no status file remains and the exit code is 0. → `stop_before_ready_releases_claim_and_leaves_no_status_file` (Task 4).

---

## Contracts

The Path column gives each item's **final** location. Until Tasks 19–20 move them, items live in the interim file named in the owning task. "public" means `pub` at the crate root of the final package; `crate` means `pub(crate)` or private. Every public item gets RS-02 rustdoc covering ownership, cancellation and errors.

### Engine (`iroh_tunnel_engine`)

| Name | new / modified | Shape | public |
|---|---|---|---|
| `RunContext` | new (replaces `static DRAIN_DEADLINE`) | `#[derive(Clone)] pub struct RunContext(Arc<watch::Sender<Option<Instant>>>)`; `fn new() -> Self`; `async fn drain_deadline(&self) -> Instant` (pending until the first drain starts, returns that drain's deadline); `pub(crate) fn publish(&self, at: Instant)` (first publication wins) | public |
| `shutdown::begin` | modified | `pub(crate) fn begin(stop: &watch::Sender<Option<Instant>>, ctx: &RunContext)` | crate |
| `Protocol` | modified (serde removed) | `pub enum Protocol { Tcp, Udp }`; `fn as_str(self) -> &'static str` (`"tcp"`/`"udp"`) | public |
| `NodeSettings` | new | `pub struct NodeSettings { pub secret_key: Option<SecretKey>, pub relay_urls: Vec<String>, pub relay_token: Option<RelayToken>, pub max_concurrent_streams: Option<u32> }`; redacting `Debug` | public |
| `RelayToken` | new | `pub struct RelayToken(String)`; `fn new(String) -> Self`; `fn expose(&self) -> &str`; `Debug` prints `RelayToken([redacted])` | public |
| `UdpLimits` | modified (moved) | same fields as today; `fn from_overrides(idle_timeout: Option<Duration>, max_flows: Option<usize>, max_flows_per_peer: Option<usize>, queue_depth: Option<usize>) -> UdpLimits` applies today's defaults | public |
| `ServeSettings` / `ServeServiceSettings` | new | `{ pub node: NodeSettings, pub services: Vec<ServeServiceSettings> }`; `ServeServiceSettings { pub name: String, pub protocol: Protocol, pub host: String, pub port: u16, pub udp: UdpLimits }` | public |
| `AccessSettings` / `AccessServiceSettings` | new | `{ pub node: NodeSettings, pub services: Vec<AccessServiceSettings> }`; `AccessServiceSettings { pub name, pub node_id: String /* parsed at the existing runtime stage */, pub protocol, pub host, pub port, pub relay_urls: Vec<String>, pub udp: UdpLimits }` | public |
| `ReadyReport` | new | `pub struct ReadyReport { pub node_id: String, pub home_relay: Option<String>, pub services: ReadyServices }`; `pub enum ReadyServices { Serve(Vec<ServeReady>), Access(Vec<AccessReady>) }`; `ServeReady { pub name, pub protocol: Protocol, pub local_addr: String }`; `AccessReady { pub name, pub listen_addr: String, pub peer_display: String, pub protocol: Protocol, pub relay_override_count: usize }` | public |
| `ServeObservation` | new (replaces engine use of `status::StatusFile`) | `#[derive(Clone, PartialEq, Debug)]` with fields identical to today's `StatusFile`: `node_id, home_relay, pid: u32, started_at: u64, services: Vec<ServiceObservation>, connections: Vec<PeerConnectionObservation>`. No serde. | public |
| `AccessObservation` | new | mirrors today's `AccessStatusFile`/`AccessServiceStatus` field-for-field, with `transports: Vec<TransportObservation>`. No serde. | public |
| `ServiceObservation`, `UdpObservation`, `UdpDropCounts`, `PeerConnectionObservation`, `PeerPathObservation`, `TransportObservation`, `TransportKind` | new / moved | field-for-field mirrors of `ServiceStatus`, `UdpStatus`, `UdpDrops`, `PeerConnectionStatus`, `PeerPathReport`, `TransportStatus`, `TransportKind` without serde; `UdpStats::snapshot() -> UdpObservation` | public |
| `StatusSink<O>` | new | `pub struct StatusSink<O> { save: Box<dyn FnMut(O) -> BoxFuture<'static, anyhow::Result<()>> + Send> }`; `fn new(save: impl FnMut(O) -> BoxFuture<…> + Send + 'static) -> Self`. `BoxFuture` = `Pin<Box<dyn Future<Output = T> + Send + 'a>>`, defined locally (no `futures` crate). | public |
| `RoleHooks<O>` | new | `pub struct RoleHooks<O> { pub on_ready: Box<dyn FnOnce(&ReadyReport) -> anyhow::Result<()> + Send>, pub status: Option<StatusSink<O>> }` | public |
| `RunCompletion` | new | `pub struct RunCompletion { pub result: anyhow::Result<()>, pub status_quiesced: bool }`. `status_quiesced` is `true` only when the flush task joined within 5s **and** returned `Ok`, or when no sink was configured. | public |
| `bind_serve` / `bind_access` | modified (from `create_*_endpoint`) | `pub async fn bind_serve(s: &ServeSettings) -> anyhow::Result<Endpoint>`; `pub async fn bind_access(s: &AccessSettings) -> anyhow::Result<Endpoint>` | public |
| `run_serve` / `run_access` | new (from `ServeStrategy`/`AccessStrategy::run_loop*`) | `pub async fn run_serve(ep: Endpoint, s: ServeSettings, ctx: RunContext, hooks: RoleHooks<ServeObservation>, shutdown: impl Future<Output = ()>) -> RunCompletion` (Access is analogous). Order: emit `on_ready` → initial status save → admission. The endpoint is closed before returning. | public |
| `node_id_string`, `home_relay` | unchanged (moved) | as today | public |
| `QuarantineSinks` | new | `pub(crate) struct QuarantineSinks { tasks: Mutex<JoinSet<()>> }`; `fn spawn(&self, fut)`, `fn reap(&self)` (observes finished sinks), `async fn shutdown(&self, deadline: Instant)` (join, abort the rest at the deadline, observe each result). Owned per role run. | crate |
| `testing` module | new | `#[cfg(feature = "test-utils")] pub mod testing { pub fn set_relay_only(on: bool); pub async fn hang_point(); }` (moved from `endpoint::set_relay_only_for_tests` and `shutdown::test_hang_point`) | public (test-utils) |

### Host (`iroh_tunnel_host`)

| Name | new / modified | Shape | public |
|---|---|---|---|
| `error::CliError` | modified (moved) | unchanged variants `Config, Permission, Iroh, Service, Conflict` (each `String`); `thiserror` messages unchanged | public |
| `Role` | modified (renamed from `status::StatusRole`) | `pub enum Role { Serve, Access }`; `fn name(self) -> &'static str`; `fn status_file_name(self) -> &'static str` | public |
| `config::{ServeConfig, AccessConfig, NodeConfig, ServeService, AccessService, UdpSettings}` | modified | Serde DTOs as today (unknown fields accepted, legacy `multiplex` ignored); `protocol` uses `#[serde(with = "protocol_serde")]` mapped to engine `Protocol`; redacting `Debug` on `NodeConfig`, `ServeConfig`, `AccessConfig`; no filesystem methods | public |
| `config::to_settings` | new | `pub fn serve_settings(c: &ServeConfig) -> anyhow::Result<ServeSettings>`; `pub fn access_settings(c: &AccessConfig) -> anyhow::Result<AccessSettings>`. They resolve `secret_key` with the existing `resolve_secret_key` (same error text, same stage: before bind). | public |
| `config::resolve_config_path` | unchanged signature, `role: Role` | `pub fn resolve_config_path(role: Role, override_path: Option<&Path>) -> anyhow::Result<PathBuf>` | public |
| `config::AddService` | new (replaces `cli::AddServiceArgs` in host) | `pub struct AddService { pub config: Option<PathBuf>, pub name: String, pub protocol: String, pub host: String, pub port: u16, pub node_id: Option<String> }` (raw strings keep today's validation stage and messages) | public |
| `config::Mutation` | new | `pub struct Mutation { pub service: String, pub path: PathBuf }` | public |
| `config::add_service` / `remove_service` | new | `pub fn add_service(role: Role, req: &AddService) -> anyhow::Result<Mutation>`; `pub fn remove_service(role: Role, config: Option<&Path>, name: &str) -> anyhow::Result<Mutation>` | public |
| `config::ServiceList` / `ServiceRow` | new | `pub struct ServiceList { pub path: PathBuf, pub rows: Option<Vec<ServiceRow>> }` (`None` means no `services` array, i.e. the "No services configured" line); `ServiceRow { pub name: Option<String>, pub node_id: Option<String>, pub protocol: Option<String>, pub host: Option<String>, pub port: Option<i64> }` (the CLI renders `?`/`0` as today) | public |
| `config::list_services` | new | `pub fn list_services(role: Role, config: Option<&Path>) -> anyhow::Result<ServiceList>` | public |
| `config::Disclosure` / `ShownConfig` / `show_config` | new | `pub enum Disclosure { Redacted, Reveal }`; `pub struct ShownConfig(String)` with redacting `Debug` and `fn text(&self) -> &str`; `pub fn show_config(role: Role, config: Option<&Path>, d: Disclosure) -> anyhow::Result<ShownConfig>` | public |
| `config::IdentityState` / `identity_state` | new | `pub enum IdentityState { NoKey, HasKey }`; `pub fn identity_state(role: Role, config: Option<&Path>) -> anyhow::Result<IdentityState>` (unlocked pre-check; an unparsable file reads as `NoKey`, as today) | public |
| `config::RotationConsent` / `KeygenOutcome` / `keygen` | new | `pub enum RotationConsent { NotGiven, Given }`; `pub enum KeygenOutcome { Generated { path: PathBuf, node_id: String, rotated: bool }, ConsentNeeded { path: PathBuf, current_node_id: Option<String> } }`; `pub fn keygen(role: Role, config: Option<&Path>, consent: RotationConsent) -> anyhow::Result<KeygenOutcome>` (decides under the writer lock) | public |
| `config::GeneratedIdentity` / `random_identity` | new | `pub struct GeneratedIdentity { secret: String, pub node_id: String }`; `fn secret(&self) -> &str`; redacting `Debug`; `pub fn random_identity() -> GeneratedIdentity` | public |
| `config::EditAttempt` / `prepare_edit` | new | `pub fn prepare_edit(role: Role, config: Option<&Path>) -> anyhow::Result<EditAttempt>`; `EditAttempt { path, scratch, original: Option<String> }` (private fields); `fn scratch_path(&self) -> &Path`; `fn commit(self) -> anyhow::Result<EditOutcome>`; `Drop` removes the scratch file; redacting `Debug` | public |
| `config::EditOutcome` / `EditRejection` | new | `pub enum EditOutcome { Unchanged, Saved { path: PathBuf }, Rejected { reason: EditRejection, sidecar: Option<PathBuf>, message: String } }`; `pub enum EditRejection { Conflict, Invalid }`. `message` is today's exact `CliError::Config` text. | public |
| `config::require_existing` | unchanged, `role: Role` | as today | public |
| `service::ServiceScope` | unchanged | `pub enum ServiceScope { User, System }` | public |
| `service::Environment` | modified | `pub struct Environment { pub elevated: bool, pub can_answer: bool }` (renamed from `interactive`: the caller can answer a decision); `fn detect() -> Self` for the CLI | public |
| `service::ScopeDecision` / `decide_install_scope` | new (pure part of `choose_install_scope`) | `pub enum ScopeDecision { Chosen(ServiceScope), NeedsChoice }`; `pub fn decide_install_scope(requested: Option<ServiceScope>, env: Environment) -> ScopeDecision` (explicit → that scope; elevated → System; `can_answer` → NeedsChoice; otherwise User). `parse_scope_answer(&str) -> anyhow::Result<ServiceScope>` keeps the existing error text. | public |
| `service::Pin` | unchanged (now public) | `pub struct Pin { pub config: PathBuf, pub uid: u32 }` | public |
| `service::RetargetConsent` | new | `pub struct RetargetConsent { pub previous: Option<Pin>, pub proposed: Pin }` (`previous: None` = consent for a legacy unpinned registration; deviation from `Pin` — needs controller decision) | public |
| `service::InstallRequest` | new | `pub struct InstallRequest { pub role: Role, pub scope: Option<ServiceScope> /* None → decide_install_scope */, pub config: Option<PathBuf>, pub tunnel_executable: PathBuf, pub sudo_user: Option<String>, pub invoking_user: Option<String>, pub state_dir_override: Option<PathBuf>, pub retarget: Option<RetargetConsent> }` (`tunnel_executable` must be absolute, otherwise `CliError::Service`) | public |
| `service::InstallOutcome` / `InstallNeed` | new | `pub enum InstallOutcome { Installed(ServiceReport), Needs(InstallNeed) }`; `pub enum InstallNeed { ScopeChoice, InvokingUser, Retarget { previous: Option<Pin>, proposed: Pin } }` (`ScopeChoice` when `decide_install_scope` yields `NeedsChoice`) (`previous: None` = legacy unpinned registration) | public |
| `service::OutputMode` / `ServiceCall` | new | `pub enum OutputMode { Capture, Inherit }`; `pub struct ServiceCall<'a> { pub output: OutputMode, pub notes: &'a mut dyn FnMut(&ServiceNote) }`; `ServiceCall::capture(notes)` helper | public |
| `service::ServiceNote` | new | `pub enum ServiceNote { BusyboxSystemWide { script: PathBuf }, LaunchdBootstrapHint { target: String, domain: String, plist: PathBuf }, Installed { name: String, at: InstalledAt }, Uninstalled { name: String }, AlreadyStopped { name: String }, RoleRunning { role: String, owner: String }, RoleNotRunning { role: String }, TaskListing { text: String } }`; `pub enum InstalledAt { Path(PathBuf), WindowsTask }`. Deviation (Task 13): the last three variants carry the Windows `status` lines (`{role} is running ({owner})`, `{role} is not running`, the verbatim `schtasks /Query` listing), which the five planned variants cannot express byte-identically | public |
| `service::NativeOutput` | new | `pub struct NativeOutput { pub program: String, pub status: Option<i32>, pub stdout: Vec<u8>, pub stderr: Vec<u8>, status_display: Option<String> }` (stdout and stderr are empty in `Inherit` mode; the private `status_display` carries std's `ExitStatus` Display text from the production runner so signal-killed errors keep their original wording) | public |
| `service::AppliedEffect` / `ServiceReport` / `ServiceFailure` | new | `pub enum AppliedEffect { RegistrationWritten(PathBuf), Enabled, Started, Stopped, Disabled, RegistrationRemoved(PathBuf) }`; `pub struct ServiceReport { pub applied: Vec<AppliedEffect>, pub native: Vec<NativeOutput> }`; `pub struct ServiceFailure { pub applied: Vec<AppliedEffect>, pub native: Vec<NativeOutput>, pub error: anyhow::Error }`; `impl From<ServiceFailure> for anyhow::Error` returns `error` unchanged so the `CliError` downcast still works | public |
| `service::install` | new (replaces `app::dispatch_service` install path + raw `service::install`) | `pub fn install(req: &InstallRequest, env: Environment, call: &mut ServiceCall) -> Result<InstallOutcome, ServiceFailure>` | public |
| `service::{uninstall, start, stop, restart}` | modified | `pub fn <verb>(role: Role, scope: ServiceScope, env: Environment, call: &mut ServiceCall) -> Result<ServiceReport, ServiceFailure>`. They enforce `ensure_supported` and `ensure_privilege` (existing rerun text). `start` calls `ensure_not_running` before the manager. | public |
| `service::Fact<T>` / `Registration` / `ServiceStatusReport` / `status` | new | `pub enum Fact<T> { Known(T), Unavailable(String) }`; `pub enum Registration { Present { location: String }, Absent }`; `pub struct ServiceStatusReport { pub registration: Fact<Registration>, pub liveness: Fact<Liveness>, pub owner: Fact<OwnerRecord>, pub native: Option<NativeOutput> }`; `pub fn status(role, scope, env, call) -> Result<ServiceStatusReport, ServiceFailure>` | public |
| backend modules (`systemd`, `busybox`, `launchd`, `windows`, `task_xml`, `trust`, `account`, `system`) | modified | private (`pub(crate)` at most); they take `&mut Native` and push `ServiceNote`/`AppliedEffect` | crate |
| `Native` | new | `pub(crate) struct Native<'a> { mode: OutputMode, runner: &'a mut dyn FnMut(&mut Command, OutputMode) -> io::Result<NativeOutput> }`; the production runner uses `.status()` for Inherit and `.output()` for Capture; tests inject a fake | crate |
| `status::Liveness` / `Tier` / `OwnerRecord` | modified (now public) | `pub enum Liveness { NotRunning, Running { tier: Tier, owner: Option<OwnerRecord> } }`; `pub enum Tier { User, Machine }` + `fn label(self)`; `pub struct OwnerRecord { pub role: String, pub pid: u32, pub node_id: Option<String>, pub started_at: u64 }` | public |
| `status::{StatusFile, AccessStatusFile, …, StatusPayload}` | unchanged schema (moved) | serde DTOs as today; `impl From<&ServeObservation> for StatusFile` and `impl From<&AccessObservation> for AccessStatusFile` | public |
| `status::RoleStatus` / `Snapshot` / `Freshness` / `read_role_status` | new | `pub struct RoleStatus { pub role: Role, pub path: PathBuf, pub liveness: Liveness, pub snapshot: Snapshot }`; `pub enum Snapshot { Absent, Present { raw: String, payload: StatusPayload }, Invalid(anyhow::Error) }`; `pub enum Freshness { Current, LiveWithoutSnapshot, AbsentNotRunning, Unreadable, StalePid { live_pid: Option<u32> }, OwnerUnknown }`; `fn freshness(&self) -> Freshness`; `pub fn read_role_status(role: Role) -> anyhow::Result<RoleStatus>` (`Err` only for path resolution and probe failures, as today) | public |
| `runtime::run_foreground` | new (from `role_run::run_skeleton/run_config`) | `pub async fn run_foreground(role: Role, config: &Path, ctx: RunContext, on_ready: Box<dyn FnOnce(&ReadyReport) -> anyhow::Result<()> + Send>, shutdown: impl Future<Output = ()>) -> anyhow::Result<()>`. Order: claim → spawn_blocking load + key persist → publish configured id → bind → publish ephemeral id → engine run → remove own status file only if `status_quiesced` → drop claim. | public |
| `runtime::run_quick_serve` / `run_quick_access` | modified | `pub async fn run_quick_serve(cfg: ServeConfig, ctx: RunContext, on_ready: …, shutdown) -> anyhow::Result<()>` (Access analogous); never writes a config | public |
| `runtime::remove_own_status_file_for_exit` | unchanged (moved) | `pub fn remove_own_status_file_for_exit(role: Role)` | public |
| bridge re-exports | new (deviation R3, design Decision 1 "selected runtime bridge types") | `pub use iroh_tunnel_engine::{RunContext, ReadyReport, ReadyServices, ServeReady, AccessReady, Protocol};` at the host crate root so the root package needs no normal engine dependency | public |
| `testing` module | new | `#[cfg(feature = "test-utils")] pub mod testing { pub async fn run_unowned(role: Role, config: &Path, state_dir: Option<&Path>, shutdown: impl Future<Output = ()>) -> anyhow::Result<()>; }`. It replaces `serve::run_with_shutdown*` and `access::run_with_shutdown*`. | public (test-utils) |
| `update::UpdateRequest` / `Verification` / `OverwritePolicy` | new (replaces `cli::UpdateArgs` in host) | `pub struct UpdateRequest { pub to: Option<String>, pub verification: Verification, pub overwrite: OverwritePolicy }`; `pub enum Verification { Verified, Insecure }`; `pub enum OverwritePolicy { RespectOwner, ReplacePackageOwned }` | public |
| `update::UpdateContext` | modified | unchanged fields; `install_path` is now always supplied by the caller (no `current_exe` inside host) | public |
| `update::OwnerDecision` / `check_owner` | new | `pub enum OwnerDecision { Proceed { overriding_package: bool }, Delegate { guidance: String, overridable: bool } }`; `pub fn check_owner(ctx: &UpdateContext, o: OverwritePolicy) -> OwnerDecision` | public |
| `update::UpdateOutcome` / `update` | modified | `pub enum UpdateOutcome { AlreadyCurrent { version: String }, NewerThanLatest { current: String, latest: String }, Updated { path: PathBuf, from: String, to: String } }`; `pub async fn update(req: &UpdateRequest, ctx: &UpdateContext<'_>) -> anyhow::Result<UpdateOutcome>` (it does not check ownership; the caller calls `check_owner` first) | public |
| `update::{SignatureVerifier, SigstoreVerifier, Owner, detect, current_target, DEFAULT_BASE_URL, BASE_URL_ENV}` | unchanged (moved; `current_target`/consts now public) | as today | public |

### Root package (`iroh-tunnel`, binary-internal)

| Name | new / modified | Shape | public |
|---|---|---|---|
| `lifecycle::ForcedExit` / `forced_exit` | modified (moved from `shutdown`) | `pub enum ForcedExit { Signal, Watchdog }` + `exit_code()` (130 / 1); `pub async fn forced_exit(ctx: &RunContext) -> ForcedExit` (awaits `ctx.drain_deadline()`; same watchdog and repeated-signal rules) | crate |
| `lifecycle::wait_for_signal` | moved | as today | crate |
| `render` module | new | `render_ready(&ReadyReport) -> String`, `render_service_list(Role, &ServiceList) -> String`, `render_service_note(&ServiceNote) -> (Stream, String)` with `enum Stream { Stdout, Stderr }` (deviation R2: routing must stay byte-identical, as Task 13 specifies), `render_keygen(&KeygenOutcome)`, `render_update(&UpdateOutcome, exe_name)`; plus the moved `render_serve_status`/`render_access_status`. Each output is byte-identical to the Task 2 goldens. | crate |
| `cli_service::resolve_tunnel_executable` | moved from `service::resolve_binary` | `fn resolve_tunnel_executable() -> anyhow::Result<PathBuf>` (`which("iroh-tunnel")`, else `current_exe()`) | crate |
| `lib.rs` | modified | the root library target is removed; root tests depend on `iroh-tunnel-host`/`iroh-tunnel-engine` dev-dependencies with `features = ["test-utils"]`, or on binaries | — |
| Workspace manifest | modified | `[workspace] members = [".", "crates/host", "crates/engine"]`, `default-members` = same three, `resolver = "2"`; `[workspace.package] version, edition, rust-version, license`; `[workspace.dependencies]` without test features | — |
| CLI arguments, config keys, status JSON, registration formats | unchanged | — | — |

---

## Task 1: Baseline inventories — implements 1.1, 1.2, 1.3, 1.4

**Files:**
- Create: `openspec/changes/refactor-codebase/baseline/{tests.txt,tests-ignored.txt,external-crates.txt,features-normal.txt,features-all.txt,binaries.txt,public-api.txt,path-assumptions.md}`
- Create: `openspec/changes/refactor-codebase/baseline/capture.sh`

**Interfaces:** Produces the baseline files that Tasks 19, 23, 24 and 26 read. `public-api.txt` has one line per item in the form `iroh_tunnel::<module>::<item>`.

- [ ] **Step 1:** Write `capture.sh <outdir>`. It runs `cargo +1.91 test --all-features --locked -- --list --format terse` and the same with `--ignored`, then `sort -u` both. It extracts external crate names from `Cargo.lock` (`name = ` lines minus workspace packages, `sort -u`), runs `cargo +1.91 tree -e features --locked` and `cargo +1.91 tree -e features --all-features --locked`, and lists `[[bin]]` names.
- [ ] **Step 2:** Run `bash openspec/changes/refactor-codebase/baseline/capture.sh openspec/changes/refactor-codebase/baseline`. Expected: every file is non-empty, `binaries.txt` lists exactly `iroh-tunnel`, `iroh-tunnel-serve` and `iroh-tunnel-access`, and `features-normal.txt` contains no `test-utils`.
- [ ] **Step 3:** Fill `public-api.txt` from `src/lib.rs` `pub mod` items and their `pub` items, plus every `iroh_tunnel::*` path used under `tests/`.
- [ ] **Step 4:** Write `path-assumptions.md`. Grep `.github/workflows/*.yml`, `Dockerfile*`, `docker-compose.yml`, `packaging/**`, `install.sh`, `install.ps1`, `build.rs`, `docs/**`, `README.md`, `CONTRIBUTING.md` and `tests/**` (`include_str!`/`include_bytes!`) for `src/`, `tests/`, `Cargo.toml`, `[package]` and `version =`. Record one row per hit: file, line and which later task updates it. Known rows: `build.yml:57` (root version sed → Task 21) and `Dockerfile:16` (`COPY . .`, still valid → Task 22).
- [ ] **Step 5:** Commit: `git add openspec/changes/refactor-codebase/baseline && git commit -m "chore: record pre-refactor baseline inventories"`.

## Task 2: CLI exit and output characterization — implements 1.5, 1.6

**Files:**
- Create: `tests/cli_golden.rs`, `tests/fixtures/golden/*.txt`
- Modify: `tests/service_user.rs` (ordering assertion in `user_service_lifecycle_uses_the_pinned_absolute_paths`)

**Interfaces:** Produces golden files that Tasks 8, 10–17 and 21 must keep passing unchanged.

- [ ] **Step 1:** Confirm that existing tests already pin exits 2/3/5/6: `config_paths::invalid_config_reports_path_with_config_error_exit` (2), `service_user::system_install_without_privilege_stops_with_rerun_guidance` and `system_scope_operations_without_privilege_are_refused_with_guidance` (3), `service_user_windows::system_scope_is_unsupported_on_windows` (5), `role_ownership::second_serve_is_refused_across_binaries_overrides_and_start_order` (6). Run `cargo +1.91 test --all-features --locked --test config_paths --test service_user --test role_ownership`. Expected: PASS. Record the test names in `baseline/path-assumptions.md` under "pinned exits".
- [ ] **Step 2:** Write the failing golden tests in `tests/cli_golden.rs`. Each spawns `env!("CARGO_BIN_EXE_iroh-tunnel")` with an isolated `HOME`/`IROH_TUNNEL_STATE_DIR`, normalizes the temp dir to `<TMP>`, and compares stdout and stderr with a golden file:
  - `config_list_serve_table_is_unchanged`, `config_list_access_table_is_unchanged`, `config_list_without_services_array_prints_guidance_line`, `config_list_with_empty_services_prints_header_only`
  - `config_show_redacted_output_is_unchanged`, `config_path_prints_resolved_path`
  - `config_add_and_remove_messages_are_unchanged`, `keygen_messages_are_unchanged`, `keygen_random_prints_key_and_node_id_lines`
  - `status_json_forwards_file_bytes`: write a valid serve status file and assert stdout == file bytes + `"\n"`
  - `status_not_running_message_is_unchanged`
  - `serve_startup_lines_are_unchanged`: quick-mode serve against a local relay. Stdout lines before shutdown must be `NodeId: <ID>`, `Home relay: <URL>` (if present) and `Serving: <name> tcp://<addr>`, with dynamic values normalized.
- [ ] **Step 3:** Run `cargo +1.91 test --all-features --locked --test cli_golden -- --nocapture` once with `GOLDEN_BLESS=1` to write the goldens from the **current** code, then without it. Expected: PASS on both runs, and the goldens are committed unedited.
- [ ] **Step 4:** In `user_service_lifecycle_uses_the_pinned_absolute_paths` (linux-native, opt-in as today), assert that the `Installed and started …` line precedes the native `systemctl status` output in the combined stream, and that `status` of a stopped unit exits 0.
- [ ] **Step 5:** Commit: `git commit -m "test: pin CLI output and exit characterization"`.

## Task 3: Acceptance and status-publishing characterization — implements 1.7, 1.8

**Files:**
- Modify: `src/config.rs` (tests module), `src/serve.rs` (tests module), `src/access.rs` (tests module)

**Interfaces:** Produces test names that Tasks 6, 9 and 20 move and must keep green.

- [ ] **Step 1:** Write failing-or-passing characterization tests in `src/config.rs`:
  - `unknown_fields_and_legacy_multiplex_still_load`: TOML containing `foo = 1` under `[node]` and `multiplex = true` under `[[services]]` → `Ok`.
  - `hostname_target_still_loads`: `host = "example.internal"` → `Ok`, host unchanged.
  - `access_node_id_length_checked_at_load_full_parse_at_runtime`: a 64-char non-hex `node_id` loads `Ok`, while a wrong length fails with the existing message.
- [ ] **Step 2:** Write the status flush tests in `src/serve.rs` with `#[tokio::test(start_paused = true)]` and an injected `save` closure that records calls:
  - `initial_status_written_before_first_admission`: the first save completes before `accept_loop` starts.
  - `unchanged_status_is_not_rewritten`: two ticks with equal rendered files → one save.
  - `failed_status_write_is_retried_next_tick`: the first save returns `Err` → the second tick saves the same file.
  - `status_file_kept_when_flush_join_times_out`: the save blocks for more than 5s at shutdown → the removal is skipped.
- [ ] **Step 3:** Run `cargo +1.91 test --all-features --locked --lib config:: serve::`. Expected: PASS. These pin today's behaviour, so a failure here is a characterization bug: fix the test, not the code.
- [ ] **Step 4:** Commit: `git commit -m "test: characterize config acceptance and status publishing"`.

## Task 4: Per-run drain context — implements 2.1, 2.2

**Files:**
- Modify: `src/shutdown.rs` (add `RunContext`, remove `DRAIN_DEADLINE`), `src/serve.rs`, `src/access.rs`, `src/role_run.rs`, `src/app.rs`
- Test: `src/shutdown.rs` tests, `tests/role_ownership.rs`

**Interfaces:** Produces `RunContext` and `shutdown::begin(stop, ctx)`. The role loops take a `RunContext` parameter, and `app.rs` creates one per invocation and passes it to both `run` and `forced_exit`.

- [ ] **Step 1:** Write the failing test `two_runs_do_not_share_a_deadline`. Create `RunContext::new()` twice, call `begin` on the first context's stop channel, then assert that `tokio::time::timeout(1ms, ctx2.drain_deadline())` times out and `ctx1.drain_deadline()` returns immediately.
- [ ] **Step 2:** Replace `begin_publishes_the_same_deadline_to_forced_exit` with `begin_publishes_to_its_own_context`: `begin(&stop, &ctx)` → `ctx.drain_deadline().await == *stop.borrow()`.
- [ ] **Step 3:** Run `cargo +1.91 test --all-features --locked --lib shutdown::`. Expected: FAIL (`RunContext` not defined).
- [ ] **Step 4:** Implement `RunContext` per Contracts and change `begin` to publish into `ctx`. Change `forced_exit` to `forced_exit(ctx: &RunContext)`, and delete `DRAIN_DEADLINE`. In `app::execute`, create one `RunContext` and pass clones to `run(...)` and `forced_exit(&ctx)`. Thread `ctx` through `role_run::run_with_shutdown`/`run_config` → `run_loop*`.
- [ ] **Step 5:** Write `stop_before_ready_releases_claim_and_leaves_no_status_file` in `tests/role_ownership.rs` (Review Focus 5). Spawn `serve` with an unreachable relay so that startup blocks, send SIGTERM before `NodeId:` appears, then assert exit 0, no status file, and that a second `serve` start is not refused.
- [ ] **Step 6:** Run `cargo +1.91 test --all-features --locked --lib shutdown:: && cargo +1.91 test --all-features --locked --test configured_multiplex repeated_stop_after_grace_exits_130_and_cleans_status stuck_shutdown_is_abandoned_by_watchdog_with_exit_1 && cargo +1.91 test --all-features --locked --test role_ownership`. Expected: PASS.
- [ ] **Step 7:** Commit: `git commit -m "refactor: replace process-global drain deadline with per-run context"`.

## Task 5: Role-lifetime Quarantine sinks — implements 2.3

**Files:**
- Modify: `src/serve.rs` (`quarantine`, run loop), `src/shutdown.rs` (`QuarantineSinks`)
- Test: `src/serve.rs` tests, `tests/udp_tunnel.rs`

**Interfaces:** Produces `QuarantineSinks` (crate). `quarantine(upstream, &stats, &stop, &sinks)` spawns into `sinks`. The Serve run loop owns one `Arc<QuarantineSinks>` and calls `sinks.shutdown(deadline)` after `shutdown::join(&mut accept)`.

- [ ] **Step 1:** Write the failing tests in `src/serve.rs`:
  - `quarantine_sinks_are_joined_on_role_stop` (`start_paused`): spawn 3 sinks, call `begin`, `sinks.shutdown(deadline)` → `Handle::current().metrics().num_alive_tasks()` returns to its pre-spawn value, and `stats.quarantined_flows() == 0`.
  - `failed_quarantine_sink_releases_its_slot`: a sink whose socket recv errors → slot released, failure observed by `reap()` (no panic).
  - `quarantine_sink_outlives_its_connection`: drop the connection's `JoinSet`; the sink is still alive until `QUARANTINE` elapses.
- [ ] **Step 2:** Run `cargo +1.91 test --all-features --locked --lib serve::tests::quarantine`. Expected: FAIL.
- [ ] **Step 3:** Implement `QuarantineSinks` per Contracts. `spawn` calls `reap()` first so that finished handles do not accumulate. Keep `QuarantineSlot` drop semantics.
- [ ] **Step 4:** Run the step-2 command plus `cargo +1.91 test --all-features --locked --test udp_tunnel quarantine_holds_the_slot_and_then_releases_it at_the_service_cap_a_new_flow_is_refused_and_old_flows_keep_working`. Expected: PASS.
- [ ] **Step 5:** Commit: `git commit -m "refactor: own quarantine sinks for the role lifetime"`.

## Task 6: Status writer quiescence reporting — implements 2.4

**Files:**
- Modify: `src/serve.rs`, `src/access.rs` (flush join blocks), `src/role_run.rs`

**Interfaces:** Consumes Task 3 tests. Produces an internal `RunCompletion { result, status_quiesced }` returned by the role loops (final shape per Contracts). Removal of the status file happens in `role_run` only when `status_quiesced`.

- [ ] **Step 1:** Write the failing test `run_completion_reports_unquiesced_writer` (`src/serve.rs`): a save that blocks for 6s → `RunCompletion.status_quiesced == false`; a fast save → `true`; a flush task that panics → `false`.
- [ ] **Step 2:** Run it. Expected: FAIL (type missing).
- [ ] **Step 3:** Make `run_loop_with_state_dir` (both roles) return `RunCompletion`. Compute `status_quiesced` exactly as today's `flush_stopped`, close the endpoint, and move `remove_own_status_file` into `role_run::run_config`, guarded by `status_quiesced`.
- [ ] **Step 4:** Run `cargo +1.91 test --all-features --locked --lib serve:: access:: && cargo +1.91 test --all-features --locked --test serve_status_conn_path --test access_status_file`. Expected: PASS, including `flush_stop_mid_save_lets_it_finish_and_prevents_a_second_save` and the Task 3 tests.
- [ ] **Step 5:** Commit: `git commit -m "refactor: report status writer quiescence to the run owner"`.

## Task 7: Secret redaction — implements 2.5

**Files:**
- Modify: `src/config.rs` (manual `Debug` for `NodeConfig`, `ServeConfig`, `AccessConfig`), `src/config_cmd.rs`
- Test: `src/config.rs` tests, `tests/config_lifecycle.rs`

**Interfaces:** Produces the redacting `Debug` that later secret-bearing types copy (Contracts: `RelayToken`, `NodeSettings`, `ShownConfig`, `GeneratedIdentity`, `EditAttempt`).

- [ ] **Step 1:** Write the failing tests:
  - `config_debug_redacts_secret_key`: load a config with a known `secret_key` and `relay_token`, then assert `format!("{cfg:?}")` contains neither value and does contain `[redacted]`.
  - `error_chain_never_contains_secret_line`: a parse error on a file whose secret line is malformed → neither `format!("{e:#}")` nor `format!("{e:?}")` contains the secret text.
- [ ] **Step 2:** Run `cargo +1.91 test --all-features --locked --lib config::tests::config_debug_redacts_secret_key config::tests::error_chain_never_contains_secret_line`. Expected: FAIL on Debug.
- [ ] **Step 3:** Replace the derived `Debug` with manual impls that print `secret_key: "[redacted]"` when non-empty, `relay_token: Some("[redacted]")`, and every other field verbatim.
- [ ] **Step 4:** Run the step-2 command and `--test config_lifecycle parse_errors_never_echo_secret_lines show_redacts_secrets_unless_revealed`. Expected: PASS.
- [ ] **Step 5:** Commit: `git commit -m "fix: redact secret-bearing config Debug output"`.

## Task 8: Typed ready report — implements 3.1

**Files:**
- Create: `src/render.rs`
- Modify: `src/role_run.rs` (remove `println!` and `RoleStrategy::print_services`), `src/serve.rs`, `src/access.rs`, `src/app.rs`

**Interfaces:** Produces `ReadyReport`/`ReadyServices`/`ServeReady`/`AccessReady` and `render::render_ready(&ReadyReport) -> String`. The role loops take `on_ready: Box<dyn FnOnce(&ReadyReport) -> anyhow::Result<()> + Send>`.

- [ ] **Step 1:** Write the failing unit test `render_ready_matches_legacy_lines` in `src/render.rs`. Build a `ReadyReport` for serve (`ServeReady{name:"web", Tcp, "127.0.0.1:8080"}`) and access (`relay_override_count: 2`), then assert the exact output `"NodeId: X\nHome relay: R\nServing: web tcp://127.0.0.1:8080\n"` and `"Exposed: … [relay override: 2]"` as produced by today's `print_services`.
- [ ] **Step 2:** Write the failing test `ready_hook_failure_closes_endpoint_and_releases_claim` (Review Focus 3, `src/role_run.rs`). `on_ready` returns `Err` → the run returns that error, the endpoint is closed, the claim is dropped (a new `ownership::claim(Serve)` succeeds), and no status file exists.
- [ ] **Step 3:** Run both. Expected: FAIL.
- [ ] **Step 4:** Implement: build the `ReadyReport` where `print_services` ran. Keep the `tracing::warn!("no services configured; …")` in the engine path. Call `on_ready` before `tracing::info!("endpoint ready")`. On `Err`, close the endpoint and return it. In `app.rs`, pass `Box::new(|r| { print!("{}", render::render_ready(r)); Ok(()) })`.
- [ ] **Step 5:** Run the step-1/2 tests and `cargo +1.91 test --all-features --locked --test cli_golden serve_startup_lines_are_unchanged --test role_binaries`. Expected: PASS.
- [ ] **Step 6:** Commit: `git commit -m "refactor: report role readiness as a typed value"`.

## Task 9: Native observations and injected status sink — implements 3.2, 3.3

**Files:**
- Create: `src/observation.rs` (observation types per Contracts)
- Modify: `src/serve.rs`, `src/access.rs`, `src/udp.rs` (`snapshot()` returns `UdpObservation`), `src/conn_path.rs` (report returns `PeerPathObservation`), `src/status.rs` (`From<&…Observation>` impls), `src/role_run.rs`

**Interfaces:** Produces `ServeObservation`, `AccessObservation`, `StatusSink<O>`, `RoleHooks<O>`. The flush loops compare observations with `PartialEq` and call `sink.save(obs)`. Host-side (`role_run` for now) builds a `StatusSink` that maps through `StatusFile::from(&obs)` and `StatusWriter::save_with_state_dir` in `spawn_blocking`.

- [ ] **Step 1:** Write the failing test `observation_mapping_is_field_for_field` in `src/status.rs`. For a fully populated `ServeObservation` (UDP block, two peers, relay and direct transports), `serde_json::to_string(&StatusFile::from(&obs))` equals the JSON of today's `StatusFile` built from the same values (golden string). Do the same for Access.
- [ ] **Step 2:** Write the failing test `observation_equality_ignores_nothing_rendered`: two observations that differ only in `connections[0].transports[0].active` are `!=`, and identical ones are `==`. There is no timestamp field.
- [ ] **Step 3:** Run `cargo +1.91 test --all-features --locked --lib status:: observation::`. Expected: FAIL.
- [ ] **Step 4:** Implement the observation types and conversions. Change the flush loops to `StatusSink<O>`. Keep `biased` stop handling, `last` updated only on `Ok`, and the initial save before admission. Remove `crate::status` imports from `serve.rs`, `access.rs`, `udp.rs` and `conn_path.rs`, except in tests.
- [ ] **Step 5:** Run the Task 3 status tests, the step-1/2 tests, and `cargo +1.91 test --all-features --locked --test serve_status_conn_path --test access_status_file --test udp_tunnel status_files_report_flows_limit_and_drops_by_reason --test cli_golden`. Expected: PASS.
- [ ] **Step 6:** Commit: `git commit -m "refactor: publish native observations through an injected status sink"`.

## Task 10: Config operations return outcomes — implements 4.1

**Files:**
- Modify: `src/config_cmd.rs` (split into host-style operations + CLI wrappers), `src/app.rs` (`dispatch_config`), `src/render.rs`
- Test: `src/config_cmd.rs` tests

**Interfaces:** Produces `Role` (renamed `StatusRole`; keep `pub type StatusRole = Role` until Task 20), `AddService`, `Mutation`, `add_service`, `remove_service`, `ServiceList`, `ServiceRow`, `list_services`, `Disclosure`, `ShownConfig`, `show_config`, `GeneratedIdentity`, `random_identity` and `resolve_config_path(Role, …)`, all per Contracts. The CLI wrappers render with `render::render_service_list` etc.

- [ ] **Step 1:** Write the failing tests (tempdir configs, no terminal):
  - `add_service_returns_name_and_path`: `add_service(Role::Serve, &AddService{name:"web",protocol:"tcp",host:"127.0.0.1",port:8080,..})` → `Mutation{service:"web", path}`, and the file contains `name = "web"`.
  - `invalid_access_add_leaves_bytes_unchanged`: Access without `node_id` → `Err` downcasting to `CliError::Config`, and the bytes are unchanged.
  - `list_services_returns_rows_without_terminal`: the rows equal the entries; `rows == None` when no `services` array.
  - `show_redacts_secret_key_by_default`: `Disclosure::Redacted` → `text()` lacks the key; `Reveal` → contains it.
  - `random_identity_touches_no_file`: with `HOME` set to an empty tempdir → after the call the dir is still empty and `node_id` equals the public key of `secret()`.
  - `add_service_accepts_non_utf8_config_path` (`#[cfg(unix)]`, Review Focus 1): a config path with byte `0xff` in its file name → `Ok`.
- [ ] **Step 2:** Run `cargo +1.91 test --all-features --locked --lib config_cmd::`. Expected: FAIL.
- [ ] **Step 3:** Implement the operations by extracting the bodies of today's `add/remove/list/show/keygen_random/path`. Keep `mutate`, the lock, validation and atomic write unchanged, and move every `println!` to `render.rs` plus the CLI wrappers.
- [ ] **Step 4:** Run the step-2 command plus `--test cli_golden --test config_lifecycle --test config_multiplex_compat`. Expected: PASS with unchanged goldens.
- [ ] **Step 5:** Commit: `git commit -m "refactor: config operations return outcomes instead of printing"`.

## Task 11: Keygen rotation consent — implements 4.2

**Files:**
- Modify: `src/config_cmd.rs`, `src/app.rs`, `src/render.rs`

**Interfaces:** Consumes `Role` and `resolve_config_path` from Task 10. Produces `IdentityState`, `identity_state`, `RotationConsent`, `KeygenOutcome` and `keygen`. CLI flow (preserves today's ordering): `identity_state` → if `HasKey` and not `--yes`: non-TTY → the existing `rotation_refused` error, TTY → existing prompt; then `keygen(Given|NotGiven)`; `ConsentNeeded` → `rotation_refused` error.

- [ ] **Step 1:** Write the failing tests:
  - `keygen_without_consent_returns_consent_needed`: the config has a key → `ConsentNeeded{current_node_id: Some(<public of old>)}`, bytes unchanged.
  - `keygen_with_consent_rotates_and_returns_new_node_id`: `Generated{rotated:true}`, and the new `node_id` differs from the old one.
  - `first_keygen_needs_no_consent`: empty `secret_key` → `Generated{rotated:false}`.
- [ ] **Step 2:** Run `cargo +1.91 test --all-features --locked --lib config_cmd::tests::keygen`. Expected: FAIL.
- [ ] **Step 3:** Implement `keygen` with the decision inside `mutate` (under the lock). `identity_state` keeps the unlocked `.ok()` parse behaviour.
- [ ] **Step 4:** Run the step-2 command and `--test config_lifecycle keygen_rotation_needs_confirmation first_keygen_does_not_prompt --test cli_golden keygen_messages_are_unchanged`. Expected: PASS.
- [ ] **Step 5:** Commit: `git commit -m "refactor: keygen returns a rotation-consent outcome"`.

## Task 12: Edit prepare/commit — implements 4.3

**Files:**
- Modify: `src/config_cmd.rs`, `src/app.rs`

**Interfaces:** Produces `prepare_edit`, `EditAttempt`, `EditOutcome` and `EditRejection`. The CLI wrapper calls `prepare_edit`, runs `edit::edit_file(attempt.scratch_path())` outside any lock, then `commit()`: `Unchanged` → `No changes.`; `Saved` → `Saved <path>.`; `Rejected{message}` → `CliError::Config(message)`.

- [ ] **Step 1:** Write the failing tests:
  - `edit_commit_rejects_concurrent_change_and_keeps_sidecar`: prepare, rewrite the live file, write a valid edit to the scratch file, commit → `Rejected{reason:Conflict, sidecar:Some(p)}`. The live file keeps the concurrent bytes and `p` contains the edit.
  - `edit_commit_rejects_invalid_text_and_names_sidecar`: an invalid TOML edit → `Rejected{reason:Invalid}`, and `message` contains the sidecar path.
  - `dropped_edit_attempt_removes_scratch`: `drop(attempt)` → the scratch path no longer exists and the live bytes are unchanged.
  - `edit_of_missing_config_conflicts_if_created_meanwhile` (Review Focus 2): prepare on a missing config, create the file, commit → `Rejected{Conflict}`, and the created file is unchanged.
- [ ] **Step 2:** Run `cargo +1.91 test --all-features --locked --lib config_cmd::tests::edit`. Expected: FAIL.
- [ ] **Step 3:** Implement by moving today's `edit` body. `prepare_edit` writes the scratch file with `write_atomic`, and `commit` keeps the exact comparison against the original bytes or their absence, `keep_rejected`, and message texts.
- [ ] **Step 4:** Run the step-2 command and `--test config_lifecycle rejected_edit_keeps_prior_file_and_a_private_sidecar valid_edit_is_applied_atomically_and_owner_only edit_conflicting_with_a_concurrent_change_is_rejected --test config_multiplex_compat config_edit_saves_valid_editor_output_and_drops_multiplex`. Expected: PASS.
- [ ] **Step 5:** Commit: `git commit -m "refactor: split config edit into prepare and commit"`.

## Task 13: Native command adapter and service notes — implements 4.6

**Files:**
- Create: `src/service/native.rs`
- Modify: `src/service/{systemd,busybox,launchd,windows}.rs` (every `Command` call goes through `Native`; `println!`/`eprintln!` become `ServiceNote`s), `src/service/mod.rs`, `src/render.rs`

**Interfaces:** Produces `OutputMode`, `ServiceCall`, `ServiceNote`, `InstalledAt`, `NativeOutput` and `Native` (crate). `render::render_service_note(&ServiceNote) -> (Stream, String)` reproduces each removed `println!`/`eprintln!` byte-for-byte (`Stream::{Stdout, Stderr}`).

- [ ] **Step 1:** Write the failing tests with a fake runner that records `(program, args, mode)` and returns configured `NativeOutput`s:
  - `capture_mode_returns_native_output`: systemd `status` in `Capture` → returned `NativeOutput.stdout` equals the fake's bytes, and no note is printed.
  - `notes_reach_observer_before_native_command`: launchd install where bootstrap fails → the observer sees `LaunchdBootstrapHint` before the recorded `launchctl print` call. The test uses a shared `Vec<Event>`.
  - `stopped_systemd_unit_status_is_not_failure`: fake `systemctl status` exit 3 → `Ok`.
  - `render_service_note_matches_legacy_text`: one assertion per variant against the strings now in `systemd.rs:153,170`, `busybox.rs:192,207,223`, `launchd.rs:197,204,220,253`, `windows.rs:133,196,223`.
- [ ] **Step 2:** Run `cargo +1.91 test --all-features --locked --lib service::`. Expected: FAIL.
- [ ] **Step 3:** Implement `Native` per Contracts. Keep the backend command arguments, exit-status rules and error texts unchanged. The backends push notes through `call.notes` at the line where they printed before.
- [ ] **Step 4:** Run the step-2 command (on the host OS). Run `cargo +1.91 check --all-features --locked --target x86_64-pc-windows-msvc` and `--target aarch64-apple-darwin` where those targets are installed, otherwise record them for CI (Task 24).
- [ ] **Step 5:** Commit: `git commit -m "refactor: route service native commands and notes through an adapter"`.

## Task 14: Service operations with structured needs — implements 4.4, 4.5, 4.8

**Files:**
- Modify: `src/service/mod.rs` (public operations), `src/service/scope.rs` (`decide_install_scope`, `parse_scope_answer`), `src/service/system.rs` (`resolve_target` returns `InstallNeed::InvokingUser` instead of prompting; `retarget_decision`), `src/app.rs` (`dispatch_service` becomes prompt/render glue only)
- Create: `src/cli_service.rs` (`resolve_tunnel_executable`, prompt loops)

**Interfaces:** Consumes Task 13. Produces `Environment{elevated, can_answer}`, `ScopeDecision`, `decide_install_scope`, `InstallRequest`, `InstallOutcome`, `InstallNeed`, `RetargetConsent`, `Pin` (public), `AppliedEffect`, `ServiceReport`, `ServiceFailure`, `install` and `uninstall/start/stop/restart`, per Contracts. The CLI loop: call `install` → on `Needs(ScopeChoice)` show the existing scope prompt, answer via `parse_scope_answer` and retry with `scope: Some(..)`; on `Needs(InvokingUser)` prompt with the existing text and retry with `invoking_user`; on `Needs(Retarget)` prompt with the existing text, or for a non-TTY return the existing `CliError::Service` refusal, then retry with `retarget`.

- [ ] **Step 1:** Write the failing pure tests:
  - `install_scope_needs_choice_when_caller_can_answer`: `decide_install_scope(None, {elevated:false, can_answer:true}) == NeedsChoice`; `{false,false}` → `Chosen(User)`; `{true,_}` → `Chosen(System)`.
  - `install_without_scope_returns_scope_choice_need`: `install(InstallRequest{scope:None}, env{elevated:false, can_answer:true})` → `Ok(Needs(ScopeChoice))` and the fake records no calls.
  - `unprivileged_system_install_is_refused_with_guidance`: `install(scope: Some(System), env{elevated:false})` → `Err` downcasting to `CliError::Permission`, with message containing `sudo`, `serve service install --system` and the absolute config path. The fake runner records no calls.
  - `retarget_without_consent_needs_confirmation`: `retarget_decision(Some(body pinned uid 1001), proposed uid 1002, None)` → `Needs(Retarget{previous:Some(1001), proposed:1002})`.
  - `stale_retarget_consent_is_not_applied`: consent `previous uid 1001` but the registration now pins 1003 → `Needs(Retarget{previous:Some(1003)})`.
  - `retarget_from_unpinned_registration_matches_existing_policy` (Review Focus 4): a body without a pin → same result as today's `confirm_retarget` (assert against a table derived from `system.rs:187-230`).
  - `install_renders_supplied_tunnel_executable`: `InstallRequest{tunnel_executable:"/opt/x/iroh-tunnel"}` with a fake runner → the written unit/plist/script/task XML contains `/opt/x/iroh-tunnel`, and `current_exe()` is never consulted (`resolve_binary` deleted).
  - `relative_tunnel_executable_is_rejected`: `"iroh-tunnel"` → `CliError::Service`.
  - `partial_install_reports_written_registration`: the fake fails `systemctl enable` → `Err(ServiceFailure{applied:[RegistrationWritten(p)]})`, and no stop/disable command is recorded.
  - `start_is_refused_before_manager_when_role_runs`: hold `ownership::claim(Serve)` in an isolated state dir, call `start` → `CliError::Conflict`, and the fake recorded no calls.
  - `abandoned_install_at_decision_changes_nothing`: `Needs(InvokingUser)` → no registration file, and the config bytes are unchanged.
  - `busybox_user_scope_request_emits_system_wide_note` (`#[cfg(target_os = "linux")]`, pure helper): the user-scope request on the busybox path yields `ServiceNote::BusyboxSystemWide`.
  - `windows_system_scope_is_service_error` (`#[cfg(windows)]`): `install(System)` → `CliError::Service` with `--user`.
  - `cli_resolves_tunnel_binary_from_path_first` (`src/cli_service.rs`): with `PATH` pointing at a dir containing `iroh-tunnel` → that path.
- [ ] **Step 2:** Run `cargo +1.91 test --all-features --locked --lib service:: cli_service::`. Expected: FAIL.
- [ ] **Step 3:** Implement. Move the policy from `app::dispatch_service`/`install_system` into `service::install` and the verbs: rerun argv construction, `ensure_supported`, `ensure_privilege`, `check_installable_*`, `trust::verify_dir`/`read_verified`, and the existing-registration read and retarget decision, recomputed inside `install` immediately before the write. The CLI keeps the standalone-binary refusal, the scope prompt text and `absolutize_config_path` for the user scope.
- [ ] **Step 4:** Run the step-2 command and `cargo +1.91 test --all-features --locked --test service_user --test cli_golden`. Linux-native opt-in: `--test service_system -- --ignored` as root per that file's docs (blocked-ok if unavailable; record).
- [ ] **Step 5:** Commit: `git commit -m "refactor: service operations enforce policy and return structured needs"`.

## Task 15: Structured service status — implements 4.7

**Files:**
- Modify: `src/service/mod.rs`, backend `status` fns, `src/app.rs`, `src/render.rs`

**Interfaces:** Consumes Tasks 13–14. Produces `Fact<T>`, `Registration`, `ServiceStatusReport` and `status(...)`. In `Inherit` mode the CLI prints exactly what it prints today (native passthrough plus the Windows liveness lines rendered from `liveness`).

- [ ] **Step 1:** Write the failing tests:
  - `status_reports_foreground_role_without_registration`: no registration plus a claimed role → `registration: Known(Absent)`, `liveness: Known(Running{..})`.
  - `status_reports_missing_owner_as_unavailable`: claimed role with the owner record deleted → `owner: Unavailable(_)`, and the call is `Ok`.
  - `status_does_not_parse_native_prose`: the fake returns localized text → `liveness` comes from `ownership::probe` only.
- [ ] **Step 2:** Run `cargo +1.91 test --all-features --locked --lib service::tests::status`. Expected: FAIL.
- [ ] **Step 3:** Implement: registration from the existing unit/plist/script/task existence checks, liveness and owner from `ownership::probe`, native output from the backend's existing status command. Optional fact failures become `Unavailable(error.to_string())`.
- [ ] **Step 4:** Run the step-2 command and `--test cli_golden --test service_user`. Expected: PASS.
- [ ] **Step 5:** Commit: `git commit -m "refactor: service status returns separated facts"`.

## Task 16: Role-status read — implements 4.9

**Files:**
- Modify: `src/status_cmd.rs` (split into `read_role_status` + CLI renderer), `src/ownership.rs` (public `Liveness`/`Tier`/`OwnerRecord`), `src/render.rs`

**Interfaces:** Produces `RoleStatus`, `Snapshot`, `Freshness` and `read_role_status`. The CLI `status_cmd::run` becomes: read → match freshness for warnings and the live-without-file JSON → `print_line(raw)` for `--json`, or the table.

- [ ] **Step 1:** Write the failing tests:
  - `read_status_absent_and_not_running`: empty state dir → `Freshness::AbsentNotRunning`.
  - `read_status_stale_pid_includes_snapshot`: a file with a dead pid → `StalePid{live_pid:None}` and `Snapshot::Present`.
  - `read_status_creates_nothing`: no config and an empty state dir → the dir is still empty afterwards.
  - `read_status_invalid_file_is_unreadable`: garbage JSON → `Snapshot::Invalid`, `Freshness::Unreadable`.
- [ ] **Step 2:** Run `cargo +1.91 test --all-features --locked --lib status_cmd::`. Expected: FAIL.
- [ ] **Step 3:** Implement by moving the read and probe logic. The CLI maps `Snapshot::Invalid(e)` → `Err(e)` with today's context, and keeps the `warn_unless_live` texts, `describe_running` and broken-pipe tolerance.
- [ ] **Step 4:** Run the step-2 command, the golden tests (`serve_table_golden` etc.), `--test cli_golden status_json_forwards_file_bytes status_not_running_message_is_unchanged`, and `--test role_ownership status_does_not_trust_a_reused_pid running_role_status_names_role_and_owner_pid`. Expected: PASS.
- [ ] **Step 5:** Commit: `git commit -m "refactor: role status read returns classified data"`.

## Task 17: Updater with explicit inputs — implements 4.10

**Files:**
- Modify: `src/update.rs`, `src/update_owner.rs`, `src/app.rs`, `src/render.rs`
- Test: `tests/update.rs`, `tests/update_ownership.rs`, `tests/update_zip_windows.rs` (switch from `cli::UpdateArgs` to `UpdateRequest`)

**Interfaces:** Produces `UpdateRequest`, `Verification`, `OverwritePolicy`, `OwnerDecision`, `check_owner`, `UpdateOutcome` and `update`. The CLI wrapper, in today's order: build the context with `install_path = current_exe().canonicalize()` → `check_owner` → print the delegation guidance or the package-override warning → print the `--insecure` warning → `update` → `render_update`.

- [ ] **Step 1:** Write the failing tests in `src/update.rs`:
  - `package_owned_target_returns_delegation`: `check_owner(ctx{owner: Deb}, RespectOwner)` → `Delegate{overridable:true}`; with `ReplacePackageOwned` → `Proceed{overriding_package:true}`; container → `Delegate{overridable:false}` regardless.
  - `update_uses_supplied_install_path`: the in-process update against `release_server` with `install_path` in a tempdir → `Updated{path == tempdir path}`.
- [ ] **Step 2:** Run `cargo +1.91 test --all-features --locked --lib update::`. Expected: FAIL.
- [ ] **Step 3:** Implement. Keep `MAX_*` bounds, the verifier calls, URL formats, `http_client` user-agent `concat!("iroh-tunnel/", env!("CARGO_PKG_VERSION"))`, and `replace_executable`/`classify_replace_error`. Remove `current_exe` from host-bound code.
- [ ] **Step 4:** Run the step-2 command and `--test update --test update_ownership`. Run `--test update_zip_windows` on windows-native (CI `windows-update-e2e.yml`). Expected: PASS, including `update_does_not_stop_a_running_process_of_the_old_binary` and `package_owned_file_is_not_replaced_by_default`.
- [ ] **Step 5:** Commit: `git commit -m "refactor: updater takes explicit inputs and returns outcomes"`.

## Task 18: Blocking execution model — implements 4.11

**Files:**
- Modify: `src/app.rs` (config, service and status dispatch run inside `tokio::task::spawn_blocking`)
- Test: `src/service/mod.rs` tests

**Interfaces:** Consumes Tasks 10–16. Host operations stay synchronous with no runtime inside them.

- [ ] **Step 1:** Write the failing tests:
  - `service_operation_runs_inside_spawn_blocking`: on a multi-thread runtime, `spawn_blocking(|| status(..fake..)).await` → `Ok`, no panic.
  - `dropped_awaiter_does_not_stop_service`: the fake `start` sleeps 200ms; drop the `JoinHandle` after 10ms, wait 300ms → the recorded calls are exactly `[start]`.
- [ ] **Step 2:** Run them. Expected: the first may already pass; the second fails until `ServiceCall` can be built with an owned (`'static`) note sink. Add `ServiceCall::capture_owned()` if needed. It is crate-internal and does not change Contracts.
- [ ] **Step 3:** Wrap the CLI dispatch of config, service and status in `spawn_blocking`, propagating `JoinError` with `context("… task failed")`. Prompts run inside the blocking closure, so stdin is never read on a runtime worker.
- [ ] **Step 4:** Run `cargo +1.91 test --all-features --locked` (full). Expected: PASS.
- [ ] **Step 5:** Commit: `git commit -m "refactor: run synchronous host operations on blocking workers"`.

## Task 19: Workspace manifest and engine extraction — implements 5.1, 5.2, 5.3, 5.4

**Files:**
- Modify: `Cargo.toml` (workspace + package), `Cargo.lock`
- Create: `crates/engine/Cargo.toml`, `crates/engine/src/lib.rs`, one file per exported operation: `runtime/{mod,types,run_context}.rs` (`RunContext`, `RoleHooks`, `StatusSink`, `RunCompletion`, `ReadyReport` in `types.rs`), `serve/{mod,bind_serve,run_serve}.rs`, `access/{mod,bind_access,run_access}.rs`, `endpoint/{mod,node_id_string,home_relay}.rs`, `settings/{mod,types}.rs` (`UdpLimits::from_overrides` is a method and stays with its type), `observation/{mod,types}.rs`, private `tcp.rs` (pipe), `udp/{mod,…}.rs` (decision types in `types.rs`; crate-private helpers stay in cohesive private files), `proto.rs` (private), `shutdown.rs` (private), `testing.rs`
- Move: `src/{pipe,proto,endpoint,udp,conn_path(observation part)}.rs` and the role loops of `src/{serve,access}.rs` → `crates/engine/src/`
- Move tests: `tests/{serve_access_tunnel,udp_tunnel,configured_multiplex,relay_auth,access_path_logging}.rs`. They stay at root if they spawn binaries; in-process ones move to `crates/engine/tests/` when they use only engine API.
- Create: `tests/workspace_layout.rs`

**Interfaces:** Produces the engine public API per Contracts (`RunContext`, settings, `ReadyReport`, observations, `StatusSink`, `RoleHooks`, `RunCompletion`, `bind_*`, `run_*`, `testing`). The root temporarily depends on engine directly until Task 20 moves orchestration to host.

- [ ] **Step 1:** Write the failing tests in `tests/workspace_layout.rs`. Use `std::process::Command::new(env!("CARGO"))` with `metadata --format-version 1 --locked`, parsed with `serde_json`:
  - `workspace_has_exactly_three_members_all_default` (members, `workspace_default_members`, resolver `"2"` from `Cargo.toml` text)
  - `internal_packages_are_not_published` (`publish == []`)
  - `engine_depends_on_neither_host_nor_root` (all dependency kinds)
  - `engine_sources_have_no_exit_signal_or_print`: scan `crates/engine/src/**/*.rs` outside `#[cfg(test)]` modules and `testing.rs` for `process::exit`, `tokio::signal`, `println!`, `eprintln!`, `print!`; expect none
  - `udp_decision_model_lives_in_engine`: `FlowId`, `Admission`, `ClientFlows`, `PeerFlows`, `QUARANTINE`, `payload_limit` are defined under `crates/engine/src/`
  - `engine_files_export_at_most_one_operation`: for each `crates/engine/src/**/*.rs` other than `lib.rs`, `mod.rs`, `types.rs` and `testing.rs`, count the top-level `pub fn`/`pub async fn` items (excluding `impl` blocks and `#[cfg(test)]` modules). Expect ≤ 1, and expect each to have a `#[cfg(test)] mod tests` unless the file is private infrastructure listed in the test's allowlist (`tcp.rs`, `proto.rs`, `shutdown.rs`).
  - `no_external_crate_added_since_baseline`: the external crate set from `Cargo.lock` ⊆ `baseline/external-crates.txt`
- [ ] **Step 2:** Run `cargo +1.91 test --all-features --locked --test workspace_layout`. Expected: FAIL.
- [ ] **Step 3:** Convert `Cargo.toml`: add `[workspace]` (members, default-members, resolver 2), `[workspace.package]` (version, edition, rust-version, license), and `[workspace.dependencies]` for shared crates with no test features. The root `[package]` uses `version.workspace = true` etc.
- [ ] **Step 4:** Create `crates/engine` (`publish = false`, `[features] test-utils = ["iroh/test-utils"]`) and move the modules. Engine `settings.rs` defines `NodeSettings`/`ServeSettings`/`AccessSettings`/`UdpLimits::from_overrides`. Root `config.rs` gets `serve_settings`/`access_settings` mappings. Root `test-utils = ["iroh-tunnel-engine/test-utils"]`.
- [ ] **Step 5:** Run `cargo +1.91 build --workspace --locked && cargo +1.91 test --workspace --all-features --locked`. Expected: PASS including `workspace_layout`.
- [ ] **Step 6:** Commit: `git commit -m "refactor: extract iroh-tunnel-engine package"`.

## Task 20: Host extraction, error move, visibility and features — implements 6.1, 6.2, 6.3, 6.4, 6.5

**Files:**
- Create: `crates/host/Cargo.toml`, `crates/host/src/{lib.rs,error.rs,role.rs,testing.rs}` and one file per exported operation:
  - `config/{mod,types,dto,store,to_settings,resolve_config_path,require_existing,add_service,remove_service,list_services,show_config,identity_state,keygen,random_identity,edit}.rs` (`store.rs` is private infrastructure; `serve_settings` and `access_settings` share `to_settings.rs` as one mapping operation per role pair, listed in the allowlist; `edit.rs` holds `prepare_edit` plus the `EditAttempt::commit` method)
  - `paths/{mod,default_config_path,default_state_dir}.rs`, private `ownership.rs`
  - `status/{mod,types,schema,writer,read_role_status}.rs` (`schema.rs` = serde DTOs + `From` impls; `writer.rs` private)
  - `service/{mod,types,decide_install_scope,parse_scope_answer,install,uninstall,start,stop,restart,status}.rs`, with private `native.rs`, `trust.rs`, `account.rs`, `system.rs`, `pin.rs`, `task_xml.rs`, `backend/{systemd,busybox,launchd,windows}.rs`
  - `runtime/{mod,run_foreground,run_quick_serve,run_quick_access,remove_own_status_file_for_exit}.rs`
  - `update/{mod,types,check_owner,update,detect,current_target}.rs`, with private `replace.rs` and `verify.rs`
- Move: `src/{config,config_store,error,paths,ownership,status,role_run}.rs`, the non-CLI parts of `src/{config_cmd,status_cmd,update,update_owner}.rs`, and `src/service/**` → `crates/host/src/`
- Move tests: in-process host tests to `crates/host/tests/`; real-binary tests stay at root
- Modify: `Cargo.toml` (root depends on host only; remove `iroh-tunnel = { path = ".", features = ["test-utils"] }`), `tests/workspace_layout.rs`

**Interfaces:** Produces the host public API per Contracts, including `runtime::run_foreground`, `run_quick_*` and `testing::run_unowned`. Root `src/` keeps only `main.rs`, `bin_*.rs`, `app.rs`, `cli.rs`, `render.rs`, `cli_service.rs` and `lifecycle.rs`.

- [ ] **Step 1:** Add the failing tests to `tests/workspace_layout.rs`:
  - `host_depends_neither_on_root_nor_clap`
  - `host_sources_have_no_exit_stdin_print_or_signal`: scan `crates/host/src` outside tests and `testing.rs` for `process::exit`, `stdin()`, `println!`, `eprintln!`, `print!`, `tokio::signal`
  - `host_sources_open_no_listener`: no `TcpListener`, `UdpSocket::bind`, `UnixListener`
  - `ownership_bypass_is_test_utils_gated`: every `run_unowned` / `Ownership::Unenforced` occurrence is inside a `#[cfg(feature = "test-utils")]` item
  - `release_feature_graph_excludes_test_utils`: `cargo tree -e features -p iroh-tunnel --locked` output contains no `test-utils`
  - `root_has_no_self_dev_dependency`
  - `host_files_export_at_most_one_operation`: the same rule as `engine_files_export_at_most_one_operation` over `crates/host/src`. Allowlist: `store.rs`, `writer.rs`, `ownership.rs`, `native.rs`, `trust.rs`, `account.rs`, `system.rs`, `pin.rs`, `task_xml.rs`, `backend/*.rs`, `replace.rs`, `verify.rs`, `to_settings.rs` (2 fns).
- [ ] **Step 2:** Run `cargo +1.91 test --workspace --all-features --locked --test workspace_layout`. Expected: FAIL.
- [ ] **Step 3:** Create `crates/host` (`publish = false`, `[features] test-utils = ["iroh-tunnel-engine/test-utils"]`; deps: engine plus every current root dependency used by the moved modules, declared via `workspace = true`; `clap`, `edit`, `which` and `tracing-subscriber` stay root-only). Move the modules. Place each exported operation in its own file as listed above, moving its unit tests from Tasks 3–18 into that file's `#[cfg(test)] mod tests`. Make backend modules, `Native`, trust handles, lock internals and `config_store` private. Add RS-02 rustdoc to every `pub` item.
- [ ] **Step 4:** Move `CliError` to `iroh_tunnel_host::error`. The root `app.rs` downcasts `iroh_tunnel_host::error::CliError` with the same match. Root `test-utils = ["iroh-tunnel-host/test-utils"]`; root dev-dependencies `iroh-tunnel-host = { path = "crates/host", features = ["test-utils"] }` and `iroh-tunnel-engine = { path = "crates/engine", features = ["test-utils"] }`.
- [ ] **Step 5:** Update root and in-process tests to `iroh_tunnel_host::…`/`iroh_tunnel_engine::…` paths and to `testing::run_unowned`.
- [ ] **Step 6:** Run `cargo +1.91 test --workspace --all-features --locked` and `cargo +1.91 build --workspace --locked` (default features). Expected: PASS, including the exit-code tests from Task 2 Step 1 and `workspace_layout`.
- [ ] **Step 7:** Commit: `git commit -m "refactor: extract iroh-tunnel-host package and move CliError"`.

## Task 21: Root CLI composition, binaries and version — implements 7.1, 7.2

**Files:**
- Modify: `src/app.rs`, `src/lifecycle.rs`, `.github/workflows/build.yml` (version step), `tests/workspace_layout.rs`

**Interfaces:** Consumes host/engine APIs. Produces no new API.

- [ ] **Step 1:** Add failing tests to `tests/workspace_layout.rs`:
  - `root_package_defines_three_binaries`: the metadata targets of `iroh-tunnel` include bins `iroh-tunnel`, `iroh-tunnel-serve`, `iroh-tunnel-access`
  - `all_packages_share_one_version`
  - `ci_build_workflow_reads_workspace_version`: `build.yml` has no `sed -n 's/^version = …' Cargo.toml` and uses `cargo metadata --no-deps --format-version 1` (`jq -r '.packages[] | select(.name=="iroh-tunnel") | .version'`)
- [ ] **Step 2:** Run it. Expected: FAIL on the workflow assertion.
- [ ] **Step 3:** Update the `build.yml` version step. Audit `src/app.rs`: it must contain only argument mapping, prompts, rendering, `RunContext` creation, `forced_exit`, `spawn_blocking` dispatch and exit mapping. Any remaining policy (path absolutization for the system scope, installable checks) moves to host.
- [ ] **Step 4:** Run `cargo +1.91 build --release --locked` and check that `target/release/iroh-tunnel{,-serve,-access} --version` print `<version>` in today's format, plus `cargo +1.91 test --workspace --all-features --locked --test workspace_layout --test role_binaries --test cli_golden`. Expected: PASS.
- [ ] **Step 5:** Commit: `git commit -m "build: read workspace version and keep binary identities"`.

## Task 22: Distribution paths and fixtures — implements 7.3, 7.4

**Files:**
- Modify: `.github/workflows/{ci,checks,cache-warm,snapshot,docker,package,release,release-smoke}.yml` (path filters, `--workspace` gates), `Dockerfile`, `Dockerfile.release*`, `docker-compose.yml`, `packaging/**`, any `include_*` fixture paths, `docs/**`, `README.md`, `CONTRIBUTING.md`
- Modify: `tests/workspace_layout.rs`

**Interfaces:** Consumes `baseline/path-assumptions.md`.

- [ ] **Step 1:** Add the failing tests:
  - `ci_gates_use_workspace_flags`: `checks.yml`/`ci.yml` run `cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features`, `cargo test --workspace --all-features`
  - `ci_path_filters_include_crates`: every workflow `paths:` filter that lists `src/**` also lists `crates/**`
  - `dockerfiles_copy_member_manifests`: each builder stage that copies `Cargo.toml` for dependency caching also copies `crates/engine/Cargo.toml` and `crates/host/Cargo.toml` (or uses `COPY . .`)
  - `installer_scripts_remain_at_root`: `install.sh` and `install.ps1` exist at the repository root, and the documented URLs in `README.md` are unchanged from baseline
- [ ] **Step 2:** Run `--test workspace_layout`. Expected: FAIL where workflows lack `--workspace` or `crates/**`.
- [ ] **Step 3:** Update every row of `path-assumptions.md` and tick each row with the commit that fixed it.
- [ ] **Step 4:** Run `--test workspace_layout --test installer --test release_artifacts` and `docker build -f Dockerfile .` (blocked-ok if Docker is unavailable; record it). Expected: PASS.
- [ ] **Step 5:** Commit: `git commit -m "build: update CI, Docker and packaging for the workspace"`.

## Task 23: Migration map — implements 7.5

**Files:**
- Create: `openspec/changes/refactor-codebase/migration-map.md`
- Modify: `CONTRIBUTING.md` (summary section "Workspace layout and Rust API migration")
- Modify: `tests/workspace_layout.rs`

**Interfaces:** Consumes `baseline/public-api.txt`.

- [ ] **Step 1:** Add the failing test `migration_map_covers_every_baseline_public_path`: every line of `baseline/public-api.txt` appears in a table row of `migration-map.md` whose "new location" cell is non-empty (either a path or `private — use <operation>`).
- [ ] **Step 2:** Run it. Expected: FAIL.
- [ ] **Step 3:** Write the map: old path → new path, or `private — use <operation>`, plus a repository-path table mirroring design.md's Migration Plan table.
- [ ] **Step 4:** Run it. Expected: PASS.
- [ ] **Step 5:** Commit: `git commit -m "docs: add Rust API and repository path migration map"`.

## Task 24: Workspace gates and baseline comparison — implements 8.1, 8.2, 8.3

**Files:**
- Create: `openspec/changes/refactor-codebase/baseline/after/*` (re-captured), `openspec/changes/refactor-codebase/baseline/test-mapping.md`

**Interfaces:** Consumes Task 1 `capture.sh`.

- [ ] **Step 1:** Run the gates:
  ```sh
  cargo +1.91 fmt --all -- --check
  cargo +1.91 clippy --workspace --all-targets --all-features --locked -- -D warnings
  cargo +1.91 test --workspace --all-features --locked
  cargo +1.91 build --workspace --locked
  cargo +1.91 build --release --locked
  ```
  Expected: all exit 0. `clippy -v` output names all three packages.
- [ ] **Step 2:** Run `capture.sh baseline/after` (adapted to `--workspace`). Diff `tests.txt`/`tests-ignored.txt` against the baseline and write `test-mapping.md`: every removed name maps to a new name or a documented split/merge/removal reason. Diff `external-crates.txt` (expect no additions) and `features-normal.txt` (expect no `test-utils`).
- [ ] **Step 3:** Run the eleven-capability regression explicitly: `cargo +1.91 test --workspace --all-features --locked` plus the opt-in ignored local-relay cases named in `baseline/tests-ignored.txt` that are safe locally. Do **not** run public-relay cases (`serve_access_tunnel` ignored set, `release_smoke`); record them as blocked.
- [ ] **Step 4:** Commit: `git commit -m "test: compare post-refactor inventory with baseline"`.

## Task 25: Native lifecycle and updater E2E — implements 8.4, 8.5

**Files:**
- Create: `crates/host/examples/manage_service.rs` (test-only non-terminal caller: install, start, exit)
- Modify: `tests/service_user.rs`, `tests/service_user_windows.rs` (call the example as a second caller)

**Interfaces:** Consumes the host service API. The example takes `--role serve --config <abs> --exe <abs iroh-tunnel> --action install|status|uninstall` and prints the `Debug` of the outcome to stdout. The example is the only place a non-CLI caller prints.

- [ ] **Step 1:** Write `caller_exit_leaves_user_service_running` (linux-native systemd user, opt-in like the existing lifecycle test). Run the example `install`, let it exit, wait for the status file, assert the role is live via `iroh-tunnel serve status`, then uninstall via the example and assert the registration is removed.
- [ ] **Step 2:** Write `captured_systemctl_status_is_returned`: example `status` → stdout contains `NativeOutput { program: "systemctl"`, and the native systemctl text is not on the terminal (the example's stderr is empty).
- [ ] **Step 3:** Mirror Step 1 in `service_user_windows.rs` (Task Scheduler) and run it in `windows-service-e2e.yml`. Mirror it for launchd on macos-native if available, otherwise record it as blocked.
- [ ] **Step 4:** Run the updater workflows: `--test update --test update_ownership` locally and `update_zip_windows` via `windows-update-e2e.yml`. Record the signed-release (`release_artifacts` signed cases) and public-relay smoke checks as blocked unless a safe harness is available.
- [ ] **Step 5:** Commit: `git commit -m "test: native service lifecycle through a non-terminal caller"`.

## Task 26: Final public API and migration audit — implements 8.6

**Files:**
- Modify: `openspec/changes/refactor-codebase/migration-map.md` (only if gaps found)

- [ ] **Step 1:** Run `cargo +1.91 doc --workspace --no-deps --locked` (default features) and list every public item of `iroh_tunnel_host` and `iroh_tunnel_engine` from `target/doc/*/all.html`. Each item must appear in Contracts, and none may be a raw backend, `Native`, trust handle, lock or `run_unowned`.
- [ ] **Step 2:** Run `--test workspace_layout` (all tests) and `migration_map_covers_every_baseline_public_path`. Expected: PASS.
- [ ] **Step 3:** Commit any fixes: `git commit -m "docs: finalize public API audit"`.

---

## Scenario → test

### workspace-boundaries

- Workspace members and default members → `workspace_layout::workspace_has_exactly_three_members_all_default` (Task 19)
- Internal packages are not publishable → `workspace_layout::internal_packages_are_not_published` (Task 19)
- Engine depends on neither host nor root → `workspace_layout::engine_depends_on_neither_host_nor_root` (Task 19)
- Host does not depend on the root package → `workspace_layout::host_depends_neither_on_root_nor_clap` (Task 20)
- Lockfile crate set comparison → `workspace_layout::no_external_crate_added_since_baseline` (Task 19) + Task 24 Step 2 diff
- UDP decision model stays together → `workspace_layout::udp_decision_model_lives_in_engine` (Task 19)
- Startup readiness is reported as a value → `render::tests::render_ready_matches_legacy_lines` + `cli_golden::serve_startup_lines_are_unchanged` (Task 8)
- Status observations carry no file schema → `status::tests::observation_mapping_is_field_for_field` (Task 9) + `engine_sources_have_no_exit_signal_or_print` (no serde_json writer in engine, Task 19)
- No exit, signal or print calls → `workspace_layout::engine_sources_have_no_exit_signal_or_print` (Task 19)
- CLI mutations go through host operations → `config_cmd::tests::add_service_returns_name_and_path` (Task 10) + `service::tests::unprivileged_system_install_is_refused_with_guidance` (Task 14) + Task 21 Step 3 audit
- Host has no clap dependency → `workspace_layout::host_depends_neither_on_root_nor_clap` (Task 20)
- No exit or stdin reads → `workspace_layout::host_sources_have_no_exit_stdin_print_or_signal` (Task 20)
- Two runs in one process do not share a deadline → `shutdown::tests::two_runs_do_not_share_a_deadline` (Task 4)
- Forced exit codes are unchanged → `configured_multiplex::repeated_stop_after_grace_exits_130_and_cleans_status`, `configured_multiplex::stuck_shutdown_is_abandoned_by_watchdog_with_exit_1`, `shutdown::tests::exit_codes_are_130_for_signal_and_1_for_watchdog` (Task 4)
- Management call installs no signal handler → `workspace_layout::host_sources_have_no_exit_stdin_print_or_signal` (Task 20; `RunContext` is only created in root `app.rs`)
- No task survives the role → `serve::tests::quarantine_sinks_are_joined_on_role_stop` (alive-task count) + `serve::tests::run_completion_reports_unquiesced_writer` (Tasks 5–6)
- Quarantine sink outlives its connection → `serve::tests::quarantine_sink_outlives_its_connection` (Task 5)
- Quarantine sinks stop with the role → `serve::tests::quarantine_sinks_are_joined_on_role_stop` (Task 5)
- Slot released on sink failure → `serve::tests::failed_quarantine_sink_releases_its_slot` + `udp_tunnel::quarantine_holds_the_slot_and_then_releases_it` (Task 5)
- Writer joins in time → `serve::tests::run_completion_reports_unquiesced_writer` (fast case) + `serve_status_conn_path::serve_status_file_reports_connected_peer_paths` (file removed at stop) (Task 6)
- Writer does not quiesce → `serve::tests::status_file_kept_when_flush_join_times_out` (Task 3) + `run_completion_reports_unquiesced_writer` (Task 6)
- Initial snapshot precedes admission → `serve::tests::initial_status_written_before_first_admission` (Task 3)
- Unchanged observation is not rewritten → `serve::tests::unchanged_status_is_not_rewritten` (Task 3) + `observation_equality_ignores_nothing_rendered` (Task 9)
- Failed write is retried → `serve::tests::failed_status_write_is_retried_next_tick` (Task 3)
- Debug of a loaded configuration → `config::tests::config_debug_redacts_secret_key` (Task 7)
- Error context does not echo secrets → `config::tests::error_chain_never_contains_secret_line` + `config_lifecycle::parse_errors_never_echo_secret_lines` (Task 7)
- Permission failure keeps exit code 3 → `service_user::system_install_without_privilege_stops_with_rerun_guidance` (Tasks 2, 20)
- Ownership conflict keeps exit code 6 → `role_ownership::second_serve_is_refused_across_binaries_overrides_and_start_order` (Tasks 2, 20)
- Invalid configuration keeps exit code 2 → `config_paths::invalid_config_reports_path_with_config_error_exit` (Tasks 2, 20)
- Existing capability tests still pass → full `cargo +1.91 test --workspace --all-features --locked` + `baseline/test-mapping.md` (Task 24)
- Previously accepted configuration still loads → `config::tests::unknown_fields_and_legacy_multiplex_still_load`, `hostname_target_still_loads`, `access_node_id_length_checked_at_load_full_parse_at_runtime` (Task 3) + `config_multiplex_compat::legacy_multiplex_values_are_ignored_and_removed_on_save`
- Release graph excludes test-utils → `workspace_layout::release_feature_graph_excludes_test_utils` (Task 20)
- Test graph enables the override explicitly → `relay_auth::tokened_relay_roundtrips_bytes` under `--all-features` (Task 20 Step 6)
- Gates fail on a member-only defect → `workspace_layout::ci_gates_use_workspace_flags` (Task 22) + Task 24 Step 1 (`clippy -v` lists all three packages)
- Test inventory comparison → Task 24 Step 2 `baseline/test-mapping.md`
- Binary targets after the move → `workspace_layout::root_package_defines_three_binaries`, `all_packages_share_one_version` + Task 21 Step 4 `--version` check + `role_binaries` suite
- Build workflow reads the workspace version → `workspace_layout::ci_build_workflow_reads_workspace_version` (Task 21)
- Docker build includes member packages → `workspace_layout::dockerfiles_copy_member_manifests` + Task 22 Step 4 `docker build` (blocked-ok)
- Installer URLs unchanged → `workspace_layout::installer_scripts_remain_at_root` + `installer::default_flow_installs_all_three_binaries_and_working_aliases` (Task 22)
- Former test bypass is not a production API → `workspace_layout::ownership_bypass_is_test_utils_gated` (Task 20) + Task 26 Step 1
- Migration map covers moved modules → `workspace_layout::migration_map_covers_every_baseline_public_path` (Task 23)

### frontend-independent-management

- Caller without a terminal lists services → `config_cmd::tests::list_services_returns_rows_without_terminal` (Task 10) + `workspace_layout::host_sources_have_no_exit_stdin_print_or_signal` (Task 20)
- No clap types in requests → `workspace_layout::host_depends_neither_on_root_nor_clap` (Task 20)
- No listener is opened by management calls → `workspace_layout::host_sources_open_no_listener` (Task 20)
- Add returns the affected entry → `config_cmd::tests::add_service_returns_name_and_path` (Task 10)
- Random key touches no file → `config_cmd::tests::random_identity_touches_no_file` (Task 10)
- Invalid mutation changes nothing → `config_cmd::tests::invalid_access_add_leaves_bytes_unchanged` + `config_lifecycle::invalid_mutations_fail_and_leave_file_bytes_unchanged` (Task 10)
- Concurrent mutations serialize → `config_lifecycle::concurrent_adds_serialize_without_losing_updates` (Task 10)
- Default show → `config_cmd::tests::show_redacts_secret_key_by_default` + `cli_golden::config_show_redacted_output_is_unchanged` (Task 10)
- Rotation without consent → `config_cmd::tests::keygen_without_consent_returns_consent_needed` (Task 11)
- Rotation with consent → `config_cmd::tests::keygen_with_consent_rotates_and_returns_new_node_id` (Task 11)
- First generation needs no consent → `config_cmd::tests::first_keygen_needs_no_consent` + `config_lifecycle::first_keygen_does_not_prompt` (Task 11)
- CLI confirmation behaviour unchanged → `config_lifecycle::keygen_rotation_needs_confirmation` + `cli_golden::keygen_messages_are_unchanged` (Task 11)
- Concurrent change rejects the edit → `config_cmd::tests::edit_commit_rejects_concurrent_change_and_keeps_sidecar` + `config_lifecycle::edit_conflicting_with_a_concurrent_change_is_rejected` (Task 12)
- Invalid edit is rejected → `config_cmd::tests::edit_commit_rejects_invalid_text_and_names_sidecar` + `config_lifecycle::rejected_edit_keeps_prior_file_and_a_private_sidecar` (Task 12)
- Dropped attempt → `config_cmd::tests::dropped_edit_attempt_removes_scratch` (Task 12)
- Unprivileged system scope → `service::tests::unprivileged_system_install_is_refused_with_guidance` + `service_user::system_install_without_privilege_stops_with_rerun_guidance` (Task 14)
- BusyBox user scope unchanged → `service::busybox::tests::busybox_user_scope_request_emits_system_wide_note` (Task 14; linux-native BusyBox host blocked-ok, Task 25)
- Windows remains user-only → `service::tests::windows_system_scope_is_service_error` + `service_user_windows::system_scope_is_unsupported_on_windows` (Task 14, windows-native)
- Install without scope from an unprivileged caller → `service::tests::install_without_scope_returns_scope_choice_need` + `service::scope::tests::install_scope_needs_choice_when_caller_can_answer` (Task 14). Note: a management caller that can decide sets `can_answer: true` to receive the need; `can_answer: false` keeps today's non-interactive user-scope default (system-service-management "Non-interactive non-root default").
- CLI keeps its scope prompt → `service_user::interactive_install_offers_user_or_system_and_system_is_refused_unprivileged` (Task 14)
- Retarget without confirmation → `service::system::tests::retarget_without_consent_needs_confirmation` (Task 14)
- Stale confirmation → `service::system::tests::stale_retarget_consent_is_not_applied` (Task 14)
- Management application registers the tunnel binary → `service::tests::install_renders_supplied_tunnel_executable` (Task 14) + `caller_exit_leaves_user_service_running` (Task 25)
- CLI resolution unchanged → `cli_service::tests::cli_resolves_tunnel_binary_from_path_first` + `service_user::user_service_lifecycle_uses_the_pinned_absolute_paths` (Task 14)
- Caller exits after start → `service_user::caller_exit_leaves_user_service_running` (Task 25, linux-native) + Windows mirror (windows-native)
- Start refused when the role already runs → `service::tests::start_is_refused_before_manager_when_role_runs` + `service_user_windows::user_background_task_lifecycle` (exit 6) (Task 14)
- Partial install reports applied effects → `service::tests::partial_install_reports_written_registration` (Task 14)
- Foreground role without a registration → `service::tests::status_reports_foreground_role_without_registration` (Task 15)
- Owner metadata unavailable → `service::tests::status_reports_missing_owner_as_unavailable` (Task 15)
- Captured diagnostics → `service::native::tests::capture_mode_returns_native_output` (Task 13) + `service_user::captured_systemctl_status_is_returned` (Task 25, linux-native)
- Ordering on a terminal → `service::native::tests::notes_reach_observer_before_native_command` (Task 13) + ordering assertion in `service_user::user_service_lifecycle_uses_the_pinned_absolute_paths` (Task 2, linux-native)
- Role not running → `status_cmd::tests::read_status_absent_and_not_running` (Task 16)
- Stale snapshot → `status_cmd::tests::read_status_stale_pid_includes_snapshot` + `role_ownership::status_does_not_trust_a_reused_pid` (Task 16)
- Read in an empty state directory → `status_cmd::tests::read_status_creates_nothing` (Task 16)
- Raw JSON passthrough → `cli_golden::status_json_forwards_file_bytes` (Tasks 2, 16)
- Called from a blocking worker → `service::tests::service_operation_runs_inside_spawn_blocking` (Task 18)
- Cancelled before side effects → `service::tests::abandoned_install_at_decision_changes_nothing` (Task 14)
- Dropped awaiter during a manager command → `service::tests::dropped_awaiter_does_not_stop_service` (Task 18)
- Package-manager install is delegated → `update::tests::package_owned_target_returns_delegation` + `update_ownership::package_owned_file_is_not_replaced_by_default` (Task 17)
- Running service untouched → `update::update_does_not_stop_a_running_process_of_the_old_binary` (Task 17)
