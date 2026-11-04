//! Installing a system service across the privilege boundary (SM-02, SM-04;
//! issue #91).
//!
//! Root installs, but the service reads the *installing user's* files. This
//! module decides whose files, and confirms before an existing service that
//! belongs to someone else is retargeted. It never infers a user from root's
//! own home.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::account::{self, Account};
use super::pin::{Pin, RetargetConsent};
use super::trust;
use crate::error::CliError;

/// The user whose files a system service uses, and their pinned paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub account: Account,
    /// Canonical config file path.
    pub config: PathBuf,
    /// Absolute state directory.
    pub state: PathBuf,
}

impl Target {
    /// The pin recorded in the registration.
    pub fn pin(&self) -> Pin {
        Pin {
            config: self.config.clone(),
            uid: self.account.uid,
        }
    }
}

/// Everything the user choice depends on, injected for testing.
pub struct Request<'a> {
    pub role: &'a str,
    /// `--config`, if given.
    pub config: Option<&'a Path>,
    /// `IROH_TUNNEL_STATE_DIR`, if set.
    pub state: Option<&'a Path>,
    /// `SUDO_USER`: only an input to the choice, always re-resolved through
    /// the user database.
    pub sudo_user: Option<&'a str>,
}

pub(crate) fn unknown_user(why: &str, role: &str) -> anyhow::Error {
    CliError::Config(format!(
        "cannot tell which user this system service is for ({why}); the service must not fall \
         back to root's home. Re-run from the user's session with `sudo iroh-tunnel {role} \
         service install --system`, or pass `--config <the user's config file>` so the owner of \
         its directory is used. Nothing was changed."
    ))
    .into()
}

fn sudo_account(request: &Request<'_>) -> Option<Account> {
    request
        .sudo_user
        .filter(|name| !name.is_empty())
        .and_then(account::by_name)
        .filter(|a| a.uid != 0)
}

/// Decide the account and absolute paths for a system install.
///
/// Order: `SUDO_USER` (a real non-root account); else the owner of an explicit
/// `--config`'s directory; else the supplied `invoking_user`; never root's
/// home. `Ok(None)` means the user is unknown
/// and `invoking_user` (a name to look up) was not supplied.
///
/// # Errors
///
/// [`CliError::Config`] when the user or the paths cannot be resolved.
pub fn resolve_target_account(
    request: &Request<'_>,
    invoking_user: Option<&str>,
) -> Result<Option<Target>> {
    let from_sudo = sudo_account(request);

    let config_dir_owner = |config: &Path| -> Result<Account> {
        let parent = config.parent().unwrap_or(Path::new("/"));
        let uid = std::fs::metadata(parent)
            .map_err(|e| {
                unknown_user(
                    &format!("cannot inspect {}: {e}", parent.display()),
                    request.role,
                )
            })?
            .uid();
        account::by_uid(uid).ok_or_else(|| {
            unknown_user(
                &format!("no account owns {}", parent.display()),
                request.role,
            )
        })
    };

    let account = match (from_sudo, request.config) {
        (Some(account), Some(config)) => {
            // Both given: they must agree, or root would be asked to trust a
            // file that belongs to someone other than the pinned user.
            let owner = config_dir_owner(&trust::canonical(config)?)?;
            if owner.uid != account.uid {
                return Err(CliError::Config(format!(
                    "SUDO_USER is {} but {} is in a directory owned by {}; the service can \
                     only trust one user. Run from {}'s session, or pass a --config under \
                     {}'s home. Nothing was changed.",
                    account.name,
                    config.display(),
                    owner.name,
                    owner.name,
                    account.name
                ))
                .into());
            }
            account
        }
        (Some(account), None) => account,
        (None, Some(config)) => config_dir_owner(&trust::canonical(config)?)?,
        (None, None) => match invoking_user {
            Some(name) => account::by_name(name)
                .ok_or_else(|| unknown_user(&format!("no such user {name:?}"), request.role))?,
            None => return Ok(None),
        },
    };

    let config = match request.config {
        Some(path) => trust::canonical(path)?,
        None => {
            let path = crate::paths::config_path_in(&account.home, request.role);
            trust::canonical(&path).map_err(|_| {
                CliError::Config(format!(
                    "no configuration for user {} at {}. Create it as that user \
                     (`iroh-tunnel {} config keygen`), or pass --config",
                    account.name,
                    path.display(),
                    request.role
                ))
            })?
        }
    };
    // Canonical like the config: the runtime walk refuses every symlink, so a
    // symlinked home or /var must be resolved once, here (SM-04).
    let state = trust::canonical(&match request.state {
        Some(path) => std::path::absolute(path)?,
        None => crate::paths::state_dir_in(&account.home),
    })
    .map_err(|_| {
        CliError::Config(format!(
            "the state directory for user {} does not exist; create it as that user \
             (`mkdir -p`), the service does not create directories in a user's home as root",
            account.name
        ))
    })?;
    Ok(Some(Target {
        account,
        config,
        state,
    }))
}

/// Whether an install may replace the existing registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetargetDecision {
    Proceed,
    Needs {
        previous: Option<Pin>,
        proposed: Pin,
    },
}

