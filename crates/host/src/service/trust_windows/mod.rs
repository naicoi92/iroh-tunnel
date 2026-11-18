//! Windows trust policy for files a LocalSystem service reads.
//!
//! `model` is pure and compiles on every platform; `win32` is the adapter
//! that feeds it real descriptors read from no-follow handles.
#![cfg_attr(
    not(windows),
    expect(
        dead_code,
        reason = "the model is pure and compiled everywhere, but only Windows consumes it"
    )
)]

mod ace;
// On Windows `win32` consumes the model.
pub(crate) mod model;
#[cfg(windows)]
pub(crate) mod win32;

#[cfg(windows)]
pub(crate) use win32::{
    account_of, create_dir_with_sddl, owner_of, program_data_dir, publish, read_in, read_verified,
    remove_in, token_user, verify_binary, verify_confidential_with, verify_dir,
};

/// Serialised, scoped process-environment edits shared by every test that
/// sets the service variables (the environment is global).
#[cfg(test)]
#[cfg(windows)]
pub(crate) mod test_env {
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard};

    static LOCK: Mutex<()> = Mutex::new(());

    pub(crate) struct EnvGuard {
        saved: Vec<(&'static str, Option<OsString>)>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        /// Set (`Some`) or unset (`None`) each variable until dropped.
        pub(crate) fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
            let lock = LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let saved = vars
                .iter()
                .map(|(k, v)| {
                    let old = std::env::var_os(k);
                    match v {
                        Some(v) => std::env::set_var(k, v),
                        None => std::env::remove_var(k),
                    }
                    (*k, old)
                })
                .collect();
            Self { saved, _lock: lock }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, old) in self.saved.drain(..).rev() {
                match old {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}

use crate::error::CliError;
use model::Sid;

/// Verify the machine directory against the machine trust set.
///
/// # Errors
///
/// [`CliError::Permission`] naming the unsafe element.
#[cfg(windows)]
pub(crate) fn verify_machine_dir(dir: &std::path::Path) -> anyhow::Result<()> {
    verify_dir(dir, &model::TrustedSet::machine())
}

/// The `io::ErrorKind` for a failed `CreateDirectoryW` with the protected
/// machine descriptor. A non-elevated token cannot assign the Administrators
/// owner (`ERROR_INVALID_OWNER` 1307, `ERROR_PRIVILEGE_NOT_HELD` 1314); that
/// means "not elevated", the same as `ERROR_ACCESS_DENIED` (5).
pub(crate) fn map_create_error(code: u32) -> std::io::ErrorKind {
    match code {
        5 | 1307 | 1314 => std::io::ErrorKind::PermissionDenied,
        183 => std::io::ErrorKind::AlreadyExists,
        _ => std::io::ErrorKind::Other,
    }
}

/// The admin-only trusted set of a system service's files (SYSTEM,
/// Administrators, TrustedInstaller), plus the SID a test build names in
/// `test_owner` (always `None` in production).
///
/// # Errors
///
/// [`CliError::Config`] when `test_owner` is not a SID.
pub(crate) fn admin_set(test_owner: Option<&std::ffi::OsStr>) -> anyhow::Result<model::TrustedSet> {
    let Some(text) = test_owner else {
        return Ok(model::TrustedSet::machine());
    };
    #[cfg(any(test, feature = "test-utils"))]
    {
        text.to_str()
            .and_then(model::parse_sid)
            .map(model::TrustedSet::admin_with_test_owner)
            .ok_or_else(|| {
                CliError::Config(format!(
                    "IROH_TUNNEL_TEST_TRUSTED_OWNER must be a SID, got {}",
                    text.to_string_lossy()
                ))
                .into()
            })
    }
    #[cfg(not(any(test, feature = "test-utils")))]
    {
        let _ = text;
        Ok(model::TrustedSet::machine())
    }
}

/// Refuse `path` (a config directory or file) when its DACL grants any read
/// right to a SID outside SYSTEM, Administrators and TrustedInstaller (and
/// the test-build owner). Never changes the DACL.
///
/// # Errors
///
/// [`CliError::Permission`] naming the path and the offending SID, or when
/// the path cannot be inspected.
#[cfg(windows)]
#[expect(
    dead_code,
    reason = "the elevated install's config check calls it once the installer stops pinning a user"
)]
pub(crate) fn verify_confidential(path: &std::path::Path) -> anyhow::Result<()> {
    let trusted = super::admin_trust(&super::process_env)?;
    verify_confidential_with(path, &trusted)
}

