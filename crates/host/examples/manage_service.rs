//! Test-only non-terminal caller of the host service API.
//!
//! `manage_service --role serve|access --action install|status|uninstall
//! [--config <abs>] [--exe <abs iroh-tunnel>] [--scope user|system]`
//!
//! It captures native output and prints the `Debug` of the outcome to stdout,
//! so a test can show the host API works without a terminal. Stderr stays empty
//! on success. Exit code: 0 on success (including an install that needs a
//! decision), 1 on error with the message on stderr.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use iroh_tunnel_host::role::Role;
use iroh_tunnel_host::service::{
    self, Environment, InstallRequest, ServiceCall, ServiceNote, ServiceScope,
};

struct Args {
    role: Role,
    scope: ServiceScope,
    action: String,
    config: Option<PathBuf>,
    exe: Option<PathBuf>,
}

fn parse_args() -> Result<Args> {
    let mut role = None;
    let mut scope = ServiceScope::User;
    let mut action = None;
    let mut config = None;
    let mut exe = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--role" => {
                role = Some(match value()?.as_str() {
                    "serve" => Role::Serve,
                    "access" => Role::Access,
                    other => bail!("unknown role {other:?}"),
                });
            }
            "--scope" => {
                scope = match value()?.as_str() {
                    "user" => ServiceScope::User,
                    "system" => ServiceScope::System,
                    other => bail!("unknown scope {other:?}"),
                };
            }
            "--action" => action = Some(value()?),
            "--config" => config = Some(PathBuf::from(value()?)),
            "--exe" => exe = Some(PathBuf::from(value()?)),
            other => bail!("unknown argument {other:?}"),
        }
    }
    Ok(Args {
        role: role.context("--role is required")?,
        scope,
        action: action.context("--action is required")?,
        config,
        exe,
    })
}

fn run() -> Result<String> {
    let args = parse_args()?;
    let env = Environment {
        elevated: service::is_elevated(),
        can_answer: false,
    };
    let mut notes = |_: &ServiceNote| {};
    let mut call = ServiceCall::capture(&mut notes);
    match args.action.as_str() {
        "install" => {
            let request = InstallRequest {
                role: args.role,
                scope: Some(args.scope),
                config: args.config,
                tunnel_executable: args.exe.context("--exe is required for install")?,
                sudo_user: None,
                invoking_user: None,
                state_dir_override: std::env::var_os("IROH_TUNNEL_STATE_DIR")
                    .filter(|dir| !dir.is_empty())
                    .map(PathBuf::from),
                retarget: None,
            };
            Ok(format!(
                "{:#?}",
                service::install(&request, env, &mut call)?
            ))
        }
        "status" => Ok(format!(
            "{:#?}",
            service::status(args.role, args.scope, env, &mut call)?
        )),
        "uninstall" => Ok(format!(
            "{:#?}",
            service::uninstall(args.role, args.scope, env, &mut call)?
        )),
        other => bail!("unknown action {other:?}"),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(outcome) => {
            println!("{outcome}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::FAILURE
        }
    }
}
