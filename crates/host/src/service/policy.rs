//! Platform-support and privilege checks shared by the operations (private).

use anyhow::Result;

use super::types::ServiceScope;
use crate::error::CliError;

/// Refuse a scope this platform has no backend for.
///
/// Every supported platform (Linux, macOS, Windows) has both scopes, so this
/// accepts them all; the per-scope privilege and trust checks come later.
///
/// # Errors
///
/// Never fails today; kept as the single seam for a platform without a backend.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn ensure_supported(_scope: ServiceScope) -> Result<()> {
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
    Err(CliError::Permission(privilege_text(cfg!(windows), rerun)).into())
}

/// Guidance text for a refused system-scope operation (pure; platform passed in).
fn privilege_text(windows: bool, argv: &[String]) -> String {
    if windows {
        let command = argv
            .iter()
            .map(|arg| windows_quote(arg))
            .collect::<Vec<_>>()
            .join(" ");
        return format!(
            "a system service needs Administrator, and nothing was changed (no automatic \
             elevation, no fallback to a user service). Run as administrator: open an elevated \
             terminal and run `{command}`, or use --user."
        );
    }
    let command = argv
        .iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "a system service needs root/Administrator, and nothing was changed (no automatic \
         elevation, no fallback to a user service). Re-run as an elevated user: \
         `sudo {command}` (an elevated terminal on Windows), or use --user."
    )
}

/// Quote `arg` for a Windows command line: wrap in `"` when it holds a space
/// or `"`, doubling inner quotes.
fn windows_quote(arg: &str) -> String {
    if arg.contains([' ', '"']) {
        format!("\"{}\"", arg.replace('"', "\"\""))
    } else {
        arg.to_owned()
    }
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
        let expected = if cfg!(windows) {
            r#"Run as administrator: open an elevated terminal and run `iroh-tunnel serve service install --system --config "/a b/it's.toml"`"#
        } else {
            r"sudo iroh-tunnel serve service install --system --config '/a b/it'\''s.toml'"
        };
        assert!(msg.contains(expected), "{msg}");
    }

    #[test]
    fn privilege_is_not_required_for_user_scope_or_when_elevated() {
        assert!(ensure_privilege(ServiceScope::User, false, &[]).is_ok());
        assert!(ensure_privilege(ServiceScope::System, true, &[]).is_ok());
    }

    fn argv() -> Vec<String> {
        [
            "iroh-tunnel",
            "serve",
            "service",
            "install",
            "--system",
            "--config",
            "C:\\Program Files\\x.toml",
        ]
        .map(str::to_owned)
        .to_vec()
    }

    #[test]
    fn windows_quote_wraps_spaces_and_doubles_quotes() {
        assert_eq!(
            windows_quote("C:\\Program Files\\x"),
            "\"C:\\Program Files\\x\""
        );
        assert_eq!(windows_quote("a\"b"), "\"a\"\"b\"");
        assert_eq!(windows_quote("plain"), "plain");
    }

    #[test]
    fn windows_privilege_text_names_run_as_administrator() {
        let msg = privilege_text(true, &argv());
        assert!(msg.contains("Run as administrator"), "{msg}");
        assert!(
            msg.contains("`iroh-tunnel serve service install --system --config \"C:\\Program Files\\x.toml\"`"),
            "{msg}"
        );
        assert!(!msg.contains("sudo"), "{msg}");
    }

    #[test]
    fn unix_privilege_text_is_unchanged() {
        let rerun = ["iroh-tunnel", "install", "/a b"]
            .map(str::to_owned)
            .to_vec();
        assert_eq!(
            privilege_text(false, &rerun),
            "a system service needs root/Administrator, and nothing was changed (no automatic \
             elevation, no fallback to a user service). Re-run as an elevated user: \
             `sudo iroh-tunnel install '/a b'` (an elevated terminal on Windows), or use --user."
        );
    }
}