/// Pure retarget policy: nothing installed or the same user proceeds; another
/// owner (or an unpinned registration) proceeds only with consent that matches
/// the current registration and the proposed pin.
pub fn retarget_decision(
    existing: Option<&str>,
    target: &Target,
    consent: Option<&RetargetConsent>,
) -> RetargetDecision {
    let Some(body) = existing else {
        return RetargetDecision::Proceed;
    };
    let previous = Pin::find(body);
    if previous
        .as_ref()
        .is_some_and(|p| p.uid == target.account.uid)
    {
        return RetargetDecision::Proceed;
    }
    let proposed = target.pin();
    if consent.is_some_and(|c| c.previous == previous && c.proposed == proposed) {
        return RetargetDecision::Proceed;
    }
    RetargetDecision::Needs { previous, proposed }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(name: &str, uid: u32) -> Account {
        Account {
            name: name.into(),
            uid,
            home: PathBuf::from("/home").join(name),
        }
    }

    fn target(uid: u32) -> Target {
        Target {
            account: account("bob", uid),
            config: PathBuf::from("/home/bob/.config/iroh-tunnel/serve.toml"),
            state: PathBuf::from("/home/bob/.local/state/iroh-tunnel"),
        }
    }

    fn pinned_body(uid: u32) -> String {
        format!("# {}\n", target(uid).pin().record())
    }

    #[test]
    fn retarget_without_consent_needs_confirmation() {
        let body = pinned_body(1001);
        assert_eq!(
            retarget_decision(Some(&body), &target(1002), None),
            RetargetDecision::Needs {
                previous: Some(target(1001).pin()),
                proposed: target(1002).pin(),
            }
        );
    }

    #[test]
    fn stale_retarget_consent_is_not_applied() {
        let consent = RetargetConsent {
            previous: Some(target(1001).pin()),
            proposed: target(1002).pin(),
        };
        let body = pinned_body(1003);
        assert_eq!(
            retarget_decision(Some(&body), &target(1002), Some(&consent)),
            RetargetDecision::Needs {
                previous: Some(target(1003).pin()),
                proposed: target(1002).pin(),
            }
        );
        let body = pinned_body(1001);
        assert_eq!(
            retarget_decision(Some(&body), &target(1002), Some(&consent)),
            RetargetDecision::Proceed
        );
    }

    #[test]
    fn retarget_from_unpinned_registration_matches_existing_policy() {
        let pinned_other = pinned_body(1001);
        let pinned_same = pinned_body(1000);
        let unpinned_consent = RetargetConsent {
            previous: None,
            proposed: target(1000).pin(),
        };
        let other_consent = RetargetConsent {
            previous: Some(target(1001).pin()),
            proposed: target(1000).pin(),
        };
        // (existing, consent, expected proceeds) — mirrors confirm_retarget.
        let cases: [(Option<&str>, Option<&RetargetConsent>, bool); 8] = [
            (None, None, true),
            (Some(&pinned_same), None, true),
            (Some(&pinned_other), None, false),
            (Some(&pinned_other), Some(&other_consent), true),
            (Some("[Service]\n"), None, false),
            (Some("[Service]\n"), Some(&unpinned_consent), true),
            (Some("[Service]\n"), Some(&other_consent), false),
            (Some(&pinned_other), Some(&unpinned_consent), false),
        ];
        for (existing, consent, proceeds) in cases {
            let decision = retarget_decision(existing, &target(1000), consent);
            assert_eq!(
                decision == RetargetDecision::Proceed,
                proceeds,
                "{existing:?} {consent:?}"
            );
        }
        assert_eq!(
            retarget_decision(Some("[Service]\n"), &target(1000), None),
            RetargetDecision::Needs {
                previous: None,
                proposed: target(1000).pin()
            }
        );
    }

    #[test]
    fn target_account_needs_invoking_user_instead_of_prompting() {
        let request = Request {
            role: "serve",
            config: None,
            state: None,
            sudo_user: None,
        };
        assert!(resolve_target_account(&request, None).unwrap().is_none());
    }

    #[test]
    fn unknown_user_error_names_the_rerun_and_config_options() {
        let err = unknown_user("no SUDO_USER", "serve");
        let Some(CliError::Config(msg)) = err.downcast_ref::<CliError>() else {
            panic!("{err:?}");
        };
        assert!(
            msg.contains("sudo iroh-tunnel serve service install --system"),
            "{msg}"
        );
        assert!(msg.contains("--config"), "{msg}");
    }

    #[test]
    fn sudo_user_and_a_config_owned_by_someone_else_is_a_named_conflict() {
        // The current uid owns the temp dir; `root` stands in for SUDO_USER,
        // which is rejected as an install target, so use the owner mismatch
        // through a config whose directory owner differs from the account.
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("serve.toml");
        std::fs::write(&config, "x").unwrap();
        let me = account::by_uid(unsafe { libc::geteuid() }).expect("current account");
        if me.uid == 0 {
            return; // root owns everything; the mismatch needs a non-root owner
        }
        let other = if me.name == "daemon" { "bin" } else { "daemon" };
        if account::by_name(other).is_none_or(|a| a.uid == 0 || a.uid == me.uid) {
            return;
        }
        let request = Request {
            role: "serve",
            config: Some(&config),
            state: None,
            sudo_user: Some(other),
        };
        let err = resolve_target_account(&request, None).unwrap_err();
        let Some(CliError::Config(msg)) = err.downcast_ref::<CliError>() else {
            panic!("{err:?}");
        };
        assert!(msg.contains(other) && msg.contains(&me.name), "{msg}");
        assert!(msg.contains("--config"), "{msg}");
    }
}
