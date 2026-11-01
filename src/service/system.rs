//! Installing a system service across the privilege boundary (SM-02, SM-04;
//! issue #91).
//!
//! Root installs, but the service reads the *installing user's* files. This
//! module decides whose files, and confirms before an existing service that
//! belongs to someone else is retargeted. It never infers a user from root's
//! own home.

use std::io::{BufRead, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::account::{self, Account};
use super::pin::Pin;
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
    pub interactive: bool,
}

fn unknown_user(why: &str, role: &str) -> anyhow::Error {
    CliError::Config(format!(
        "cannot tell which user this system service is for ({why}); the service must not fall \
         back to root's home. Re-run from the user's session with `sudo iroh-tunnel {role} \
         service install --system`, or pass `--config <the user's config file>` so the owner of \
         its directory is used. Nothing was changed."
    ))
    .into()
}

/// Decide the account and absolute paths for a system install.
///
/// Order: `SUDO_USER` (a real non-root account); else the owner of an explicit
/// `--config`'s directory; else an interactive prompt; else a failure with
/// guidance. Never root's home.
///
/// # Errors
///
/// [`CliError::Config`] when the user is unknown or the paths cannot be
/// resolved; nothing is changed.
pub fn resolve_target(
    request: &Request<'_>,
    input: &mut impl BufRead,
    prompt: &mut impl Write,
) -> Result<Target> {
    let from_sudo = request
        .sudo_user
        .filter(|name| !name.is_empty())
        .and_then(account::by_name)
        .filter(|a| a.uid != 0);

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
        (None, None) if request.interactive => ask_user(request.role, input, prompt)?,
        (None, None) => return Err(unknown_user("no SUDO_USER", request.role)),
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
    Ok(Target {
        account,
        config,
        state,
    })
}

fn ask_user(role: &str, input: &mut impl BufRead, prompt: &mut impl Write) -> Result<Account> {
    write!(
        prompt,
        "Which user's configuration should the system service use? (user name): "
    )?;
    prompt.flush()?;
    let mut answer = String::new();
    if input.read_line(&mut answer)? == 0 {
        return Err(unknown_user("no answer to the user prompt", role));
    }
    let name = answer.trim();
    account::by_name(name).ok_or_else(|| unknown_user(&format!("no such user {name:?}"), role))
}

/// Confirm before replacing an existing system registration that belongs to
/// another user, or whose owner cannot be told (SM-02, AC-SM-05).
///
/// `existing` is the current registration body, `None` when nothing is
/// installed. The same user re-installing is not a retarget.
///
/// # Errors
///
/// [`CliError::Service`] when the retarget is declined or cannot be confirmed
/// without a terminal; the registration and both users' files are untouched.
pub fn confirm_retarget(
    existing: Option<&str>,
    target: &Target,
    interactive: bool,
    input: &mut impl BufRead,
    prompt: &mut impl Write,
) -> Result<()> {
    let Some(body) = existing else {
        return Ok(());
    };
    let previous = Pin::find(body);
    if previous
        .as_ref()
        .is_some_and(|p| p.uid == target.account.uid)
    {
        return Ok(());
    }
    let owner = match &previous {
        Some(p) => format!(
            "user {} (uid {}, config {})",
            account::by_uid(p.uid).map_or_else(|| "?".to_owned(), |a| a.name),
            p.uid,
            p.config.display()
        ),
        None => "an unknown owner (no pin recorded)".to_owned(),
    };
    let question = format!(
        "A system service is already installed for {owner}. Retarget it to user {} (config {})? \
         Both users' config files are kept. [y/N]: ",
        target.account.name,
        target.config.display()
    );
    let declined = |why: &str| -> anyhow::Error {
        CliError::Service(format!(
            "{why}; the existing service and both users' config files are unchanged"
        ))
        .into()
    };
    if !interactive {
        return Err(declined(&format!(
            "a system service already exists for {owner} and retargeting needs a confirmation \
             that cannot be asked without a terminal; run this install interactively"
        )));
    }
    write!(prompt, "{question}")?;
    prompt.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(declined("retarget declined"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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

    fn confirm(existing: Option<&str>, uid: u32, interactive: bool, answer: &str) -> Result<()> {
        confirm_retarget(
            existing,
            &target(uid),
            interactive,
            &mut Cursor::new(answer.to_owned()),
            &mut Vec::new(),
        )
    }

    #[test]
    fn nothing_installed_or_same_user_needs_no_confirmation() {
        assert!(confirm(None, 1000, false, "").is_ok());
        let body = format!("# {}\n", target(1000).pin().record());
        assert!(confirm(Some(&body), 1000, false, "").is_ok());
    }

    #[test]
    fn another_users_service_needs_an_explicit_yes() {
        let body = format!("# {}\n", target(1001).pin().record());
        assert!(confirm(Some(&body), 1000, true, "y\n").is_ok());
        assert!(confirm(Some(&body), 1000, true, "YES\n").is_ok());
        for refusal in ["n\n", "\n", "", "maybe\n"] {
            assert!(
                confirm(Some(&body), 1000, true, refusal).is_err(),
                "{refusal:?}"
            );
        }
    }

    #[test]
    fn unpinned_registration_counts_as_another_owner() {
        assert!(confirm(Some("[Service]\n"), 1000, true, "n\n").is_err());
        assert!(confirm(Some("[Service]\n"), 1000, true, "y\n").is_ok());
    }

    #[test]
    fn retarget_without_a_terminal_fails_instead_of_assuming_yes() {
        let body = format!("# {}\n", target(1001).pin().record());
        let err = confirm(Some(&body), 1000, false, "y\n").unwrap_err();
        assert!(matches!(
            err.downcast_ref::<CliError>(),
            Some(CliError::Service(_))
        ));
    }

    #[test]
    fn unknown_user_without_a_terminal_or_config_is_an_actionable_error() {
        let request = Request {
            role: "serve",
            config: None,
            state: None,
            sudo_user: None,
            interactive: false,
        };
        let err =
            resolve_target(&request, &mut Cursor::new(String::new()), &mut Vec::new()).unwrap_err();
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
            interactive: false,
        };
        let err =
            resolve_target(&request, &mut Cursor::new(String::new()), &mut Vec::new()).unwrap_err();
        let Some(CliError::Config(msg)) = err.downcast_ref::<CliError>() else {
            panic!("{err:?}");
        };
        assert!(msg.contains(other) && msg.contains(&me.name), "{msg}");
        assert!(msg.contains("--config"), "{msg}");
    }

    #[test]
    fn interactive_unknown_user_is_asked_and_a_bad_answer_is_an_error() {
        let request = Request {
            role: "serve",
            config: None,
            state: None,
            sudo_user: Some("root"),
            interactive: true,
        };
        let mut prompt = Vec::new();
        let err = resolve_target(
            &request,
            &mut Cursor::new("no-such-user-iroh\n".to_owned()),
            &mut prompt,
        )
        .unwrap_err();
        assert!(String::from_utf8(prompt).unwrap().contains("user name"));
        assert!(matches!(
            err.downcast_ref::<CliError>(),
            Some(CliError::Config(_))
        ));
    }
}
