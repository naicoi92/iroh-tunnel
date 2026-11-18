//! CLI handlers for the `config` subcommand: keygen / set / rm / list /
//! show / edit / path / check.
//!
//! Each handler calls the terminal-free host operation in
//! [`iroh_tunnel_host::config`] and does the prompting and printing; the
//! `$EDITOR` invocation of `edit` lives here too.

use std::io::BufRead;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::cli::{NodeArgs, ServiceArgs};
use crate::output;
use crate::render;
use iroh_tunnel_host::config::{
    check_config, identity_state, keygen, list_services, prepare_edit, random_identity,
    remove_service, set_node, set_service, show_config, Disclosure, EditOutcome, IdentityState,
    KeygenOutcome, RotationConsent,
};
use iroh_tunnel_host::error::CliError;
pub use iroh_tunnel_host::Role;

/// CLI wrapper: prompt (or refuse without a terminal) before rotating, then
/// print the result.
///
/// # Errors
///
/// Fails when rotation is refused, the config cannot be updated or stdout fails.
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
    output::stdout(&render::render_keygen(&outcome))
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
    output::stderr(&format!(
        "{} already holds a secret_key. Replacing it changes this node's NodeId and breaks \
         peers that pin it. Replace it? [y/N] ",
        path.display()
    ))?;
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

/// `config keygen --print`: print the identity. The secret goes to stdout only,
/// so it can be captured (`$(…)`, pipes).
///
/// # Errors
///
/// Fails when stdout cannot be written.
pub fn keygen_random() -> Result<()> {
    output::stdout(&render::render_identity(&random_identity()))
}

/// `config set`: create or update the service and print the change report.
///
/// # Errors
///
/// Fails when the request is incomplete, the config cannot be updated or stdout fails.
pub fn set<S: ServiceArgs>(args: S, extra: S::SetExtra, config: Option<PathBuf>) -> Result<()> {
    let outcome = set_service(S::ROLE, &args.into_set(extra, config)?)?;
    output::stdout(&render::render_set(S::ROLE, &outcome))
}

/// Read the relay token from `path` (`-` reads `stdin`), dropping one
/// leading UTF-8 BOM and one trailing `\n` or `\r\n`. Errors name the flag and path, never the content.
fn read_token_source(path: &Path, mut stdin: impl std::io::Read) -> Result<String> {
    let mut text = String::new();
    let read = if path == Path::new("-") {
        stdin.read_to_string(&mut text).map(|_| ())
    } else {
        std::fs::read_to_string(path).map(|t| text = t)
    };
    read.with_context(|| format!("--relay-token-file {}: cannot read", path.display()))
        .map_err(|e| CliError::Config(format!("{e:#}")))?;
    if let Some(t) = text.strip_prefix('\u{feff}') {
        text = t.to_owned();
    }
    if let Some(t) = text.strip_suffix('\n') {
        text = t.strip_suffix('\r').unwrap_or(t).to_owned();
    }
    if text.is_empty() {
        return Err(CliError::Config(format!(
            "--relay-token-file {}: source is empty",
            path.display()
        ))
        .into());
    }
    Ok(text)
}

/// `config node`: edit the `[node]` relay settings and print the report.
///
/// # Errors
///
/// Fails when the token source is unreadable or empty, the edit is invalid,
/// the config cannot be written or stdout fails.
pub fn node(role: Role, mut args: NodeArgs) -> Result<()> {
    // Resolve the token before taking the writer lock: stdin may block.
    let token = match (args.relay_token.take(), args.relay_token_file.take()) {
        (Some(t), _) => Some(t),
        (None, Some(path)) => Some(read_token_source(&path, std::io::stdin().lock())?),
        (None, None) => None,
    };
    let outcome = set_node(role, &args.into_request(token))?;
    output::stdout(&render::render_node(&outcome))
}

/// `config check`: validate the config and print the confirmation.
///
/// # Errors
///
/// Fails when the config is missing or invalid, or stdout fails.
pub fn check(role: Role, config: Option<&Path>) -> Result<()> {
    let path = check_config(role, config)?;
    output::stdout(&render::render_checked(role, &path))
}

