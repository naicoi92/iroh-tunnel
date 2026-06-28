//! Entry point: parse CLI, init tracing, dispatch to role handlers.
//!
//! Dispatch wires up the role run handlers (T-06 serve, T-07 access); config
//! (T-11) and service (T-12) management are still placeholders.
//! Exit-code mapping follows Page 06 v5 §6 (see [`error::CliError`]).

mod access;
mod cli;
mod config;
mod config_cmd;
mod endpoint;
mod error;
mod pipe;
mod proto;
mod serve;
mod service;
mod shutdown;

use std::path::PathBuf;

use clap::Parser;
use cli::{Cli, ConfigAction, Role, RoleCmd, ServiceAction};
use error::CliError;

fn main() {
    let cli = Cli::parse();
    cli::init_tracing(cli.verbose, cli.quiet);

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

    match rt.block_on(run(cli)) {
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
    let role_str = match &cli.role {
        Role::Serve { .. } => "serve",
        Role::Access { .. } => "access",
    };
    match cli.role {
        Role::Serve { cmd } | Role::Access { cmd } => dispatch_role_cmd(role_str, cmd).await,
    }
}

async fn dispatch_role_cmd(role: &str, cmd: RoleCmd) -> anyhow::Result<()> {
    match cmd {
        RoleCmd::Run { config } => {
            let path = resolve_config_path(role, config)?;
            match role {
                "serve" => serve::run(&path).await,
                "access" => access::run(&path).await,
                _ => unreachable!("unknown role {role}"),
            }
        }
        RoleCmd::Config { action } => dispatch_config(role, action),
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
    eprintln!("not implemented yet: {role} service {action:?}");
    Ok(())
}
