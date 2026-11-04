//! CLI handlers for the `config` subcommand: keygen / add / remove / list /
//! show / edit / path.
//!
//! Each handler calls the terminal-free host operation in
//! [`iroh_tunnel_host::config_ops`] and does the prompting and printing; the
//! `$EDITOR` invocation of `edit` lives here too.

use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result};

use crate::cli::AddServiceArgs;
use crate::render;
use iroh_tunnel_host::config::{
    add_service, identity_state, keygen, list_services, prepare_edit, random_identity,
    remove_service, show_config, AddService, Disclosure, EditOutcome, IdentityState, KeygenOutcome,
    RotationConsent,
};
use iroh_tunnel_host::error::CliError;
pub use iroh_tunnel_host::Role;

/// Request for `config add` built from the parsed CLI arguments.
fn add_request(args: &AddServiceArgs) -> AddService {
    AddService {
        config: args.config.clone(),
        name: args.name.clone(),
        protocol: args.protocol.clone(),
        host: args.host.clone(),
        port: args.port,
        node_id: args.node_id.clone(),
    }
}

/// CLI wrapper: prompt (or refuse without a terminal) before rotating, then
/// print the result.
pub fn keygen_cli(role: Role, config: Option<&Path>, yes: bool) -> Result<()> {
    let mut consent = if yes {
        RotationConsent::Given
    } else {
        RotationConsent::NotGiven
    };
    // Ask before taking the lock so a slow answer does not stall other
    // writers; the decision is re-checked under the lock in `keygen`.
    if !yes && identity_state(role, config)? == IdentityState::HasKey {
        let path = iroh_tunnel_host::config::resolve_write_target(
            &iroh_tunnel_host::config::resolve_config_path(role, config)?,
        );
        confirm_rotation(&path)?;
        consent = RotationConsent::Given;
    }
    let outcome = keygen(role, config, consent)?;
    if let KeygenOutcome::ConsentNeeded { path, .. } = &outcome {
        return Err(rotation_refused(path));
    }
    print!("{}", render::render_keygen(&outcome));
    Ok(())
}

fn rotation_refused(path: &Path) -> anyhow::Error {
    CliError::Config(format!(
        "{} already holds a secret_key; replacing it changes this node's NodeId. \
         Re-run with --yes to rotate it",
        path.display()
    ))
    .into()
}

/// Ask the user to confirm replacing the key in `path`.
///
/// # Errors
///
/// [`CliError::Config`] when no terminal is available or the answer is not
/// yes; the key is left unchanged either way.
fn confirm_rotation(path: &Path) -> Result<()> {
    if !crate::cli_service::is_interactive() {
        return Err(rotation_refused(path));
    }
    eprint!(
        "{} already holds a secret_key. Replacing it changes this node's NodeId and breaks \
         peers that pin it. Replace it? [y/N] ",
        path.display()
    );
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .context("failed to read the confirmation")?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(CliError::Config("key rotation cancelled; the secret_key is unchanged".into()).into())
    }
}

/// `config keygen random`: print the identity. The secret goes to stdout only,
/// so it can be captured (`$(…)`, pipes).
pub fn keygen_random() -> Result<()> {
    print!("{}", render::render_identity(&random_identity()));
    Ok(())
}

/// `config add`: add the service and print the confirmation.
pub fn add(role: Role, args: &AddServiceArgs) -> Result<()> {
    let outcome = add_service(role, &add_request(args))?;
    print!("{}", render::render_added(&outcome));
    Ok(())
}

/// `config remove`: remove the service and print the confirmation.
pub fn remove(role: Role, config: Option<&Path>, name: &str) -> Result<()> {
    let outcome = remove_service(role, config, name)?;
    print!("{}", render::render_removed(&outcome));
    Ok(())
}

/// `config list`: print a compact table of the configured services.
pub fn list(role: Role, config: Option<&Path>) -> Result<()> {
    let list = list_services(role, config)?;
    print!("{}", render::render_service_list(role, &list));
    Ok(())
}

/// `config show`: print the config, redacted unless `reveal`.
pub fn show(role: Role, config: Option<&Path>, reveal: bool) -> Result<()> {
    let disclosure = if reveal {
        Disclosure::Reveal
    } else {
        Disclosure::Redacted
    };
    print!("{}", show_config(role, config, disclosure)?.text());
    Ok(())
}

/// `config edit` (CLI): open the config in `$EDITOR`, then validate and save it.
///
/// The editor runs without any lock held; see [`prepare_edit`] and
/// [`EditAttempt::commit`] for the rules applied to the result.
pub fn edit_cli(role: Role, config: Option<&Path>) -> Result<()> {
    let attempt = prepare_edit(role, config)?;
    edit::edit_file(attempt.scratch_path()).map_err(anyhow::Error::from)?;
    match attempt.commit()? {
        EditOutcome::Unchanged => {
            println!("No changes.");
            Ok(())
        }
        EditOutcome::Saved { path } => {
            println!("Saved {}.", path.display());
            Ok(())
        }
        EditOutcome::Rejected { message, .. } => Err(CliError::Config(message).into()),
    }
}

/// `config path`: print the resolved config file path.
pub fn path(role: Role, config: Option<&Path>) -> Result<()> {
    let path = iroh_tunnel_host::config::resolve_config_path(role, config)?;
    print!("{}", render::render_path(&path));
    Ok(())
}