/// `config rm`: remove the service and print the confirmation.
///
/// # Errors
///
/// Fails when the service or config is missing, the config cannot be written or stdout fails.
pub fn rm(role: Role, config: Option<&Path>, name: &str) -> Result<()> {
    let outcome = remove_service(role, config, name)?;
    output::stdout(&render::render_removed(&outcome))
}

/// `config list`: print a compact table of the configured services.
///
/// # Errors
///
/// Fails when the config is missing or unreadable, or stdout fails.
pub fn list(role: Role, config: Option<&Path>) -> Result<()> {
    let list = list_services(role, config)?;
    output::stdout(&render::render_service_list(role, &list))
}

/// `config show`: print the config, redacted unless `reveal`.
///
/// # Errors
///
/// Fails when the config is missing or unreadable, or stdout fails.
pub fn show(role: Role, config: Option<&Path>, reveal: bool) -> Result<()> {
    let disclosure = if reveal {
        Disclosure::Reveal
    } else {
        Disclosure::Redacted
    };
    output::stdout(show_config(role, config, disclosure)?.text())
}

/// `config edit` (CLI): open the config in `$EDITOR`, then validate and save it.
///
/// The editor runs without any lock held; see [`prepare_edit`] and
/// [`iroh_tunnel_host::config::EditAttempt::commit`] for the rules applied to the result.
///
/// # Errors
///
/// Fails when the editor fails, the edit is rejected or the config cannot be written.
pub fn edit_cli(role: Role, config: Option<&Path>) -> Result<()> {
    let attempt = prepare_edit(role, config)?;
    edit::edit_file(attempt.scratch_path()).map_err(anyhow::Error::from)?;
    match attempt.commit()? {
        EditOutcome::Unchanged => output::stdout("No changes.\n"),
        EditOutcome::Saved { path } => output::stdout(&format!("Saved {}.\n", path.display())),
        EditOutcome::Rejected { message, .. } => Err(CliError::Config(message).into()),
    }
}

/// `config path`: print the resolved config file path.
///
/// # Errors
///
/// Fails when the path cannot be resolved or stdout fails.
pub fn path(role: Role, config: Option<&Path>) -> Result<()> {
    let path = iroh_tunnel_host::config::resolve_config_path(role, config)?;
    output::stdout(&render::render_path(&path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn from_file(content: &str) -> String {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("token.txt");
        std::fs::write(&path, content).expect("write token file");
        match read_token_source(&path, Cursor::new(Vec::new())) {
            Ok(t) => t,
            Err(e) => format!("ERR {e}"),
        }
    }

    #[test]
    fn token_source_strips_utf8_bom() {
        assert_eq!(from_file("\u{feff}secret-token\r\n"), "secret-token");
        assert_eq!(from_file("\u{feff}\u{feff}x"), "\u{feff}x");
    }

    #[test]
    fn token_source_trims_one_newline() {
        assert_eq!(from_file("secret-token\n"), "secret-token");
    }

    #[test]
    fn token_source_trims_crlf() {
        assert_eq!(from_file("secret-token\r\n"), "secret-token");
    }

    #[test]
    fn token_source_keeps_extra_newline() {
        assert_eq!(from_file("secret-token\n\n"), "secret-token\n");
    }

    #[test]
    fn token_source_reads_stdin_for_dash() {
        let got = read_token_source(Path::new("-"), Cursor::new(b"from-stdin".to_vec()));
        assert_eq!(got.unwrap(), "from-stdin");
    }

    #[test]
    fn token_source_empty_errors() {
        let err = from_file("");
        assert!(err.starts_with("ERR"), "{err}");
        assert!(err.contains("--relay-token-file"), "{err}");
        assert!(err.contains("token.txt"), "{err}");
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn token_source_unreadable_names_flag_and_path() {
        let err = read_token_source(Path::new("/nonexistent/t.txt"), Cursor::new(Vec::new()))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("--relay-token-file /nonexistent/t.txt: cannot read"),
            "{err}"
        );
    }
}
