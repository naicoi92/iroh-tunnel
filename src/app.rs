//! Shared entry points for the three role binaries (`iroh-tunnel`,
//! `iroh-tunnel-serve`, `iroh-tunnel-access`): parse, init tracing, build the
//! runtime, dispatch and map errors to exit codes (see [`CliError`], RS-06).
//!
//! The binaries are thin wrappers so each one is a self-contained executable
//! that never needs another binary at runtime.

use std::path::{Path, PathBuf};

use crate::cli;
use crate::cli::{Binary, Cli, ConfigAction, Role, RoleCmd, ServiceArgs};
use crate::cli_args::{ProtocolArg, ServiceAddr};
use crate::config_cmd;
use crate::lifecycle;
use crate::status_cmd;
use anyhow::Context as _;
use iroh_tunnel_host::config as config_ops;
use iroh_tunnel_host::config::SetError;
use iroh_tunnel_host::error::CliError;
use iroh_tunnel_host::runtime;
use iroh_tunnel_host::update;
use iroh_tunnel_host::Role as StatusRole;
use iroh_tunnel_host::RunContext;

/// Entry point of the all-in-one `iroh-tunnel` (`it`) binary.
pub fn main_all_in_one() -> ! {
    install_migration_notice();
    execute(parse_or_exit(Binary::AllInOne), Binary::AllInOne)
}

/// Entry point of the standalone `iroh-tunnel-serve` (`its`) binary.
pub fn main_serve() -> ! {
    install_migration_notice();
    execute(parse_or_exit(Binary::Serve), Binary::Serve)
}

/// Entry point of the standalone `iroh-tunnel-access` (`ita`) binary.
pub fn main_access() -> ! {
    install_migration_notice();
    execute(parse_or_exit(Binary::Access), Binary::Access)
}

/// Show the operator the macOS legacy-configuration copy on stderr; the host
/// only reports it through this observer. Runs before any path resolution.
fn install_migration_notice() {
    iroh_tunnel_host::paths::set_migration_notice(|legacy, dest| {
        eprintln!(
            "migrated configuration {} -> {} (original kept)",
            legacy.display(),
            dest.display()
        );
    });
}

/// Parse the process arguments; clap renders usage errors and exits.
fn parse_or_exit(binary: Binary) -> Cli {
    cli::parse_cli(binary, std::env::args_os()).unwrap_or_else(|e| e.exit())
}

/// Host of a quick-mode address; a bare port means loopback.
fn quick_host(addr: &ServiceAddr) -> String {
    addr.host.clone().unwrap_or_else(|| "127.0.0.1".to_owned())
}

fn execute(parsed: Cli, binary: Binary) -> ! {
    cli::init_tracing(parsed.verbose, parsed.quiet);

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: failed to start runtime: {e}");
            std::process::exit(1);
        }
    };

    // Role to clean up if shutdown is abandoned; only `run` commands own
    // a status file, but removal is pid-checked so other commands are safe.
    let status_role = match &parsed.role {
        Role::Serve { .. } => Some(StatusRole::Serve),
        Role::Access { .. } => Some(StatusRole::Access),
        Role::Update(_) => None,
    };
    // Racing the run future means a forced exit first DROPS it: owned tasks
    // and the endpoint are cancelled before the process status is decided.
    // One drain context per invocation. Management commands never begin a
    // drain, so `forced_exit` stays pending and installs no signal handler.
    let ctx = RunContext::new();
    let outcome = rt.block_on(async {
        tokio::select! {
            result = run(parsed, binary, ctx.clone()) => Ok(result),
            forced = lifecycle::forced_exit(&ctx) => Err(forced),
        }
    });
    let outcome = match outcome {
        Ok(result) => result,
        Err(forced) => {
            // Exit statuses: 130 repeated stop after grace, 1 watchdog (see
            // CHANGELOG). Own-state cleanup is still attempted here.
            eprintln!("error: shutdown abandoned ({forced:?}), exiting");
            if let Some(role) = status_role {
                runtime::remove_own_status_file_for_exit(role);
            }
            std::process::exit(forced.exit_code());
        }
    };
    match outcome {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("error: {e:#}");
            let code = if e.downcast_ref::<SetError>().is_some() {
                2
            } else {
                match e.downcast_ref::<CliError>() {
                    Some(CliError::Config(_)) => 2,
                    Some(CliError::Permission(_)) => 3,
                    Some(CliError::Iroh(_)) => 4,
                    Some(CliError::Service(_)) => 5,
                    Some(CliError::Conflict(_)) => 6,
                    None => 1,
                }
            };
            std::process::exit(code);
        }
    }
}

