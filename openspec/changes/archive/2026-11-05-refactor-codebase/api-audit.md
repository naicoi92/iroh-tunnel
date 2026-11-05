# Public API audit (Task 26)

Source: `cargo +1.91 doc --workspace --no-deps --locked` (default features), `target/doc/iroh_tunnel_{host,engine}/all.html`. Test-utils-only items (`engine::testing::set_relay_only`, `host::testing::run_unowned`) are absent from the default-feature listing, as required.

## Result

- VIOLATIONs: **0**. No raw backend module, `Native`, trust/ownership handle, lock, `run_unowned` (default features) or OS command construction is public.
- Gates: `cargo +1.91 test --workspace --all-features --locked --test workspace_layout --test layout_migration_map` → 14 + 1 passed.
- Migration map: no gap found (`layout_migration_map` passes); `migration-map.md` unchanged.
- Rustdoc: host builds under `#![deny(missing_docs)]` (0 missing). Engine has no `missing_docs` lint; probing with `-W missing_docs` shows 75 undocumented public **fields/variants** (no undocumented types or functions): `observation/types.rs` (31), `runtime/types.rs` (16), `settings.rs` (28). Recorded as a finding for a follow-up code fix (add field docs, then add `#![deny(missing_docs)]` to `crates/engine/src/lib.rs`).

## Counts

| Crate | contract | contract-addition | baseline-preserved | VIOLATION |
|---|---|---|---|---|
| host | 88 | 18 | 13 | 0 |
| engine | 33 | 0 | 30 | 0 |

Notes: `In Contracts?` is `yes` when named in `plan.md` Contracts (including the rows added by this task), `no` for baseline-preserved items. Engine `udp::*`, `conn_path::*`, `endpoint::*` helpers are baseline public paths kept for tests and listed in `migration-map.md`. `Protocol`, `UdpLimits`, `TransportKind` in host are re-exports of the engine types.

## Items

