//! Platform-support and privilege checks shared by the operations (private).

use anyhow::Result;

use super::types::ServiceScope;
use crate::error::CliError;

/// Refuse a scope this platform has no backend for.
///
/// Windows has only the user-background task: a Windows Service is deferred
/// (SM-04), so `--system` is reported as unsupported rather than as a missing
/// privilege that elevation could not fix.
///
/// # Errors
///
/// [`CliError::Service`] for system scope on Windows; nothing is changed.
pub(crate) fn ensure_supported(scope: ServiceScope) -> Result<()> {
    if cfg!(windows) && scope == ServiceScope::System {
        return Err(CliError::Service(
            "system services are not supported on Windows; use --user, which runs the role \
             in your logged-in session as a scheduled task"
                .into(),
        )
        .into());
    }
    Ok(())
}

/// Refuse a system-scope operation without privilege.
///
/// `rerun` holds the argv (after `sudo`) that repeats the intended operation,
/// with absolute paths already resolved. Nothing is changed.
///
/// # Errors
///
/// [`CliError::Permission`] with rerun guidance.
pub(crate) fn ensure_privilege(
    scope: ServiceScope,
    elevated: bool,
    rerun: &[String],
) -> Result<()> {
    if scope == ServiceScope::User || elevated {
        return Ok(());
    }
    let command = rerun
        .iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    Err(CliError::Permission(format!(
        "a system service needs root/Administrator, and nothing was changed (no automatic \
         elevation, no fallback to a user service). Re-run as an elevated user: \
         `sudo {command}` (an elevated terminal on Windows), or use --user."
    ))
    .into())
}

/// Quote `arg` for a POSIX shell only when it needs it.
fn shell_quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if plain {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_scope_without_privilege_is_a_permission_error_with_the_rerun_command() {
        let rerun: Vec<String> = [
            "iroh-tunnel",
            "serve",
            "service",
            "install",
            "--system",
            "--config",
            "/a b/it's.toml",
        ]
        .map(str::to_owned)
        .to_vec();
        let err = ensure_privilege(ServiceScope::System, false, &rerun).unwrap_err();
        let Some(CliError::Permission(msg)) = err.downcast_ref::<CliError>() else {
            panic!("expected Permission, got {err:?}");
        };
        assert!(
            msg.contains(
                r"sudo iroh-tunnel serve service install --system --config '/a b/it'\''s.toml'"
            ),
            "{msg}"
        );
    }

    #[test]
    fn privilege_is_not_required_for_user_scope_or_when_elevated() {
        assert!(ensure_privilege(ServiceScope::User, false, &[]).is_ok());
        assert!(ensure_privilege(ServiceScope::System, true, &[]).is_ok());
    }
}