/// Set by the SCM registration of a system service: the SID whose files the
/// service may trust besides the system principals.
pub const ENV_TRUSTED_SID: &str = "IROH_TUNNEL_TRUSTED_SID";

/// The trust pin requested by the registration, if any.
///
/// # Errors
///
/// [`CliError::Config`] when the variable is set but is not a SID.
#[cfg_attr(
    windows,
    expect(
        dead_code,
        reason = "the run path refuses this pin as legacy; removed with the installer pin"
    )
)]
pub(crate) fn trusted_sid_from_env() -> anyhow::Result<Option<Sid>> {
    trusted_sid_from(std::env::var(ENV_TRUSTED_SID))
}

/// [`trusted_sid_from_env`] with the lookup injected, so it is testable
/// without touching the process environment.
#[cfg_attr(
    all(windows, not(test)),
    expect(
        dead_code,
        reason = "only the legacy pin reader calls it; removed with the installer pin"
    )
)]
fn trusted_sid_from(value: Result<String, std::env::VarError>) -> anyhow::Result<Option<Sid>> {
    match value {
        Ok(text) => model::parse_sid(&text).map(Some).ok_or_else(|| {
            CliError::Config(format!(
                "{ENV_TRUSTED_SID} must be a SID; re-run the service install"
            ))
            .into()
        }),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(e) => Err(CliError::Config(format!("{ENV_TRUSTED_SID}: {e}")).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::VarError;

    #[test]
    fn create_errors_for_an_unelevated_token_are_permission_denied() {
        use std::io::ErrorKind::{AlreadyExists, Other, PermissionDenied};
        assert_eq!(map_create_error(1307), PermissionDenied);
        assert_eq!(map_create_error(1314), PermissionDenied);
        assert_eq!(map_create_error(5), PermissionDenied);
        assert_eq!(map_create_error(183), AlreadyExists);
        assert_eq!(map_create_error(3), Other);
    }

    fn config_message(r: anyhow::Result<Option<Sid>>) -> String {
        match r.unwrap_err().downcast_ref::<CliError>() {
            Some(CliError::Config(m)) => m.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn admin_set_is_machine_without_a_test_owner_and_refuses_a_bad_one() {
        assert!(admin_set(None).is_ok());
        assert!(admin_set(Some(std::ffi::OsStr::new("S-1-5-21-1-2-3-1001"))).is_ok());
        let msg = config_message(admin_set(Some(std::ffi::OsStr::new("alice"))).map(|_| None));
        assert!(msg.contains("IROH_TUNNEL_TEST_TRUSTED_OWNER"), "{msg}");
    }

    #[test]
    fn trusted_sid_absent_is_none_and_valid_is_parsed() {
        assert_eq!(trusted_sid_from(Err(VarError::NotPresent)).unwrap(), None);
        assert_eq!(
            trusted_sid_from(Ok("S-1-5-21-1-2-3-1001".into())).unwrap(),
            Some(Sid("S-1-5-21-1-2-3-1001".into()))
        );
    }

    #[test]
    fn malformed_trusted_sid_is_a_config_error() {
        for bad in ["", "garbage", "S-1-5", " S-1-5-18"] {
            assert_eq!(
                config_message(trusted_sid_from(Ok(bad.into()))),
                "IROH_TUNNEL_TRUSTED_SID must be a SID; re-run the service install",
                "{bad:?}"
            );
        }
        let msg = config_message(trusted_sid_from(Err(VarError::NotUnicode("x".into()))));
        assert!(msg.contains(ENV_TRUSTED_SID), "{msg}");
    }
}
