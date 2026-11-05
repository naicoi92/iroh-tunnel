# Rust API and repository-path migration map

Packages: root `iroh-tunnel` (CLI, three binaries, minimal `[lib]`) → `crates/host` (`iroh_tunnel_host`) → `crates/engine` (`iroh_tunnel_engine`). No wire, TOML, status-schema or service-registration change.

## 1. Rust API map (old → new)

Every public path in `baseline/public-api.txt` has one row. "private" means the item is no longer public; use the named operation instead. "removed" means there is no equivalent item; the note gives the replacement.

| Old path | New location | Notes |
|---|---|---|
| `iroh_tunnel` | `iroh_tunnel` (root, CLI only) | Re-exports nothing from host or engine. |
| `iroh_tunnel::access` | `iroh_tunnel_engine::access` + `iroh_tunnel_host::runtime` | Role loop in engine; production startup in host. |
| `iroh_tunnel::access::PATH_POLL_INTERVAL` | `iroh_tunnel_engine::access::PATH_POLL_INTERVAL` | Only with the `test-utils` feature. |
| `iroh_tunnel::access::run` | `iroh_tunnel_host::runtime::run_foreground` | Role passed as `iroh_tunnel_host::Role`; ready/status via hooks. |
| `iroh_tunnel::access::run_quick` | `iroh_tunnel_host::runtime::run_quick_access` |  |
| `iroh_tunnel::access::run_with_shutdown` | removed — `iroh_tunnel_host::testing::run_unowned` (test-utils) | Test bypass moved to test support. |
| `iroh_tunnel::access::run_with_shutdown_with_state_dir` | removed — `iroh_tunnel_host::testing::run_unowned` (test-utils) | Test bypass moved to test support. |
| `iroh_tunnel::app` | `iroh_tunnel::app` | Unchanged (root composition). |
| `iroh_tunnel::app::main_access` | `iroh_tunnel::app::main_access` | Unchanged (root). |
| `iroh_tunnel::app::main_all_in_one` | `iroh_tunnel::app::main_all_in_one` | Unchanged (root). |
| `iroh_tunnel::app::main_serve` | `iroh_tunnel::app::main_serve` | Unchanged (root). |
| `iroh_tunnel::cli` | `iroh_tunnel::cli` | Unchanged (root clap definitions). |
| `iroh_tunnel::cli::AccessCli` | `iroh_tunnel::cli::AccessCli` | Unchanged (root). |
| `iroh_tunnel::cli::AccessCli::into_cli` | `iroh_tunnel::cli::AccessCli::into_cli` | Unchanged (root). |
| `iroh_tunnel::cli::AccessQuickArgs` | `iroh_tunnel::cli::AccessQuickArgs` | Unchanged (root). |
| `iroh_tunnel::cli::AddServiceArgs` | `iroh_tunnel::cli::AddServiceArgs` | Unchanged (root). |
| `iroh_tunnel::cli::Cli` | `iroh_tunnel::cli::Cli` | Unchanged (root). |
| `iroh_tunnel::cli::ConfigAction` | `iroh_tunnel::cli::ConfigAction` | Unchanged (root). |
| `iroh_tunnel::cli::init_tracing` | `iroh_tunnel::cli::init_tracing` | Unchanged (root). |
| `iroh_tunnel::cli::KeygenMode` | `iroh_tunnel::cli::KeygenMode` | Unchanged (root). |
| `iroh_tunnel::cli::Role` | `iroh_tunnel::cli::Role` | Root clap enum; maps to `iroh_tunnel_host::Role`. |
| `iroh_tunnel::cli::RoleCmd` | `iroh_tunnel::cli::RoleCmd` | Unchanged (root). |
| `iroh_tunnel::cli::ScopeArgs` | `iroh_tunnel::cli::ScopeArgs` | Unchanged (root). |
| `iroh_tunnel::cli::ServeCli` | `iroh_tunnel::cli::ServeCli` | Unchanged (root). |
| `iroh_tunnel::cli::ServeCli::into_cli` | `iroh_tunnel::cli::ServeCli::into_cli` | Unchanged (root). |
| `iroh_tunnel::cli::ServeQuickArgs` | `iroh_tunnel::cli::ServeQuickArgs` | Unchanged (root). |
| `iroh_tunnel::cli::ServiceAction` | `iroh_tunnel::cli::ServiceAction` | Unchanged (root). |
| `iroh_tunnel::cli::UpdateArgs` | `iroh_tunnel::cli::UpdateArgs` | Root-only flags; translated to `iroh_tunnel_host::update::UpdateRequest`. |
| `iroh_tunnel::config` | `iroh_tunnel_host::config` | DTOs, validation and use cases. |
| `iroh_tunnel::config_cmd` | `iroh_tunnel::config_cmd` | Root prompts/rendering only; use cases are `iroh_tunnel_host::config`. |
| `iroh_tunnel::config_cmd::add` | `iroh_tunnel_host::config::add_service` + root `iroh_tunnel::config_cmd::add` | Use case in host; CLI wrapper (prompts/output) in root. |
| `iroh_tunnel::config_cmd::edit` | `iroh_tunnel_host::config::prepare_edit` + root `iroh_tunnel::config_cmd::edit_cli` | Use case in host; CLI wrapper (prompts/output) in root. |
| `iroh_tunnel::config_cmd::keygen` | `iroh_tunnel_host::config::keygen` + root `iroh_tunnel::config_cmd::keygen_cli` | Use case in host; CLI wrapper (prompts/output) in root. |
| `iroh_tunnel::config_cmd::keygen_random` | `iroh_tunnel_host::config::random_identity` + root `iroh_tunnel::config_cmd::keygen_random` | Use case in host; CLI wrapper (prompts/output) in root. |
| `iroh_tunnel::config_cmd::list` | `iroh_tunnel_host::config::list_services` + root `iroh_tunnel::config_cmd::list` | Use case in host; CLI wrapper (prompts/output) in root. |
| `iroh_tunnel::config_cmd::path` | `iroh_tunnel_host::config::resolve_config_path` + root `iroh_tunnel::config_cmd::path` | Use case in host; CLI wrapper (prompts/output) in root. |
| `iroh_tunnel::config_cmd::remove` | `iroh_tunnel_host::config::remove_service` + root `iroh_tunnel::config_cmd::remove` | Use case in host; CLI wrapper (prompts/output) in root. |
| `iroh_tunnel::config_cmd::require_existing` | `iroh_tunnel_host::config::require_existing` |  |
| `iroh_tunnel::config_cmd::resolve_config_path` | `iroh_tunnel_host::config::resolve_config_path` | Takes `Role` and an optional override path. |
| `iroh_tunnel::config_cmd::show` | `iroh_tunnel_host::config::show_config` + root `iroh_tunnel::config_cmd::show` | Use case in host; CLI wrapper (prompts/output) in root. |
| `iroh_tunnel::config::AccessConfig` | `iroh_tunnel_host::config::AccessConfig` |  |
| `iroh_tunnel::config::AccessConfig::load` | `iroh_tunnel_host::config::AccessConfig::load` | Still an inherent method; prefer the use-case functions for edits. |
| `iroh_tunnel::config::AccessConfig::resolve_and_save_key` | `iroh_tunnel_host::config::AccessConfig::resolve_and_save_key` | Still an inherent method; prefer the use-case functions for edits. |
| `iroh_tunnel::config::AccessConfig::save` | `iroh_tunnel_host::config::AccessConfig::save` | Still an inherent method; prefer the use-case functions for edits. |
| `iroh_tunnel::config::AccessConfig::validate` | `iroh_tunnel_host::config::AccessConfig::validate` | Still an inherent method; prefer the use-case functions for edits. |
| `iroh_tunnel::config::AccessService` | `iroh_tunnel_host::config::AccessService` |  |
| `iroh_tunnel::config::encode_secret_key` | `iroh_tunnel_host::config::encode_secret_key` |  |
| `iroh_tunnel::config::NodeConfig` | `iroh_tunnel_host::config::NodeConfig` |  |
| `iroh_tunnel::config::NodeConfig::validate` | `iroh_tunnel_host::config::NodeConfig::validate` |  |
| `iroh_tunnel::config::Protocol` | `iroh_tunnel_host::config::Protocol` | Re-export of `iroh_tunnel_engine::settings::Protocol`. |
| `iroh_tunnel::config::resolve_secret_key` | `iroh_tunnel_host::config::resolve_secret_key` |  |
| `iroh_tunnel::config::ServeConfig` | `iroh_tunnel_host::config::ServeConfig` |  |
| `iroh_tunnel::config::ServeConfig::load` | `iroh_tunnel_host::config::ServeConfig::load` | Still an inherent method; prefer the use-case functions for edits. |
| `iroh_tunnel::config::ServeConfig::resolve_and_save_key` | `iroh_tunnel_host::config::ServeConfig::resolve_and_save_key` | Still an inherent method; prefer the use-case functions for edits. |
| `iroh_tunnel::config::ServeConfig::save` | `iroh_tunnel_host::config::ServeConfig::save` | Still an inherent method; prefer the use-case functions for edits. |
| `iroh_tunnel::config::ServeConfig::validate` | `iroh_tunnel_host::config::ServeConfig::validate` | Still an inherent method; prefer the use-case functions for edits. |
| `iroh_tunnel::config::ServeService` | `iroh_tunnel_host::config::ServeService` |  |
| `iroh_tunnel::config::UdpLimits` | `iroh_tunnel_engine::settings::UdpLimits` | Also re-exported as `iroh_tunnel_host::config::UdpLimits`. |
| `iroh_tunnel::config::UdpSettings` | `iroh_tunnel_host::config::UdpSettings` |  |
| `iroh_tunnel::config::UdpSettings::resolve` | `iroh_tunnel_host::config::UdpSettings::resolve` | Returns `iroh_tunnel_engine::settings::UdpLimits`. |
| `iroh_tunnel::conn_path` | `iroh_tunnel_engine::conn_path` | Observation types in `iroh_tunnel_engine::observation`; file DTOs in `iroh_tunnel_host::status`. |
| `iroh_tunnel::conn_path::peer_path_report` | `iroh_tunnel_engine::conn_path::peer_path_report` | Returns `PeerPathObservation`. |
| `iroh_tunnel::conn_path::PeerPathReport` | `iroh_tunnel_host::status::PeerPathReport` + `iroh_tunnel_engine::observation::PeerPathObservation` | File DTO in host; native value in engine. |
| `iroh_tunnel::conn_path::TransportKind` | `iroh_tunnel_host::status::TransportKind` + `iroh_tunnel_engine::observation::TransportKind` |  |
| `iroh_tunnel::conn_path::TransportStatus` | `iroh_tunnel_host::status::TransportStatus` + `iroh_tunnel_engine::observation::TransportObservation` |  |
| `iroh_tunnel::endpoint` | `iroh_tunnel_engine::endpoint` |  |
| `iroh_tunnel::endpoint::build_dial_addr` | `iroh_tunnel_engine::endpoint::build_dial_addr` |  |
| `iroh_tunnel::endpoint::create_access_endpoint` | `iroh_tunnel_engine::access::bind_access` | Renamed to `bind_*`. |
| `iroh_tunnel::endpoint::create_serve_endpoint` | `iroh_tunnel_engine::serve::bind_serve` | Renamed to `bind_*`. |
| `iroh_tunnel::endpoint::home_relay` | `iroh_tunnel_engine::endpoint::home_relay` |  |
| `iroh_tunnel::endpoint::n0_default_relay_urls` | `iroh_tunnel_engine::endpoint::n0_default_relay_urls` |  |
| `iroh_tunnel::endpoint::node_id_string` | `iroh_tunnel_engine::endpoint::node_id_string` |  |
| `iroh_tunnel::endpoint::resolve_relay_urls` | `iroh_tunnel_engine::endpoint::resolve_relay_urls` |  |
| `iroh_tunnel::endpoint::set_relay_only_for_tests` | `iroh_tunnel_engine::testing::set_relay_only` | Only with the `test-utils` feature. |
| `iroh_tunnel::error` | `iroh_tunnel_host::error` |  |
| `iroh_tunnel::error::CliError` | `iroh_tunnel_host::error::CliError` |  |
| `iroh_tunnel::paths` | `iroh_tunnel_host::paths` |  |
| `iroh_tunnel::paths::config_path_in` | private — use `iroh_tunnel_host::paths::default_config_path` |  |
| `iroh_tunnel::paths::default_config_path` | `iroh_tunnel_host::paths::default_config_path` |  |
| `iroh_tunnel::paths::default_state_dir` | `iroh_tunnel_host::paths::default_state_dir` |  |
| `iroh_tunnel::paths::state_dir_in` | private — use `iroh_tunnel_host::paths::default_state_dir` |  |
| `iroh_tunnel::pipe` | private — use `iroh_tunnel_engine::serve::run_serve` / `iroh_tunnel_engine::access::run_access` | TCP piping is engine-internal. |
| `iroh_tunnel::pipe::pipe_tcp_bidirectional` | private — use `iroh_tunnel_engine::serve::run_serve` / `iroh_tunnel_engine::access::run_access` |  |
| `iroh_tunnel::proto` | private — use `iroh_tunnel_engine::serve::run_serve` / `iroh_tunnel_engine::access::run_access` | ALPN format unchanged on the wire. |
| `iroh_tunnel::proto::alpn_for` | private — use `iroh_tunnel_engine::serve::run_serve` / `iroh_tunnel_engine::access::run_access` | ALPN `iroh-tunnel/{service}` unchanged. |
| `iroh_tunnel::proto::ALPN_PREFIX` | private — use `iroh_tunnel_engine::serve::run_serve` / `iroh_tunnel_engine::access::run_access` | ALPN `iroh-tunnel/{service}` unchanged. |
| `iroh_tunnel::proto::name_from_alpn` | private — use `iroh_tunnel_engine::serve::run_serve` / `iroh_tunnel_engine::access::run_access` | ALPN `iroh-tunnel/{service}` unchanged. |
| `iroh_tunnel::serve` | `iroh_tunnel_engine::serve` + `iroh_tunnel_host::runtime` | Role loop in engine; production startup in host. |
| `iroh_tunnel::serve::run` | `iroh_tunnel_host::runtime::run_foreground` | Role passed as `iroh_tunnel_host::Role`; ready/status via hooks. |
| `iroh_tunnel::serve::run_quick` | `iroh_tunnel_host::runtime::run_quick_serve` |  |
| `iroh_tunnel::serve::run_with_shutdown` | removed — `iroh_tunnel_host::testing::run_unowned` (test-utils) | Test bypass moved to test support. |
| `iroh_tunnel::serve::run_with_shutdown_with_state_dir` | removed — `iroh_tunnel_host::testing::run_unowned` (test-utils) | Test bypass moved to test support. |
| `iroh_tunnel::service` | `iroh_tunnel_host::service` |  |
| `iroh_tunnel::service::account` | private — use `iroh_tunnel_host::service::account_name` | Account lookup is host-internal. |
| `iroh_tunnel::service::account::Account` | private — use `iroh_tunnel_host::service::account_name` |  |
| `iroh_tunnel::service::account::by_name` | private — use `iroh_tunnel_host::service::account_name` |  |
| `iroh_tunnel::service::account::by_uid` | private — use `iroh_tunnel_host::service::account_name` |  |
| `iroh_tunnel::service::existing_registration` | private — use `iroh_tunnel_host::service::status` | `ServiceStatusReport` carries the `Registration`. |
| `iroh_tunnel::service::install` | `iroh_tunnel_host::service::install` | Takes `InstallRequest`, returns `InstallOutcome`. |
| `iroh_tunnel::service::pin` | `iroh_tunnel_host::service::Pin` |  |
| `iroh_tunnel::service::pin::Pin` | `iroh_tunnel_host::service::Pin` |  |
| `iroh_tunnel::service::pin::Pin::find` | `iroh_tunnel_host::service::Pin::find` |  |
| `iroh_tunnel::service::pin::Pin::record` | `iroh_tunnel_host::service::Pin::record` |  |
| `iroh_tunnel::service::restart` | `iroh_tunnel_host::service::restart` | New signature: typed request/`ServiceCall`, returns `ServiceReport`; see rustdoc. |
| `iroh_tunnel::service::scope` | removed — use `iroh_tunnel_host::service::{Environment, ScopeDecision, decide_install_scope, default_scope, is_elevated}` |  |
| `iroh_tunnel::service::scope::choose_install_scope` | `iroh_tunnel_host::service::decide_install_scope` | Renamed; returns `ScopeDecision`. |
| `iroh_tunnel::service::scope::default_scope` | `iroh_tunnel_host::service::default_scope` |  |
| `iroh_tunnel::service::scope::ensure_privilege` | private — use `iroh_tunnel_host::service::install` | Enforced inside the operations. |
| `iroh_tunnel::service::scope::ensure_supported` | private — use `iroh_tunnel_host::service::install` | Enforced inside the operations. |
| `iroh_tunnel::service::scope::Environment` | `iroh_tunnel_host::service::Environment` |  |
| `iroh_tunnel::service::scope::Environment::detect` | removed — root builds `iroh_tunnel_host::service::Environment` (`cli_service::detect_environment`) | Host does no environment/tty probing for the caller. |
| `iroh_tunnel::service::scope::is_elevated` | `iroh_tunnel_host::service::is_elevated` |  |
| `iroh_tunnel::service::ServiceScope` | `iroh_tunnel_host::service::ServiceScope` |  |
| `iroh_tunnel::service::start` | `iroh_tunnel_host::service::start` | New signature: typed request/`ServiceCall`, returns `ServiceReport`; see rustdoc. |
| `iroh_tunnel::service::status` | `iroh_tunnel_host::service::status` | New signature: typed request/`ServiceCall`, returns `ServiceReport`; see rustdoc. |
| `iroh_tunnel::service::stop` | `iroh_tunnel_host::service::stop` | New signature: typed request/`ServiceCall`, returns `ServiceReport`; see rustdoc. |
| `iroh_tunnel::service::system` | private — use `iroh_tunnel_host::service::install` | Target resolution is host-internal. |
| `iroh_tunnel::service::system::confirm_retarget` | removed — root prompts; pass `iroh_tunnel_host::service::RetargetConsent` in `InstallRequest` |  |
| `iroh_tunnel::service::system::Request` | private — use `iroh_tunnel_host::service::install` | Target resolution is host-internal. |
| `iroh_tunnel::service::system::resolve_target` | private — use `iroh_tunnel_host::service::install` | Target resolution is host-internal. |
| `iroh_tunnel::service::system::Target` | private — use `iroh_tunnel_host::service::install` | Target resolution is host-internal. |
| `iroh_tunnel::service::system::Target::pin` | private — use `iroh_tunnel_host::service::install` | Target resolution is host-internal. |
| `iroh_tunnel::service::trust` | private — use `iroh_tunnel_host::service::install` | Trusted-directory checks are host-internal. |
| `iroh_tunnel::service::trust::canonical` | private — use `iroh_tunnel_host::service::install` | Trusted-directory checks are host-internal. |
| `iroh_tunnel::service::trust::ENV_TRUSTED_UID` | private — use `iroh_tunnel_host::service::install` | Trusted-directory checks are host-internal. |
| `iroh_tunnel::service::trust::read_verified` | private — use `iroh_tunnel_host::service::install` | Trusted-directory checks are host-internal. |
| `iroh_tunnel::service::trust::verify_dir` | private — use `iroh_tunnel_host::service::install` | Trusted-directory checks are host-internal. |
| `iroh_tunnel::service::uninstall` | `iroh_tunnel_host::service::uninstall` | New signature: typed request/`ServiceCall`, returns `ServiceReport`; see rustdoc. |
| `iroh_tunnel::shutdown` | private — use `iroh_tunnel_host::runtime::run_foreground` | Signals/watchdog/exit policy live in root `lifecycle`; drain mechanics in `RunContext`. |
| `iroh_tunnel::shutdown::forced_exit` | private — root `lifecycle` (CLI signal/exit policy) | Drain context is `iroh_tunnel_host::RunContext`. |
| `iroh_tunnel::shutdown::ForcedExit` | private — root `lifecycle` (CLI signal/exit policy) | Drain context is `iroh_tunnel_host::RunContext`. |
| `iroh_tunnel::shutdown::ForcedExit::exit_code` | private — root `lifecycle` (CLI signal/exit policy) | Drain context is `iroh_tunnel_host::RunContext`. |
| `iroh_tunnel::shutdown::test_hang_point` | `iroh_tunnel_engine::testing::hang_point` | Only with the `test-utils` feature. |
| `iroh_tunnel::shutdown::wait_for_signal` | private — root `lifecycle` (CLI signal/exit policy) | Drain context is `iroh_tunnel_host::RunContext`. |
| `iroh_tunnel::status` | `iroh_tunnel_host::status` |  |
| `iroh_tunnel::status_cmd` | `iroh_tunnel::status_cmd` | Unchanged (root renderers). |
| `iroh_tunnel::status_cmd::render_access_status` | `iroh_tunnel::status_cmd::render_access_status` | Unchanged (root). |
| `iroh_tunnel::status_cmd::render_serve_status` | `iroh_tunnel::status_cmd::render_serve_status` | Unchanged (root). |
| `iroh_tunnel::status_cmd::run` | `iroh_tunnel::status_cmd::run` | Unchanged (root). |
| `iroh_tunnel::status::AccessServiceStatus` | `iroh_tunnel_host::status::AccessServiceStatus` |  |
| `iroh_tunnel::status::AccessStatusFile` | `iroh_tunnel_host::status::AccessStatusFile` |  |
| `iroh_tunnel::status::PeerConnectionStatus` | `iroh_tunnel_host::status::PeerConnectionStatus` |  |
| `iroh_tunnel::status::remove_own_status_file_for_exit` | `iroh_tunnel_host::runtime::remove_own_status_file_for_exit` |  |
| `iroh_tunnel::status::ServiceStatus` | `iroh_tunnel_host::status::ServiceStatus` |  |
| `iroh_tunnel::status::StatusFile` | `iroh_tunnel_host::status::StatusFile` |  |
| `iroh_tunnel::status::StatusPayload` | `iroh_tunnel_host::status::StatusPayload` |  |
| `iroh_tunnel::status::StatusPayload::role` | `iroh_tunnel_host::status::StatusPayload::role` |  |
| `iroh_tunnel::status::StatusRole` | `iroh_tunnel_host::Role` | Renamed. |
| `iroh_tunnel::status::StatusRole::file_name` | `iroh_tunnel_host::Role::status_file_name` | Renamed. |
| `iroh_tunnel::status::StatusRole::name` | `iroh_tunnel_host::Role::name` |  |
| `iroh_tunnel::status::StatusWriter` | private — use `iroh_tunnel_host::status::read_role_status` | Host writes status itself via the engine `StatusSink` callback. |
| `iroh_tunnel::status::StatusWriter::access` | private — use `iroh_tunnel_host::status::read_role_status` | Host writes status itself via the engine `StatusSink` callback. |
| `iroh_tunnel::status::StatusWriter::file_name` | private — use `iroh_tunnel_host::status::read_role_status` | Host writes status itself via the engine `StatusSink` callback. |
| `iroh_tunnel::status::StatusWriter::new` | private — use `iroh_tunnel_host::status::read_role_status` | Host writes status itself via the engine `StatusSink` callback. |
| `iroh_tunnel::status::StatusWriter::path` | private — use `iroh_tunnel_host::status::read_role_status` | Host writes status itself via the engine `StatusSink` callback. |
| `iroh_tunnel::status::StatusWriter::path_for_state_dir` | private — use `iroh_tunnel_host::status::read_role_status` | Host writes status itself via the engine `StatusSink` callback. |
| `iroh_tunnel::status::StatusWriter::path_with` | private — use `iroh_tunnel_host::status::read_role_status` | Host writes status itself via the engine `StatusSink` callback. |
| `iroh_tunnel::status::StatusWriter::role` | private — use `iroh_tunnel_host::status::read_role_status` | Host writes status itself via the engine `StatusSink` callback. |
| `iroh_tunnel::status::StatusWriter::save` | private — use `iroh_tunnel_host::status::read_role_status` | Host writes status itself via the engine `StatusSink` callback. |
| `iroh_tunnel::status::StatusWriter::save_to` | private — use `iroh_tunnel_host::status::read_role_status` | Host writes status itself via the engine `StatusSink` callback. |
| `iroh_tunnel::status::StatusWriter::save_with_state_dir` | private — use `iroh_tunnel_host::status::read_role_status` | Host writes status itself via the engine `StatusSink` callback. |
| `iroh_tunnel::status::StatusWriter::serve` | private — use `iroh_tunnel_host::status::read_role_status` | Host writes status itself via the engine `StatusSink` callback. |
| `iroh_tunnel::status::UdpDrops` | `iroh_tunnel_host::status::UdpDrops` |  |
| `iroh_tunnel::status::UdpStatus` | `iroh_tunnel_host::status::UdpStatus` |  |
| `iroh_tunnel::udp` | `iroh_tunnel_engine::udp` |  |
| `iroh_tunnel::udp::Admission` | `iroh_tunnel_engine::udp::Admission` |  |
| `iroh_tunnel::udp::ClientFlows` | `iroh_tunnel_engine::udp::ClientFlows` |  |
| `iroh_tunnel::udp::ClientFlows::client_of` | `iroh_tunnel_engine::udp::ClientFlows::client_of` |  |
| `iroh_tunnel::udp::ClientFlows::expire` | `iroh_tunnel_engine::udp::ClientFlows::expire` |  |
| `iroh_tunnel::udp::ClientFlows::is_empty` | `iroh_tunnel_engine::udp::ClientFlows::is_empty` |  |
| `iroh_tunnel::udp::ClientFlows::len` | `iroh_tunnel_engine::udp::ClientFlows::len` |  |
| `iroh_tunnel::udp::ClientFlows::touch` | `iroh_tunnel_engine::udp::ClientFlows::touch` |  |
| `iroh_tunnel::udp::decode` | `iroh_tunnel_engine::udp::decode` |  |
| `iroh_tunnel::udp::DecodeError` | `iroh_tunnel_engine::udp::DecodeError` |  |
| `iroh_tunnel::udp::DEFAULT_MAX_FLOWS` | `iroh_tunnel_engine::udp::DEFAULT_MAX_FLOWS` |  |
| `iroh_tunnel::udp::DEFAULT_MAX_FLOWS_PER_PEER` | `iroh_tunnel_engine::udp::DEFAULT_MAX_FLOWS_PER_PEER` |  |
| `iroh_tunnel::udp::DEFAULT_QUEUE_DEPTH` | `iroh_tunnel_engine::udp::DEFAULT_QUEUE_DEPTH` |  |
| `iroh_tunnel::udp::drain_queue` | `iroh_tunnel_engine::udp::drain_queue` |  |
| `iroh_tunnel::udp::DropReason` | `iroh_tunnel_engine::udp::DropReason` |  |
| `iroh_tunnel::udp::encode` | `iroh_tunnel_engine::udp::encode` |  |
| `iroh_tunnel::udp::FlowId` | `iroh_tunnel_engine::udp::FlowId` |  |
| `iroh_tunnel::udp::FlowQueue` | `iroh_tunnel_engine::udp::FlowQueue` |  |
| `iroh_tunnel::udp::FlowQueue::new` | `iroh_tunnel_engine::udp::FlowQueue::new` |  |
| `iroh_tunnel::udp::FlowQueue::push` | `iroh_tunnel_engine::udp::FlowQueue::push` |  |
| `iroh_tunnel::udp::HEADER_LEN` | `iroh_tunnel_engine::udp::HEADER_LEN` |  |
| `iroh_tunnel::udp::IDLE_TIMEOUT` | `iroh_tunnel_engine::udp::IDLE_TIMEOUT` |  |
| `iroh_tunnel::udp::is_transient_recv_error` | `iroh_tunnel_engine::udp::is_transient_recv_error` |  |
| `iroh_tunnel::udp::payload_limit` | `iroh_tunnel_engine::udp::payload_limit` |  |
| `iroh_tunnel::udp::PeerFlows` | `iroh_tunnel_engine::udp::PeerFlows` |  |
| `iroh_tunnel::udp::PeerFlows::admit` | `iroh_tunnel_engine::udp::PeerFlows::admit` |  |
| `iroh_tunnel::udp::PeerFlows::drain_all` | `iroh_tunnel_engine::udp::PeerFlows::drain_all` |  |
| `iroh_tunnel::udp::PeerFlows::expire` | `iroh_tunnel_engine::udp::PeerFlows::expire` |  |
| `iroh_tunnel::udp::PeerFlows::get` | `iroh_tunnel_engine::udp::PeerFlows::get` |  |
| `iroh_tunnel::udp::PeerFlows::insert` | `iroh_tunnel_engine::udp::PeerFlows::insert` |  |
| `iroh_tunnel::udp::PeerFlows::is_empty` | `iroh_tunnel_engine::udp::PeerFlows::is_empty` |  |
| `iroh_tunnel::udp::PeerFlows::len` | `iroh_tunnel_engine::udp::PeerFlows::len` |  |
| `iroh_tunnel::udp::PeerFlows::refresh` | `iroh_tunnel_engine::udp::PeerFlows::refresh` |  |
| `iroh_tunnel::udp::PeerFlows::refuse` | `iroh_tunnel_engine::udp::PeerFlows::refuse` |  |
| `iroh_tunnel::udp::Push` | `iroh_tunnel_engine::udp::Push` |  |
| `iroh_tunnel::udp::QUARANTINE` | `iroh_tunnel_engine::udp::QUARANTINE` |  |
| `iroh_tunnel::udp::Refused` | `iroh_tunnel_engine::udp::Refused` |  |
| `iroh_tunnel::udp::sweep_interval` | `iroh_tunnel_engine::udp::sweep_interval` |  |
| `iroh_tunnel::udp::SWEEP_INTERVAL` | `iroh_tunnel_engine::udp::SWEEP_INTERVAL` |  |
| `iroh_tunnel::udp::UdpStats` | `iroh_tunnel_engine::udp::UdpStats` |  |
| `iroh_tunnel::udp::UdpStats::flows_closed` | `iroh_tunnel_engine::udp::UdpStats::flows_closed` |  |
| `iroh_tunnel::udp::UdpStats::quarantine_ended` | `iroh_tunnel_engine::udp::UdpStats::quarantine_ended` |  |
| `iroh_tunnel::udp::UdpStats::quarantine_started` | `iroh_tunnel_engine::udp::UdpStats::quarantine_started` |  |
| `iroh_tunnel::udp::UdpStats::record_drop` | `iroh_tunnel_engine::udp::UdpStats::record_drop` |  |
| `iroh_tunnel::udp::UdpStats::set_active_flows` | `iroh_tunnel_engine::udp::UdpStats::set_active_flows` |  |
| `iroh_tunnel::udp::UdpStats::set_datagram_limit` | `iroh_tunnel_engine::udp::UdpStats::set_datagram_limit` |  |
| `iroh_tunnel::udp::UdpStats::snapshot` | `iroh_tunnel_engine::udp::UdpStats::snapshot` | Returns `iroh_tunnel_engine::observation::UdpObservation`. |
| `iroh_tunnel::udp::UdpStats::try_open_flow` | `iroh_tunnel_engine::udp::UdpStats::try_open_flow` |  |
| `iroh_tunnel::update` | `iroh_tunnel_host::update` | CLI flag translation stays in root. |
| `iroh_tunnel::update_owner` | `iroh_tunnel_host::update` | Merged into `update`. |
| `iroh_tunnel::update_owner::classify` | private — use `iroh_tunnel_host::update::detect` |  |
| `iroh_tunnel::update_owner::CONTAINER_ENV` | private — use `iroh_tunnel_host::update::detect` |  |
| `iroh_tunnel::update_owner::detect` | `iroh_tunnel_host::update::detect` |  |
| `iroh_tunnel::update_owner::in_homebrew_prefix` | private — use `iroh_tunnel_host::update::detect` |  |
| `iroh_tunnel::update_owner::Owner` | `iroh_tunnel_host::update::Owner` |  |
| `iroh_tunnel::update_owner::Owner::guidance` | `iroh_tunnel_host::update::Owner::guidance` |  |
| `iroh_tunnel::update_owner::Owner::overwrite_allowed_with_confirmation` | `iroh_tunnel_host::update::Owner::overwrite_allowed_with_confirmation` |  |
| `iroh_tunnel::update_owner::Owner::Standalone` | `iroh_tunnel_host::update::Owner::Standalone` |  |
| `iroh_tunnel::update_owner::Probe` | private — use `iroh_tunnel_host::update::detect` |  |
| `iroh_tunnel::update::replace_executable` | private — use `iroh_tunnel_host::update::update` |  |
| `iroh_tunnel::update::run` | removed — use `iroh_tunnel_host::update::update` with `UpdateRequest` | Root `run_update_cli` translates `UpdateArgs`. |
| `iroh_tunnel::update::SignatureVerifier` | `iroh_tunnel_host::update::SignatureVerifier` |  |
| `iroh_tunnel::update::SigstoreVerifier` | `iroh_tunnel_host::update::SigstoreVerifier` |  |
| `iroh_tunnel::update::SigstoreVerifier::new` | `iroh_tunnel_host::update::SigstoreVerifier::new` |  |
| `iroh_tunnel::update::SigstoreVerifier::release` | `iroh_tunnel_host::update::SigstoreVerifier::release` |  |
| `iroh_tunnel::update::update` | `iroh_tunnel_host::update::update` | Takes `UpdateRequest` + `UpdateContext`. |
| `iroh_tunnel::update::UpdateContext` | `iroh_tunnel_host::update::UpdateContext` |  |

