//! Service scope selection and privilege checks (SM-01, SM-03; issue #90).
//!
//! - A non-elevated user chooses user or system; elevated (root /
//!   Administrator) installs default to system without a prompt.
//! - Choosing system without privilege stops with rerun guidance that keeps
//!   the intended arguments and absolute paths. The process never elevates
//!   itself and never falls back to a user service.

use std::io::{BufRead, Write};

use anyhow::Result;

use super::ServiceScope;
use crate::error::CliError;

/// What the install-time scope decision depends on, injected so the policy is
/// a pure function of its inputs.
#[derive(Debug, Clone, Copy)]
pub struct Environment {
    /// The process runs as root / an elevated Administrator.
    pub elevated: bool,
    /// Standard input is a terminal that can answer the scope prompt.
    pub interactive: bool,
}

impl Environment {
    /// Probe the running process.
    pub fn detect() -> Self {
        Self {
            elevated: is_elevated(),
            interactive: is_interactive(),
        }
    }
}

/// Whether the process runs with root privileges. Always `false` on targets
/// without a service backend (where every operation is refused anyway).
#[cfg(unix)]
pub fn is_elevated() -> bool {
    // SAFETY: geteuid(2) has no preconditions and no failure mode.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
pub fn is_elevated() -> bool {
    false
}

/// Whether a person can answer a prompt (stdin is a terminal).
#[cfg(feature = "test-utils")]
pub(crate) fn is_interactive() -> bool {
    use std::io::IsTerminal;
    // Integration tests have no terminal; this stands in for one.
    std::env::var_os("IROH_TUNNEL_TEST_INTERACTIVE").is_some() || std::io::stdin().is_terminal()
}

#[cfg(not(feature = "test-utils"))]
pub(crate) fn is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

/// Scope for verbs that act on an existing service when no flag is given:
/// the elevated installer owns a system service, everyone else a user one.
pub fn default_scope(elevated: bool) -> ServiceScope {
    if elevated {
        ServiceScope::System
    } else {
        ServiceScope::User
    }
}

/// Decide the install scope.
///
/// An explicit request wins. Otherwise an elevated process installs a system
/// service without asking; a non-elevated interactive process is asked; a
/// non-elevated non-interactive process gets the unprivileged user scope.
///
/// # Errors
///
/// [`CliError::Config`] when the prompt gets no answer or an unrecognised one:
/// the scope is never guessed.
pub fn choose_install_scope(
    requested: Option<ServiceScope>,
    env: Environment,
    input: &mut impl BufRead,
    prompt: &mut impl Write,
) -> Result<ServiceScope> {
    if let Some(scope) = requested {
        return Ok(scope);
    }
    if env.elevated {
        return Ok(ServiceScope::System);
    }
    if !env.interactive {
        return Ok(ServiceScope::User);
    }
    write!(
        prompt,
        "Install as a `user` service (no privileges, runs for your account) \
         or a `system` service (needs sudo/root)? [user]: "
    )?;
    prompt.flush()?;
    let mut answer = String::new();
    if input.read_line(&mut answer)? == 0 {
        return Err(CliError::Config(
            "no answer to the scope prompt; pass --user or --system".into(),
        )
        .into());
    }
    parse_answer(&answer)
}

fn parse_answer(answer: &str) -> Result<ServiceScope> {
    match answer.trim().to_ascii_lowercase().as_str() {
        "" | "u" | "user" => Ok(ServiceScope::User),
        "s" | "system" => Ok(ServiceScope::System),
        other => Err(CliError::Config(format!(
            "unrecognised scope {other:?}; answer `user` or `system`, or pass --user / --system"
        ))
        .into()),
    }
}

/// Refuse a system-scope operation without privilege.
///
/// `rerun` holds the argv (after `sudo`) that repeats the intended operation,
/// with absolute paths already resolved. Nothing is changed.
///
/// # Errors
///
/// [`CliError::Permission`] with rerun guidance.
pub fn ensure_privilege(scope: ServiceScope, elevated: bool, rerun: &[String]) -> Result<()> {
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
    use std::io::Cursor;

    const USER_AT_TTY: Environment = Environment {
        elevated: false,
        interactive: true,
    };

    fn choose(
        requested: Option<ServiceScope>,
        env: Environment,
        answer: &str,
    ) -> Result<ServiceScope> {
        let mut prompt = Vec::new();
        choose_install_scope(
            requested,
            env,
            &mut Cursor::new(answer.to_owned()),
            &mut prompt,
        )
    }

    #[test]
    fn explicit_scope_is_never_overridden_or_prompted() {
        let env = USER_AT_TTY;
        assert_eq!(
            choose(Some(ServiceScope::System), env, "").unwrap(),
            ServiceScope::System
        );
        assert_eq!(
            choose(Some(ServiceScope::User), env, "").unwrap(),
            ServiceScope::User
        );
    }

    #[test]
    fn elevated_defaults_to_system_without_a_prompt() {
        let env = Environment {
            elevated: true,
            interactive: true,
        };
        let mut prompt = Vec::new();
        let scope =
            choose_install_scope(None, env, &mut Cursor::new(String::new()), &mut prompt).unwrap();
        assert_eq!(scope, ServiceScope::System);
        assert!(prompt.is_empty(), "elevated install must not prompt");
    }

    #[test]
    fn interactive_user_is_offered_both_scopes() {
        let mut prompt = Vec::new();
        let scope = choose_install_scope(
            None,
            USER_AT_TTY,
            &mut Cursor::new("system\n".to_owned()),
            &mut prompt,
        )
        .unwrap();
        let text = String::from_utf8(prompt).unwrap();
        assert_eq!(scope, ServiceScope::System);
        assert!(text.contains("user") && text.contains("system"), "{text}");
    }

    #[test]
    fn answers_map_to_scopes_and_empty_means_user() {
        for (answer, expected) in [
            ("user\n", ServiceScope::User),
            ("U\n", ServiceScope::User),
            ("\n", ServiceScope::User),
            ("s\n", ServiceScope::System),
            (" SYSTEM \n", ServiceScope::System),
        ] {
            assert_eq!(
                choose(None, USER_AT_TTY, answer).unwrap(),
                expected,
                "{answer:?}"
            );
        }
    }

    #[test]
    fn unknown_or_missing_answer_is_an_error_not_a_guess() {
        for answer in ["maybe\n", ""] {
            let err = choose(None, USER_AT_TTY, answer).unwrap_err();
            assert!(
                matches!(err.downcast_ref::<CliError>(), Some(CliError::Config(_))),
                "{answer:?}"
            );
        }
    }

    #[test]
    fn non_interactive_unprivileged_gets_the_user_scope() {
        let env = Environment {
            elevated: false,
            interactive: false,
        };
        assert_eq!(choose(None, env, "").unwrap(), ServiceScope::User);
    }

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

    #[test]
    fn default_scope_follows_privilege() {
        assert_eq!(default_scope(true), ServiceScope::System);
        assert_eq!(default_scope(false), ServiceScope::User);
    }
}
