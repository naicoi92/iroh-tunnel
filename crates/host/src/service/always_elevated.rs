//! Whether a Windows account's logon task would run with an elevated token.
//!
//! A task registered with `InteractiveToken` + `LeastPrivilege` gets the full
//! token when the account has no filtered one: the built-in Administrator, or
//! an Administrators member on a host with UAC disabled (`EnableLUA` = 0).
//! Such a task process resolves the system tier, so the user tier refuses it
//! (D7b). The decision is pure; the Win32 probes live in the Windows backend.

use crate::error::CliError;

/// Guidance for the refusal (exit 3).
pub(crate) const REFUSAL: &str = "this account always runs elevated, so its logon task would run \
as a system process; install from an elevated terminal (Run as administrator) to get a system \
service";

/// Whether the account's logon task would run elevated. A missing or unreadable
/// `EnableLUA` counts as UAC enabled, so it never refuses on its own.
pub(crate) fn always_elevated(
    is_builtin_admin: bool,
    is_admin_member: bool,
    enable_lua: Option<bool>,
) -> bool {
    is_builtin_admin || (is_admin_member && enable_lua == Some(false))
}

/// `Err(Permission)` when the account would run its task elevated.
pub(crate) fn refuse_if_always_elevated(
    is_builtin_admin: bool,
    is_admin_member: bool,
    enable_lua: Option<bool>,
) -> Result<(), CliError> {
    if always_elevated(is_builtin_admin, is_admin_member, enable_lua) {
        Err(CliError::Permission(REFUSAL.into()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_elevated_account_refuses_user_task() {
        assert!(always_elevated(false, true, Some(false)));
        assert!(always_elevated(true, true, Some(true)));
        assert!(always_elevated(true, true, None));
        let err = refuse_if_always_elevated(false, true, Some(false)).unwrap_err();
        assert!(
            matches!(&err, CliError::Permission(m) if m == REFUSAL),
            "{err:?}"
        );
    }

    #[test]
    fn standard_user_with_uac_disabled_gets_a_user_task() {
        assert!(refuse_if_always_elevated(false, false, Some(false)).is_ok());
    }

    #[test]
    fn admin_with_uac_enabled_gets_a_user_task() {
        assert!(refuse_if_always_elevated(false, true, Some(true)).is_ok());
        // Unknown EnableLUA is treated as enabled.
        assert!(refuse_if_always_elevated(false, true, None).is_ok());
    }
}
