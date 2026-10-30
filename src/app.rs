//! Shared entry points for the three role binaries (`iroh-tunnel`,
//! `iroh-tunnel-serve`, `iroh-tunnel-access`): parse, init tracing, build the
//! runtime, dispatch and map errors to exit codes (see [`CliError`], RS-06).
//!
//! The binaries are thin wrappers so each one is a self-contained executable
//! that never needs another binary at runtime.

use std::path::{Path, PathBuf};

use crate::cli::{AccessCli, Cli, ConfigAction, Role, RoleCmd, ServeCli, ServiceAction};
use crate::config_cmd;
use crate::error::CliError;
use crate::status::StatusRole;
use crate::status_cmd;
use crate::{access, cli, serve, service};
use anyhow::Context as _;
use clap::Parser;

/// Entry point of the all-in-one `iroh-tunnel` (`it`) binary.
pub fn main_all_in_one() -> ! {
    let parsed = Cli::parse();
    execute(parsed)
}

/// Entry point of the standalone `iroh-tunnel-serve` (`its`) binary.
pub fn main_serve() -> ! {
    execute(ServeCli::parse().into_cli())
}

/// Entry point of the standalone `iroh-tunnel-access` (`ita`) binary.
pub fn main_access() -> ! {
    execute(AccessCli::parse().into_cli())
}

fn execute(parsed: Cli) -> ! {
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

    match rt.block_on(run(parsed)) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("error: {e:#}");
            let code = match e.downcast_ref::<CliError>() {
                Some(CliError::Config(_)) => 2,
                Some(CliError::Permission(_)) => 3,
                Some(CliError::Iroh(_)) => 4,
                Some(CliError::Service(_)) => 5,
                None => 1,
            };
            std::process::exit(code);
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    // The typed role tag comes from ONE match on the CLI enum; the dispatch
    // string is derived from it — an invalid role is unrepresentable, and
    // the two can never disagree.
    let status_role = match &cli.role {
        Role::Serve { .. } => StatusRole::Serve,
        Role::Access { .. } => StatusRole::Access,
    };
    match cli.role {
        Role::Serve { cmd: Some(cmd), .. } | Role::Access { cmd: Some(cmd), .. } => {
            dispatch_role_cmd(status_role, cmd).await
        }
        Role::Serve { quick, cmd: None } => {
            if quick.protocol != "tcp" {
                return Err(CliError::Config(
                    "serve quick mode supports only tcp protocol; use --protocol tcp".into(),
                )
                .into());
            }
            let target = quick.target.ok_or_else(|| {
                CliError::Config("serve quick mode: --target IP:PORT is required".into())
            })?;
            let secret_key = read_key_file(quick.key_file.as_deref())?;
            let cfg = crate::config::ServeConfig {
                node: crate::config::NodeConfig {
                    secret_key,
                    relay_urls: quick.relay_url.into_iter().collect(),
                    ..Default::default()
                },
                services: vec![crate::config::ServeService {
                    name: quick.name.ok_or_else(|| {
                        CliError::Config("serve quick mode: --name is required".into())
                    })?,
                    protocol: crate::config::Protocol::Tcp,
                    host: target.ip().to_string(),
                    port: target.port(),
                }],
            };
            serve::run_quick(cfg).await
        }
        Role::Access { quick, cmd: None } => {
            if quick.protocol != "tcp" {
                return Err(CliError::Config(
                    "access quick mode supports only tcp protocol; use --protocol tcp".into(),
                )
                .into());
            }
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
            let cfg = crate::config::AccessConfig {
                node: crate::config::NodeConfig {
                    secret_key,
                    relay_urls: quick.relay_url.into_iter().collect(),
                    ..Default::default()
                },
                services: vec![crate::config::AccessService {
                    name: quick.name.ok_or_else(|| {
                        CliError::Config("access quick mode: --name is required".into())
                    })?,
                    node_id,
                    protocol: crate::config::Protocol::Tcp,
                    host: listen.ip().to_string(),
                    port: listen.port(),
                    relay_urls: vec![],
                }],
            };
            access::run_quick(cfg).await
        }
    }
}

async fn dispatch_role_cmd(status_role: StatusRole, cmd: RoleCmd) -> anyhow::Result<()> {
    let role = status_role.name();
    match cmd {
        RoleCmd::Run { config } => {
            let path = resolve_config_path(role, config)?;
            // The typed role tag exhaustively selects the run handler —
            // the compiler enforces both arms, no string fallthrough.
            match status_role {
                StatusRole::Serve => serve::run(&path).await,
                StatusRole::Access => access::run(&path).await,
            }
        }
        RoleCmd::Config { action } => dispatch_config(role, action),
        RoleCmd::Status { json } => status_cmd::run(status_role, json),
        RoleCmd::Service { action } => dispatch_service(role, action),
    }
}

/// Resolve the config path: explicit `--config` wins, otherwise the
/// OS-specific default (`~/.config/iroh-tunnel/{role}.toml` on Linux,
/// `~/Library/Application Support/iroh-tunnel/{role}.toml` on macOS).
fn resolve_config_path(role: &str, config: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    config_cmd::resolve_config_path(role, config.as_deref())
}

/// Make `path` absolute for embedding in a system service file.
///
/// Installed services run with a different working directory, so a relative
/// `--config` path must be resolved against the current CWD before it is
/// written into the unit/plist. Falls back to joining the CWD if the path
/// can't be canonicalized (e.g. it doesn't exist yet).
fn absolutize_config_path(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    if let Ok(abs) = std::fs::canonicalize(path) {
        return Ok(abs);
    }
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    Ok(cwd.join(path))
}

fn dispatch_config(role: &str, action: ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::Keygen { config } => config_cmd::keygen(role, config.as_deref()),
        ConfigAction::Add(args) => config_cmd::add(role, &args),
        ConfigAction::Remove { name } => config_cmd::remove(role, None, &name),
        ConfigAction::List => config_cmd::list(role, None),
        ConfigAction::Show => config_cmd::show(role, None),
        ConfigAction::Edit => config_cmd::edit(role, None),
        ConfigAction::Path => config_cmd::path(role, None),
    }
}

fn dispatch_service(role: &str, action: ServiceAction) -> anyhow::Result<()> {
    use service::ServiceScope;
    // Default scope is per-user (LaunchAgent / `systemctl --user`): no
    // privileges needed, and it matches how iroh-tunnel is normally used on a
    // desktop. `--system` opts into a system-wide daemon (root, for servers).
    let scope_of = |system: bool| {
        if system {
            ServiceScope::System
        } else {
            ServiceScope::User
        }
    };
    match action {
        ServiceAction::Install { config, system } => {
            let path = resolve_config_path(role, config)?;
            // Persist an absolute path: the installed service runs with a
            // different CWD, so a relative --config path would break it.
            let path = absolutize_config_path(&path)?;
            service::install(role, scope_of(system), &path)
        }
        ServiceAction::Uninstall { system } => service::uninstall(role, scope_of(system)),
        ServiceAction::Start { system } => service::start(role, scope_of(system)),
        ServiceAction::Stop { system } => service::stop(role, scope_of(system)),
        ServiceAction::Restart { system } => service::restart(role, scope_of(system)),
        ServiceAction::Status { system } => service::status(role, scope_of(system)),
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
    crate::config::resolve_secret_key(key).map_err(|_| {
        CliError::Config(format!(
            "--key-file {}: not a valid base64 32-byte secret key",
            path.display()
        ))
    })?;
    Ok(key.to_owned())
}
