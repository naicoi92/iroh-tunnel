//! Whose files a Windows system service uses (SM-04): the installing user.
//!
//! Pure: the token user, the owner lookup, the LocalAppData lookup and the
//! account-name lookup are injected, so every rule runs on any platform.

// Pure and compiled everywhere for its tests, but only Windows consumes it.
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::pin::{Pin, PinOwner};
use super::trust_windows::model::{Sid, ADMINISTRATORS, SYSTEM, TRUSTED_INSTALLER};
use crate::error::CliError;

/// The user whose files a Windows system service uses, and their pinned paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WinTarget {
    pub sid: Sid,
    /// `DOMAIN\name`, or `?` when the SID cannot be named.
    pub account: String,
    /// Config file path.
    pub config: PathBuf,
    /// State directory.
    pub state: PathBuf,
}

impl WinTarget {
    /// The pin recorded in the registration.
    pub(crate) fn pin(&self) -> Pin {
        Pin {
            config: self.config.clone(),
            owner: PinOwner::Sid(self.sid.0.clone()),
        }
    }
}

/// Everything the user choice depends on, besides the token.
pub(crate) struct WinRequest<'a> {
    pub role: &'a str,
    /// `--config`, if given.
    pub config: Option<&'a Path>,
    /// `IROH_TUNNEL_STATE_DIR`, if set; verified later by the trust check.
    pub state: Option<&'a Path>,
}

fn is_machine_owner(sid: &Sid) -> bool {
    [SYSTEM, ADMINISTRATORS, TRUSTED_INSTALLER].contains(&sid.0.as_str())
}

