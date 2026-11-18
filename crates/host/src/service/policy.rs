//! Privilege checks and shell quoting shared by the operations (private).

use anyhow::Result;

use crate::error::CliError;

/// Refuse a root-run registration without privilege. Covers BusyBox hosts,
/// whose only registration is the system-wide `/etc/init.d` script: a
/// non-root caller is never silently given a system install.
///
/// # Errors
///
/// [`CliError::Permission`] with rerun guidance; nothing has been changed.
pub(crate) fn ensure_root_run_privilege(root_run: bool, elevated: bool) -> Result<()> {
    if !root_run || elevated {
        return Ok(());
    }
    Err(CliError::Permission(
        "this host runs BusyBox init, which has no per-user services; managing the \
         /etc/init.d service requires root. Nothing was changed. Re-run with sudo."
            .into(),
    )
    .into())
}

/// Quote `arg` for a POSIX shell only when it needs it, for commands shown
/// in guidance and written into init scripts.
#[must_use]
pub fn shell_quote(arg: &str) -> String {
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
    fn root_run_without_privilege_is_a_permission_error_with_sudo_guidance() {
        let err = ensure_root_run_privilege(true, false).unwrap_err();
        let Some(CliError::Permission(msg)) = err.downcast_ref::<CliError>() else {
            panic!("expected Permission, got {err:?}");
        };
        assert!(msg.contains("Re-run with sudo"), "{msg}");
        assert!(msg.contains("Nothing was changed"), "{msg}");
    }

    #[test]
    fn root_run_privilege_is_not_required_when_elevated_or_not_root_run() {
        assert!(ensure_root_run_privilege(true, true).is_ok());
        assert!(ensure_root_run_privilege(false, false).is_ok());
        assert!(ensure_root_run_privilege(false, true).is_ok());
    }

    #[test]
    fn shell_quote_leaves_plain_words_and_quotes_the_rest() {
        assert_eq!(shell_quote("/usr/bin/iroh-tunnel"), "/usr/bin/iroh-tunnel");
        assert_eq!(shell_quote("/a b/it's"), r"'/a b/it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }
}