async fn run(cli: Cli, binary: Binary, ctx: RunContext) -> anyhow::Result<()> {
    // The typed role tag comes from ONE match on the CLI enum; the dispatch
    // string is derived from it — an invalid role is unrepresentable, and
    // the two can never disagree.
    let status_role = match &cli.role {
        Role::Serve { .. } => StatusRole::Serve,
        Role::Access { .. } => StatusRole::Access,
        Role::Update(args) => {
            return run_update_cli(args, "iroh-tunnel").await;
        }
    };
    match cli.role {
        Role::Update(_) => unreachable!("handled above"),
        Role::Serve { cmd: Some(cmd), .. } => {
            dispatch_role_cmd(status_role, cmd, binary, ctx).await
        }
        Role::Access { cmd: Some(cmd), .. } => {
            dispatch_role_cmd(status_role, cmd, binary, ctx).await
        }
        Role::Serve { quick, cmd: None } => {
            let svc = quick.service;
            let (name, target) = svc
                .name
                .zip(svc.target)
                .context("internal: quick mode requires --name and --target")?;
            let protocol = svc.protocol.unwrap_or(ProtocolArg::Tcp).into();
            let secret_key = read_key_file(quick.key_file.as_deref())?;
            let cfg = iroh_tunnel_host::config::ServeConfig {
                node: iroh_tunnel_host::config::NodeConfig {
                    secret_key,
                    relay_urls: quick.relay_url.into_iter().collect(),
                    ..Default::default()
                },
                services: vec![iroh_tunnel_host::config::ServeService {
                    name,
                    protocol,
                    host: quick_host(&target),
                    port: target.port.get(),
                    udp: Default::default(),
                }],
            };
            runtime::run_quick_serve(
                cfg,
                ctx,
                crate::render::print_ready(),
                lifecycle::wait_for_signal(),
            )
            .await
        }
        Role::Access { quick, cmd: None } => {
            let svc = quick.service;
            let (name, listen, node_id) = svc
                .name
                .zip(svc.listen)
                .zip(svc.node_id)
                .map(|((n, l), i)| (n, l, i))
                .context("internal: quick mode requires --name, --listen and --node-id")?;
            let protocol = svc.protocol.unwrap_or(ProtocolArg::Tcp).into();
            node_id.parse::<iroh::EndpointId>().map_err(|e| {
                CliError::Config(format!("access quick mode: invalid --node-id: {e}"))
            })?;
            let secret_key = read_key_file(quick.key_file.as_deref())?;
            let cfg = iroh_tunnel_host::config::AccessConfig {
                node: iroh_tunnel_host::config::NodeConfig {
                    secret_key,
                    relay_urls: quick.relay_url.into_iter().collect(),
                    ..Default::default()
                },
                services: vec![iroh_tunnel_host::config::AccessService {
                    name,
                    node_id,
                    protocol,
                    host: quick_host(&listen),
                    port: listen.port.get(),
                    relay_urls: vec![],
                    udp: Default::default(),
                }],
            };
            runtime::run_quick_access(
                cfg,
                ctx,
                crate::render::print_ready(),
                lifecycle::wait_for_signal(),
            )
            .await
        }
    }
}

/// `update`: map the CLI options to a request, resolve the running
/// executable, honour its owner, then print the outcome.
async fn run_update_cli(args: &cli::UpdateArgs, exe_name: &str) -> anyhow::Result<()> {
    let request = update::UpdateRequest {
        to: args.to.clone(),
        verification: if args.insecure {
            update::Verification::Insecure
        } else {
            update::Verification::Verified
        },
        overwrite: if args.yes {
            update::OverwritePolicy::ReplacePackageOwned
        } else {
            update::OverwritePolicy::RespectOwner
        },
    };
    let target = update::current_target()?;
    let install_path = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .context("cannot locate the running executable")?;
    let base_url =
        std::env::var(update::BASE_URL_ENV).unwrap_or_else(|_| update::DEFAULT_BASE_URL.to_owned());
    let ctx = update::UpdateContext {
        base_url,
        exe_name,
        target,
        current_version: env!("APP_VERSION"),
        owner: iroh_tunnel_host::update::detect(&install_path),
        install_path,
        verifier: &update::SigstoreVerifier::release(),
    };
    match update::check_owner(&ctx, request.overwrite) {
        update::OwnerDecision::Delegate {
            guidance,
            overridable,
        } => {
            println!("{guidance}");
            if overridable {
                println!("to replace the file anyway, run again with --yes");
            }
            return Ok(());
        }
        update::OwnerDecision::Proceed {
            overriding_package: true,
        } => eprintln!(
            "warning: replacing {} although it is managed by a package manager; \
             the package database will no longer match the file",
            ctx.install_path.display()
        ),
        update::OwnerDecision::Proceed {
            overriding_package: false,
        } => {}
    }
    if request.verification == update::Verification::Insecure {
        eprintln!(
            "warning: --insecure skips signature and checksum verification; \
             you accept responsibility for the installed file"
        );
    }
    let outcome = update::update(&request, &ctx).await?;
    print!("{}", crate::render::render_update(&outcome, exe_name));
    Ok(())
}