| Item | Crate | In Contracts? | Classification | Note |
|---|---|---|---|---|
| `AccessReady` | host | yes | contract |  |
| `ReadyReport` | host | yes | contract |  |
| `RunContext` | host | yes | contract |  |
| `ServeReady` | host | yes | contract |  |
| `config::AccessConfig` | host | yes | contract |  |
| `config::AccessService` | host | yes | contract |  |
| `config::AddService` | host | yes | contract |  |
| `config::EditAttempt` | host | yes | contract |  |
| `config::GeneratedIdentity` | host | yes | contract |  |
| `config::Mutation` | host | yes | contract |  |
| `config::NodeConfig` | host | yes | contract |  |
| `config::ServeConfig` | host | yes | contract |  |
| `config::ServeService` | host | yes | contract |  |
| `config::ServiceList` | host | yes | contract |  |
| `config::ServiceRow` | host | yes | contract |  |
| `config::ShownConfig` | host | yes | contract |  |
| `config::UdpLimits` | host | yes | contract | re-export of engine UdpLimits |
| `config::UdpSettings` | host | yes | contract |  |
| `service::Environment` | host | yes | contract |  |
| `service::InstallRequest` | host | yes | contract |  |
| `service::NativeOutput` | host | yes | contract-addition | private status_display field (std ExitStatus text) |
| `service::Pin` | host | yes | contract |  |
| `service::RetargetConsent` | host | yes | contract-addition | previous: Option<Pin> deviation |
| `service::ServiceCall` | host | yes | contract |  |
| `service::ServiceFailure` | host | yes | contract |  |
| `service::ServiceReport` | host | yes | contract |  |
| `service::ServiceStatusReport` | host | yes | contract-addition | owner: Fact<OwnerRecord> (Unavailable when not running); user decision pending (Inbox) |
| `status::AccessServiceStatus` | host | yes | contract |  |
| `status::AccessStatusFile` | host | yes | contract |  |
| `status::OwnerRecord` | host | yes | contract-addition | Task 20b re-export |
| `status::PeerConnectionStatus` | host | yes | contract |  |
| `status::PeerPathReport` | host | yes | contract |  |
| `status::RoleStatus` | host | yes | contract |  |
| `status::ServiceStatus` | host | yes | contract |  |
| `status::StatusFile` | host | yes | contract |  |
| `status::TransportStatus` | host | yes | contract |  |
| `status::UdpDrops` | host | yes | contract |  |
| `status::UdpStatus` | host | yes | contract |  |
| `update::SigstoreVerifier` | host | no | baseline-preserved | unchanged public item, moved |
| `update::UpdateContext` | host | yes | contract |  |
| `update::UpdateRequest` | host | yes | contract |  |
| `Protocol` | host | yes | contract |  |
| `ReadyServices` | host | yes | contract |  |
| `config::Disclosure` | host | yes | contract |  |
| `config::EditOutcome` | host | yes | contract |  |
| `config::EditRejection` | host | yes | contract |  |
| `config::IdentityState` | host | yes | contract |  |
| `config::KeygenOutcome` | host | yes | contract |  |
| `config::Protocol` | host | yes | contract | re-export of engine Protocol (serde via protocol_serde) |
| `config::RotationConsent` | host | yes | contract |  |
| `error::CliError` | host | yes | contract |  |
| `role::Role` | host | yes | contract-addition | status_file_name() is a Task 20b accessor |
| `service::AppliedEffect` | host | yes | contract |  |
| `service::Fact` | host | yes | contract |  |
| `service::InstallNeed` | host | yes | contract |  |
| `service::InstallOutcome` | host | yes | contract |  |
| `service::InstalledAt` | host | yes | contract-addition |  |
| `service::OutputMode` | host | yes | contract |  |
| `service::Registration` | host | yes | contract |  |
| `service::ScopeDecision` | host | yes | contract |  |
| `service::ServiceNote` | host | yes | contract-addition | extra variants RoleRunning/RoleNotRunning/TaskListing (Task 13) |
| `service::ServiceScope` | host | yes | contract |  |
| `status::Freshness` | host | yes | contract |  |
| `status::Liveness` | host | yes | contract-addition | Task 20b re-export |
| `status::Snapshot` | host | yes | contract |  |
| `status::StatusPayload` | host | yes | contract-addition | contract; accessors pid()/started_at() are Task 20b additions |
| `status::Tier` | host | yes | contract-addition | Task 20b re-export |
| `status::TransportKind` | host | yes | contract | re-export of engine TransportKind |
| `update::OverwritePolicy` | host | yes | contract |  |
| `update::Owner` | host | no | baseline-preserved | unchanged public item, moved |
| `update::OwnerDecision` | host | yes | contract |  |
| `update::UpdateOutcome` | host | yes | contract |  |
| `update::Verification` | host | yes | contract |  |
| `update::SignatureVerifier` | host | no | baseline-preserved | unchanged public item, moved |
| `config::access_settings` | host | yes | contract |  |
| `config::add_service` | host | yes | contract |  |
| `config::encode_secret_key` | host | no | baseline-preserved | unchanged public item, moved |
| `config::identity_state` | host | yes | contract |  |
| `config::keygen` | host | yes | contract |  |
| `config::list_services` | host | yes | contract |  |
| `config::prepare_edit` | host | yes | contract |  |
| `config::random_identity` | host | yes | contract |  |
| `config::remove_service` | host | yes | contract |  |
| `config::require_existing` | host | yes | contract |  |
| `config::resolve_config_path` | host | yes | contract |  |
| `config::resolve_secret_key` | host | no | baseline-preserved | unchanged public item, moved |
| `config::resolve_write_target` | host | no | contract-addition | Task 20b accessor |
| `config::serve_settings` | host | yes | contract |  |
| `config::show_config` | host | yes | contract |  |
| `paths::default_config_path` | host | no | baseline-preserved | unchanged public item, moved |
| `paths::default_state_dir` | host | no | baseline-preserved | unchanged public item, moved |
| `paths::set_migration_notice` | host | yes | contract-addition | Task 20a ruling |
| `runtime::remove_own_status_file_for_exit` | host | yes | contract |  |
| `runtime::run_foreground` | host | yes | contract |  |
| `runtime::run_quick_access` | host | yes | contract |  |
| `runtime::run_quick_serve` | host | yes | contract |  |
| `service::account_name` | host | no | contract-addition | Task 20b accessor |
| `service::decide_install_scope` | host | yes | contract |  |
| `service::default_scope` | host | no | baseline-preserved | unchanged public item, moved |
| `service::install` | host | yes | contract |  |
| `service::is_elevated` | host | no | baseline-preserved | unchanged public item, moved |
| `service::parse_scope_answer` | host | yes | contract |  |
| `service::restart` | host | yes | contract |  |
| `service::start` | host | yes | contract |  |
| `service::status` | host | yes | contract |  |
| `service::stop` | host | yes | contract |  |
| `service::uninstall` | host | yes | contract |  |
| `service::unknown_user` | host | no | contract-addition | Task 20b accessor |
| `status::describe_owner` | host | no | contract-addition | Task 20b re-export |
| `status::read_role_status` | host | yes | contract |  |
| `status::short_peer_id` | host | no | contract-addition | Task 20b re-export (engine conn_path helper) |
| `update::check_owner` | host | yes | contract |  |
| `update::current_target` | host | no | baseline-preserved | unchanged public item, moved |
| `update::detect` | host | no | baseline-preserved | unchanged public item, moved |
| `update::update` | host | yes | contract |  |
| `OnReady` | host | yes | contract-addition | bridge re-export (Task 20a) |
| `paths::MigrationNotice` | host | yes | contract-addition | Task 20a ruling |
| `update::BASE_URL_ENV` | host | no | baseline-preserved | unchanged public item, moved |
| `update::DEFAULT_BASE_URL` | host | no | baseline-preserved | unchanged public item, moved |
| `observation::AccessObservation` | engine | yes | contract |  |
| `observation::AccessServiceObservation` | engine | yes | contract |  |
| `observation::PeerConnectionObservation` | engine | yes | contract |  |
| `observation::PeerPathObservation` | engine | yes | contract |  |
| `observation::ServeObservation` | engine | yes | contract |  |
| `observation::ServiceObservation` | engine | yes | contract |  |
| `observation::TransportObservation` | engine | yes | contract |  |
| `observation::UdpDropCounts` | engine | yes | contract |  |
| `observation::UdpObservation` | engine | yes | contract |  |
| `runtime::AccessReady` | engine | yes | contract |  |
| `runtime::ReadyReport` | engine | yes | contract |  |
| `runtime::RoleHooks` | engine | yes | contract |  |
| `runtime::RunCompletion` | engine | yes | contract |  |
| `runtime::RunContext` | engine | yes | contract |  |
| `runtime::ServeReady` | engine | yes | contract |  |
| `runtime::StatusSink` | engine | yes | contract |  |
| `settings::AccessServiceSettings` | engine | yes | contract |  |
| `settings::AccessSettings` | engine | yes | contract |  |
| `settings::NodeSettings` | engine | yes | contract |  |
| `settings::RelayToken` | engine | yes | contract |  |
| `settings::ServeServiceSettings` | engine | yes | contract |  |
| `settings::ServeSettings` | engine | yes | contract |  |
| `settings::UdpLimits` | engine | yes | contract |  |
| `udp::ClientFlows` | engine | no | baseline-preserved |  |
| `udp::FlowId` | engine | no | baseline-preserved |  |
| `udp::FlowQueue` | engine | no | baseline-preserved |  |
| `udp::PeerFlows` | engine | no | baseline-preserved |  |
| `udp::UdpStats` | engine | no | baseline-preserved |  |
| `observation::TransportKind` | engine | yes | contract |  |
| `runtime::ReadyServices` | engine | yes | contract |  |
| `settings::Protocol` | engine | yes | contract |  |
| `udp::Admission` | engine | no | baseline-preserved |  |
| `udp::DecodeError` | engine | no | baseline-preserved |  |
| `udp::DropReason` | engine | no | baseline-preserved |  |
| `udp::Push` | engine | no | baseline-preserved |  |
| `udp::Refused` | engine | no | baseline-preserved |  |
| `access::bind_access` | engine | yes | contract |  |
| `access::run_access` | engine | yes | contract |  |
| `conn_path::peer_path_report` | engine | no | baseline-preserved |  |
| `conn_path::short_peer_id` | engine | no | baseline-preserved |  |
| `endpoint::build_dial_addr` | engine | no | baseline-preserved |  |
| `endpoint::home_relay` | engine | no | baseline-preserved |  |
| `endpoint::n0_default_relay_urls` | engine | no | baseline-preserved |  |
| `endpoint::node_id_string` | engine | no | baseline-preserved |  |
| `endpoint::resolve_relay_urls` | engine | no | baseline-preserved |  |
| `serve::bind_serve` | engine | yes | contract |  |
| `serve::run_serve` | engine | yes | contract |  |
| `testing::hang_point` | engine | yes | contract |  |
| `udp::decode` | engine | no | baseline-preserved |  |
| `udp::drain_queue` | engine | no | baseline-preserved |  |
| `udp::encode` | engine | no | baseline-preserved |  |
| `udp::is_transient_recv_error` | engine | no | baseline-preserved |  |
| `udp::payload_limit` | engine | no | baseline-preserved |  |
| `udp::sweep_interval` | engine | no | baseline-preserved |  |
| `runtime::BoxFuture` | engine | yes | contract |  |
| `runtime::OnReady` | engine | yes | contract |  |
| `udp::DEFAULT_MAX_FLOWS` | engine | no | baseline-preserved |  |
| `udp::DEFAULT_MAX_FLOWS_PER_PEER` | engine | no | baseline-preserved |  |
| `udp::DEFAULT_QUEUE_DEPTH` | engine | no | baseline-preserved |  |
| `udp::HEADER_LEN` | engine | no | baseline-preserved |  |
| `udp::IDLE_TIMEOUT` | engine | no | baseline-preserved |  |
| `udp::QUARANTINE` | engine | no | baseline-preserved |  |
| `udp::SWEEP_INTERVAL` | engine | no | baseline-preserved |  |
