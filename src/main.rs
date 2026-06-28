//! Entry point: parse CLI, init tracing, dispatch to role handlers.
//!
//! Handlers are placeholders for now (T-01); real logic lands in T-02..T-12.
//! Exit-code mapping follows Page 06 v5 §6 (see [`error::CliError`]).

mod access;
mod cli;
mod config_cmd;
mod error;
mod serve;
mod service;

use clap::Parser;
use cli::{Cli, ConfigAction, Role, RoleCmd, ServiceAction};
use error::CliError;

fn main() {
    let cli = Cli::parse();
    cli::init_tracing(cli.verbose, cli.quiet);

    match run(cli) {
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

fn run(cli: Cli) -> anyhow::Result<()> {
    let role_str = match &cli.role {
        Role::Serve { .. } => "serve",
        Role::Access { .. } => "access",
    };
    match cli.role {
        Role::Serve { cmd } | Role::Access { cmd } => dispatch_role_cmd(role_str, cmd),
    }
}

fn dispatch_role_cmd(role: &str, cmd: RoleCmd) -> anyhow::Result<()> {
    match cmd {
        RoleCmd::Run { config } => {
            eprintln!("not implemented yet: {role} run (config={config:?})");
            Ok(())
        }
        RoleCmd::Config { action } => dispatch_config(role, action),
        RoleCmd::Service { action } => dispatch_service(role, action),
    }
}

fn dispatch_config(role: &str, action: ConfigAction) -> anyhow::Result<()> {
    eprintln!("not implemented yet: {role} config {action:?}");
    Ok(())
}

fn dispatch_service(role: &str, action: ServiceAction) -> anyhow::Result<()> {
    eprintln!("not implemented yet: {role} service {action:?}");
    Ok(())
}
