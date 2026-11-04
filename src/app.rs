//! Shared entry points for the three role binaries (`iroh-tunnel`,
//! `iroh-tunnel-serve`, `iroh-tunnel-access`): parse, init tracing, build the
//! runtime, dispatch and map errors to exit codes (see [`CliError`], RS-06).
//!
//! The binaries are thin wrappers so each one is a self-contained executable
//! that never needs another binary at runtime.

use std::path::{Path, PathBuf};

use crate::cli;
use crate::cli::{AccessCli, Cli, ConfigAction, KeygenMode, Role, RoleCmd, ServeCli};
use crate::config_cmd;
use crate::host::config_ops;
use crate::host::error::CliError;
use crate::host::runtime;
use crate::host::status::Role as StatusRole;
use crate::host::update;
use crate::host::RunContext;
use crate::lifecycle;
use crate::status_cmd;
use anyhow::Context as _;
use clap::Parser;

/// Entry point of the all-in-one `iroh-tunnel` (`it`) binary.
pub fn main_all_in_one() -> ! {
    install_migration_notice();
    execute(Cli::parse(), Packaging::AllInOne)
}

/// Entry point of the standalone `iroh-tunnel-serve` (`its`) binary.
pub fn main_serve() -> ! {
    install_migration_notice();
    execute(ServeCli::parse().into_cli(), Packaging::Standalone)
}

/// Entry point of the standalone `iroh-tunnel-access` (`ita`) binary.
pub fn main_access() -> ! {
    install_migration_notice();
    execute(AccessCli::parse().into_cli(), Packaging::Standalone)
}

/// Show the operator the macOS legacy-configuration copy on stderr; the host
/// only reports it through this observer. Runs before any path resolution.
fn install_migration_notice() {
    crate::host::paths::set_migration_notice(|legacy, dest| {
        eprintln!(
            "migrated configuration {} -> {} (original kept)",
            legacy.display(),
            dest.display()
        );
    });
}

/// Which executable is running. Service units run `<exe> <role> run`, which
/// only the all-in-one binary accepts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Packaging {
    AllInOne,
    Standalone,
}

fn execute(parsed: Cli, packaging: Packaging) -> ! {
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
            result = run(parsed, packaging, ctx.clone()) => Ok(result),
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
            let code = match e.downcast_ref::<CliError>() {
                Some(CliError::Config(_)) => 2,
                Some(CliError::Permission(_)) => 3,
                Some(CliError::Iroh(_)) => 4,
                Some(CliError::Service(_)) => 5,
                Some(CliError::Conflict(_)) => 6,
                None => 1,
            };
            std::process::exit(code);
        }
    }
}

async fn run(cli: Cli, packaging: Packaging, ctx: RunContext) -> anyhow::Result<()> {
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
        Role::Serve { cmd: Some(cmd), .. } | Role::Access { cmd: Some(cmd), .. } => {
            dispatch_role_cmd(status_role, cmd, packaging, ctx).await
        }
        Role::Serve { quick, cmd: None } => {
            let protocol = quick_protocol("serve", &quick.protocol)?;
            let target = quick.target.ok_or_else(|| {
                CliError::Config("serve quick mode: --target IP:PORT is required".into())
            })?;
            let secret_key = read_key_file(quick.key_file.as_deref())?;
            let cfg = crate::host::config::ServeConfig {
                node: crate::host::config::NodeConfig {
                    secret_key,
                    relay_urls: quick.relay_url.into_iter().collect(),
                    ..Default::default()
                },
                services: vec![crate::host::config::ServeService {
                    name: quick.name.ok_or_else(|| {
                        CliError::Config("serve quick mode: --name is required".into())
                    })?,
                    protocol,
                    host: target.ip().to_string(),
                    port: target.port(),
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
            let protocol = quick_protocol("access", &quick.protocol)?;
            let listen = quick.listen.ok_or_else(|| {
                CliError::Config("access quick mode: --listen IP:PORT is required".into())
            })?;
            let node_id = quick.node_id.ok_or_else(|| {
                CliError::Config("access quick mode: --node-id is required".into())
            })?;
            node_id.parse::<iroh::EndpointId>().map_err(|e| {
                CliError::Config(format!("access quick mode: invalid --node-id: {e}"))
            })?;
            let secret_key = read_key_file(quick.key_file.as_deref())?;
            let cfg = crate::host::config::AccessConfig {
                node: crate::host::config::NodeConfig {
                    secret_key,
                    relay_urls: quick.relay_url.into_iter().collect(),
                    ..Default::default()
                },
                services: vec![crate::host::config::AccessService {
                    name: quick.name.ok_or_else(|| {
                        CliError::Config("access quick mode: --name is required".into())
                    })?,
                    node_id,
                    protocol,
                    host: listen.ip().to_string(),
                    port: listen.port(),
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
        owner: crate::host::update_owner::detect(&install_path),
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

/// Parse a quick-mode `--protocol` value.
fn quick_protocol(role: &str, value: &str) -> Result<crate::host::config::Protocol, CliError> {
    match value {
        "tcp" => Ok(crate::host::config::Protocol::Tcp),
        "udp" => Ok(crate::host::config::Protocol::Udp),
        other => Err(CliError::Config(format!(
            "{role} quick mode supports tcp or udp protocol, got '{other}'; use --protocol tcp or --protocol udp"
        ))),
    }
}

async fn dispatch_role_cmd(
    status_role: StatusRole,
    cmd: RoleCmd,
    packaging: Packaging,
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
        RoleCmd::Update(args) => match packaging {
            Packaging::Standalone => {
                let exe = format!("iroh-tunnel-{role}");
                run_update_cli(&args, &exe).await
            }
            Packaging::AllInOne => Err(CliError::Config(
                "update acts on the invoked executable; run `iroh-tunnel update`".into(),
            )
            .into()),
        },
        RoleCmd::Service { .. } if packaging == Packaging::Standalone => {
            Err(CliError::Service(format!(
                "service management is not yet supported by the standalone {role} binary; \
             use `iroh-tunnel {role} service ...`"
            ))
            .into())
        }
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

fn dispatch_config(role: StatusRole, action: ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::Keygen {
            mode: Some(KeygenMode::Random),
            ..
        } => config_cmd::keygen_random(),
        ConfigAction::Keygen {
            config,
            yes,
            mode: None,
        } => config_cmd::keygen_cli(role, config.as_deref(), yes),
        ConfigAction::Add(args) => config_cmd::add(role, &args),
        ConfigAction::Remove { config, name } => config_cmd::remove(role, config.as_deref(), &name),
        ConfigAction::List { config } => config_cmd::list(role, config.as_deref()),
        ConfigAction::Show { config, reveal } => config_cmd::show(role, config.as_deref(), reveal),
        ConfigAction::Edit { config } => config_cmd::edit_cli(role, config.as_deref()),
        ConfigAction::Path { config } => config_cmd::path(role, config.as_deref()),
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
    crate::host::config::resolve_secret_key(key).map_err(|_| {
        CliError::Config(format!(
            "--key-file {}: not a valid base64 32-byte secret key",
            path.display()
        ))
    })?;
    Ok(key.to_owned())
}