async fn dispatch_role_cmd<S: ServiceArgs + Send + 'static>(
    status_role: StatusRole,
    cmd: RoleCmd<S>,
    binary: Binary,
    ctx: RunContext,
) -> anyhow::Result<()> {
    let role = status_role.name();
    match cmd {
        RoleCmd::Run { config } => {
            let path = resolve_config_path(role, config)?;
            config_ops::require_existing(status_role, &path)?;
            // The typed role tag exhaustively selects the run handler —
            // the compiler enforces both arms, no string fallthrough.
            runtime::run_foreground(
                status_role,
                &path,
                ctx,
                crate::render::print_ready(),
                lifecycle::wait_for_signal(),
            )
            .await
        }
        // Host operations are synchronous (and may prompt on stdin), so they
        // run on blocking workers, never on a runtime thread (RS-07).
        RoleCmd::Config { action } => {
            tokio::task::spawn_blocking(move || dispatch_config(status_role, action))
                .await
                .context("config command task failed")?
        }
        RoleCmd::Status { json } => {
            tokio::task::spawn_blocking(move || status_cmd::run(status_role, json))
                .await
                .context("status command task failed")?
        }
        RoleCmd::Update(args) => match binary.is_standalone() {
            true => {
                let exe = format!("iroh-tunnel-{role}");
                run_update_cli(&args, &exe).await
            }
            false => Err(CliError::Config(
                "update acts on the invoked executable; run `iroh-tunnel update`".into(),
            )
            .into()),
        },
        RoleCmd::Service { .. } if binary.is_standalone() => Err(CliError::Service(format!(
            "service management is not yet supported by the standalone {role} binary; \
             use `iroh-tunnel {role} service ...`"
        ))
        .into()),
        RoleCmd::Service { action } => {
            tokio::task::spawn_blocking(move || crate::cli_service::dispatch(status_role, action))
                .await
                .context("service command task failed")?
        }
    }
}

/// Resolve the config path: explicit `--config` wins, otherwise the
/// platform default from [`config_ops::resolve_config_path`].
fn resolve_config_path(role: &str, config: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    config_ops::resolve_config_path(role_named(role)?, config.as_deref())
}

/// The typed role for a CLI role name.
fn role_named(name: &str) -> anyhow::Result<StatusRole> {
    match name {
        "serve" => Ok(StatusRole::Serve),
        "access" => Ok(StatusRole::Access),
        other => anyhow::bail!("unknown role {other:?}"),
    }
}

fn dispatch_config<S: ServiceArgs>(
    role: StatusRole,
    action: ConfigAction<S>,
) -> anyhow::Result<()> {
    match action {
        ConfigAction::Keygen { print: true, .. } => config_cmd::keygen_random(),
        ConfigAction::Keygen { config, yes, .. } => {
            config_cmd::keygen_cli(role, config.as_deref(), yes)
        }
        ConfigAction::Set { service, config } => config_cmd::set(service, config),
        ConfigAction::Rm { config, name } => config_cmd::rm(role, config.as_deref(), &name),
        ConfigAction::List { config } => config_cmd::list(role, config.as_deref()),
        ConfigAction::Show { config, reveal } => config_cmd::show(role, config.as_deref(), reveal),
        ConfigAction::Edit { config } => config_cmd::edit_cli(role, config.as_deref()),
        ConfigAction::Path { config } => config_cmd::path(role, config.as_deref()),
        ConfigAction::Check { config } => config_cmd::check(role, config.as_deref()),
    }
}

/// Read the persistent secret key supplied with `-k/--key-file`.
///
/// Returns an empty string (ephemeral identity) when no file is given. The
/// file holds the base64 key written by `config keygen`. Diagnostics name the
/// path but never echo file contents (RS-10); an unusable key is a config
/// error and nothing is written back (RS-05, RS-06).
fn read_key_file(path: Option<&Path>) -> anyhow::Result<String> {
    let Some(path) = path else {
        return Ok(String::new());
    };
    let text = std::fs::read_to_string(path).map_err(|e| {
        CliError::Config(format!("--key-file {}: cannot read: {e}", path.display()))
    })?;
    let key = text.trim();
    if key.is_empty() {
        return Err(
            CliError::Config(format!("--key-file {}: file is empty", path.display())).into(),
        );
    }
    iroh_tunnel_host::config::resolve_secret_key(key).map_err(|_| {
        CliError::Config(format!(
            "--key-file {}: not a valid base64 32-byte secret key",
            path.display()
        ))
    })?;
    Ok(key.to_owned())
}