## 2. Repository-path map

| Old path | New location | Notes |
|---|---|---|
| `src/config.rs` | `crates/host/src/config/dto.rs`, `types.rs`, `to_settings.rs`; `crates/engine/src/settings.rs` | DTO/validation/identity in host; execution settings in engine. |
| `src/config_store.rs` | `crates/host/src/config/store.rs`, `mutate.rs` | Store and write-target logic. |
| `src/config_cmd.rs` | `crates/host/src/config/*` (use cases); `src/config_cmd.rs` (CLI wrappers) | Prompts/editor/rendering stay in root. |
| `src/service/**` | `crates/host/src/service/**` | Raw backends are private (`service/backend/`). |
| service policy in `src/app.rs` | `crates/host/src/service/policy.rs`, `decide_install_scope.rs`; `src/cli_service.rs` | Root maps prompts/output and supplies the tunnel binary. |
| `src/status.rs` | `crates/host/src/status/**` | Schema, writer, read interpretation. |
| `src/status_cmd.rs` | `src/status_cmd.rs`; `crates/host/src/status/read_role_status.rs` | Renderers stay in root. |
| `src/role_run.rs` | `crates/host/src/runtime/**` | Foreground runtime. |
| `src/ownership.rs` | `crates/host/src/ownership.rs` | Crate-private; status exposes `describe_owner`. |
| `src/paths.rs` | `crates/host/src/paths/**` | |
| `src/serve.rs` | `crates/engine/src/serve/**`; `crates/host/src/runtime/run_quick_serve.rs` | |
| `src/access.rs` | `crates/engine/src/access/**`; `crates/host/src/runtime/run_quick_access.rs` | |
| `src/endpoint.rs` | `crates/engine/src/endpoint/**` | |
| `src/proto.rs` | `crates/engine/src/proto.rs` | Private. |
| `src/pipe.rs` | `crates/engine/src/tcp.rs` | Private. |
| `src/udp.rs` | `crates/engine/src/udp/**` | |
| `src/conn_path.rs` | `crates/engine/src/conn_path/**`, `crates/engine/src/observation/**`; DTOs in `crates/host/src/status/schema.rs` | |
| `src/shutdown.rs` | `crates/engine/src/shutdown.rs`, `runtime/run_context.rs`; `src/lifecycle.rs` | Signal/watchdog/exit policy in root. |
| `src/update.rs`, `src/update_owner.rs` | `crates/host/src/update/**` | |
| `src/error.rs` | `crates/host/src/error.rs` | |
| `src/cli.rs`, `src/app.rs`, three binary entrypoints, `build.rs` | unchanged in root (`src/`, `build.rs`) | Binary names unchanged. |
| test bypass entrypoints | `crates/host/src/testing.rs`, `crates/engine/src/testing.rs` | Behind the `test-utils` feature. |
| Root `tests/` and fixtures | real-binary/distribution tests stay in `tests/`; in-process tests live with their owners | |

## 3. Contract deviations from the original plan

- **`RetargetConsent.previous` is `Option<Pin>`** (user-approved): consent can be given for a legacy unpinned registration (`None`).
- **`MigrationNotice` hook**: `iroh_tunnel_host::paths::set_migration_notice` lets a frontend show the macOS legacy-configuration copy; without one the host logs at `info`.
- **`OnReady` bridge re-export**: `iroh_tunnel_host` re-exports `OnReady`, `ReadyReport`, `AccessReady`, `ServeReady`, `ReadyServices`, `RunContext`, so the root needs no direct engine dependency.
- **`ServiceNote` has extra variants** (`BusyboxSystemWide`, `LaunchdBootstrapHint`, `Installed`, `Uninstalled`, `AlreadyStopped`, `RoleRunning`, `RoleNotRunning`) so hosts never print.
- **`NativeOutput` carries `status_display`**, the rendered native status text for the frontend to show.
