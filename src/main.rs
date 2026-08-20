//! Entry point: parse CLI, init tracing, dispatch to role handlers.
//!
//! Dispatch wires up the role run handlers (T-06 serve, T-07 access); config
//! (T-11) and service (T-12) management are still placeholders.
//! Exit-code mapping follows Page 06 v5 §6 (see [`error::CliError`]).
//!
//! This is a thin binary wrapper over the [`iroh_tunnel`] library crate; all
//! real logic lives there so integration tests can exercise it in-process.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::Parser;
use iroh_tunnel::cli::{self, Cli, ConfigAction, Role, RoleCmd, ServiceAction};
use iroh_tunnel::config_cmd;
use iroh_tunnel::error::CliError;
use iroh_tunnel::status::StatusRole;
use iroh_tunnel::status_cmd;
use iroh_tunnel::{access, serve, service};

fn main() {
    let parsed = Cli::parse();
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
        Role::Serve { cmd } | Role::Access { cmd } => dispatch_role_cmd(status_role, cmd).await,
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
    if let Some(p) = config {
        return Ok(p);
    }
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?;
    Ok(dir.join("iroh-tunnel").join(format!("{role}.toml")))
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
