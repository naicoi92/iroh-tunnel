//! Windows trust policy for files a LocalSystem service reads.
//!
//! `model` is pure and compiles on every platform; `win32` is the adapter
//! that feeds it real descriptors read from no-follow handles.
#![cfg_attr(not(windows), allow(dead_code))]

mod ace;
// On Windows `win32` consumes the model; only `EVERYONE`, `CREATOR_OWNER`
// (test fixtures) and `TrustedSet::{for_user_files, machine}` (service
// runtime and machine dir, later tasks) remain unused there.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) mod model;
#[cfg(windows)]
pub(crate) mod win32;

#[cfg(windows)]
#[allow(unused_imports)] // consumed by the service runtime and SCM install later
pub(crate) use win32::{
    account_of, owner_of, publish, read_verified, remove_in, token_user, verify_binary, verify_dir,
};

use crate::error::CliError;
use model::Sid;

/// Set by the SCM registration of a system service: the SID whose files the
/// service may trust besides the system principals.
pub const ENV_TRUSTED_SID: &str = "IROH_TUNNEL_TRUSTED_SID";

/// The trust pin requested by the registration, if any.
///
/// # Errors
///
/// [`CliError::Config`] when the variable is set but is not a SID.
#[cfg_attr(windows, allow(dead_code))] // consumed by the service runtime later
pub(crate) fn trusted_sid_from_env() -> anyhow::Result<Option<Sid>> {
    trusted_sid_from(std::env::var(ENV_TRUSTED_SID))
}

/// [`trusted_sid_from_env`] with the lookup injected, so it is testable
/// without touching the process environment.
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

    fn config_message(r: anyhow::Result<Option<Sid>>) -> String {
        match r.unwrap_err().downcast_ref::<CliError>() {
            Some(CliError::Config(m)) => m.clone(),
            other => panic!("{other:?}"),
        }
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