/// Decide the account and paths for a system install: the token user is the
/// installing user.
///
/// # Errors
///
/// [`CliError::Config`] when `--config` sits in a directory owned by another
/// user, when its owner cannot be read, or when LocalAppData cannot be found.
pub(crate) fn resolve_target(
    req: &WinRequest<'_>,
    token: &Sid,
    owner_of: &dyn Fn(&Path) -> Option<Sid>,
    local_appdata: &dyn Fn(&Sid) -> Result<PathBuf>,
    account_of: &dyn Fn(&Sid) -> Option<String>,
) -> Result<WinTarget> {
    // Looked up once, and only when a default path needs it.
    let appdata = if req.config.is_none() || req.state.is_none() {
        Some(local_appdata(token)?)
    } else {
        None
    };
    let name = |sid: &Sid| account_of(sid).unwrap_or_else(|| sid.0.clone());

    let config = match req.config {
        Some(config) => {
            let parent = config.parent().unwrap_or(config);
            let owner = owner_of(parent).ok_or_else(|| {
                CliError::Config(format!(
                    "cannot determine the owner of {} (the directory of --config); pass a \
                     --config in a directory you can read. Nothing was changed.",
                    parent.display()
                ))
            })?;
            if !is_machine_owner(&owner) && owner != *token {
                return Err(CliError::Config(format!(
                    "--config {} is in a directory owned by {} but the installing user is {}; \
                     the service can only trust one user. Run from {}'s session, or pass a \
                     --config under {}'s profile. Nothing was changed.",
                    config.display(),
                    name(&owner),
                    name(token),
                    name(&owner),
                    name(token)
                ))
                .into());
            }
            config.to_path_buf()
        }
        None => crate::paths::windows_config_path_in(
            appdata.as_deref().expect("looked up above"),
            req.role,
        ),
    };
    let state = match req.state {
        Some(state) => state.to_path_buf(),
        None => crate::paths::windows_state_path_in(appdata.as_deref().expect("looked up above")),
    };
    Ok(WinTarget {
        sid: token.clone(),
        account: account_of(token).unwrap_or_else(|| "?".into()),
        config,
        state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "S-1-5-21-1-2-3-1001";
    const BOB: &str = "S-1-5-21-1-2-3-1002";

    fn sid(s: &str) -> Sid {
        Sid(s.into())
    }

    fn account_of(s: &Sid) -> Option<String> {
        match s.0.as_str() {
            ALICE => Some("PC\\alice".into()),
            BOB => Some("PC\\bob".into()),
            _ => None,
        }
    }

    fn appdata(_: &Sid) -> Result<PathBuf> {
        Ok(PathBuf::from("C:\\Users\\alice\\AppData\\Local"))
    }

    fn resolve(
        config: Option<&Path>,
        state: Option<&Path>,
        owner: Option<&str>,
    ) -> Result<WinTarget> {
        let req = WinRequest {
            role: "serve",
            config,
            state,
        };
        resolve_target(
            &req,
            &sid(ALICE),
            &|_| owner.map(sid),
            &appdata,
            &account_of,
        )
    }

    fn class(err: &anyhow::Error) -> &CliError {
        err.downcast_ref::<CliError>().expect("a CliError")
    }

    #[test]
    fn defaults_use_token_user_paths() {
        let t = resolve(None, None, None).unwrap();
        let root = Path::new("C:\\Users\\alice\\AppData\\Local");
        assert_eq!(t.sid, sid(ALICE));
        assert_eq!(t.account, "PC\\alice");
        assert_eq!(
            t.config,
            root.join("iroh-tunnel").join("config").join("serve.toml")
        );
        assert_eq!(t.state, root.join("iroh-tunnel").join("state"));
        assert_eq!(t.pin().owner, PinOwner::Sid(ALICE.into()));
        assert_eq!(t.pin().config, t.config);
    }

    #[test]
    fn explicit_state_dir_is_used_as_given() {
        let state = Path::new("D:\\state");
        assert_eq!(resolve(None, Some(state), None).unwrap().state, state);
    }

    #[test]
    fn config_owned_by_the_token_user_is_accepted() {
        let cfg = Path::new("C:\\Users\\alice\\c.toml");
        let t = resolve(Some(cfg), None, Some(ALICE)).unwrap();
        assert_eq!((t.sid, t.config), (sid(ALICE), cfg.to_path_buf()));
    }

    #[test]
    fn config_owned_by_other_user_is_config_error() {
        let cfg = Path::new("C:\\Users\\bob\\c.toml");
        let err = resolve(Some(cfg), None, Some(BOB)).unwrap_err();
        let CliError::Config(msg) = class(&err) else {
            panic!("not a config error: {err}")
        };
        assert!(
            msg.contains("PC\\bob") && msg.contains("PC\\alice"),
            "{msg}"
        );
        assert!(msg.contains("--config"), "{msg}");
    }

    #[test]
    fn unnamed_owner_falls_back_to_the_sid() {
        let cfg = Path::new("C:\\x\\c.toml");
        let err = resolve(Some(cfg), None, Some("S-1-5-21-9-9-9-5")).unwrap_err();
        assert!(err.to_string().contains("S-1-5-21-9-9-9-5"), "{err}");
    }

    fn defers(owner: &str) {
        let cfg = Path::new("C:\\ProgramData\\c.toml");
        let t = resolve(Some(cfg), None, Some(owner)).unwrap();
        assert_eq!(t.sid, sid(ALICE));
        assert_eq!(t.pin().owner, PinOwner::Sid(ALICE.into()));
    }

    #[test]
    fn config_dir_owned_by_administrators_defers_to_token() {
        defers(ADMINISTRATORS);
    }

    #[test]
    fn config_dir_owned_by_system_defers_to_token() {
        defers(SYSTEM);
    }

    #[test]
    fn config_dir_owned_by_trusted_installer_defers_to_token() {
        defers(TRUSTED_INSTALLER);
    }

    #[test]
    fn config_dir_owner_unreadable_is_config_error() {
        let err = resolve(Some(Path::new("C:\\x\\c.toml")), None, None).unwrap_err();
        let CliError::Config(msg) = class(&err) else {
            panic!("not a config error: {err}")
        };
        assert!(msg.contains("cannot determine the owner"), "{msg}");
    }
}
