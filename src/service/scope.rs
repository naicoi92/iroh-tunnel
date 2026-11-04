//! Service scope selection and privilege checks (SM-01, SM-03; issue #90).
//!
//! - A non-elevated user chooses user or system; elevated (root /
//!   Administrator) installs default to system without a prompt.
//! - Choosing system without privilege stops with rerun guidance that keeps
//!   the intended arguments and absolute paths. The process never elevates
//!   itself and never falls back to a user service.

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
    pub can_answer: bool,
}

impl Environment {
    /// Probe the running process.
    pub fn detect() -> Self {
        Self {
            elevated: is_elevated(),
            can_answer: is_interactive(),
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

/// Refuse a scope this platform has no backend for.
///
/// Windows has only the user-background task: a Windows Service is deferred
/// (SM-04), so `--system` is reported as unsupported rather than as a missing
/// privilege that elevation could not fix.
///
/// # Errors
///
/// [`CliError::Service`] for system scope on Windows; nothing is changed.
pub fn ensure_supported(scope: ServiceScope) -> Result<()> {
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

/// Scope for verbs that act on an existing service when no flag is given:
/// the elevated installer owns a system service, everyone else a user one.
pub fn default_scope(elevated: bool) -> ServiceScope {
    if elevated {
        ServiceScope::System
    } else {
        ServiceScope::User
    }
}

/// Outcome of the pure install-scope policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeDecision {
    Chosen(ServiceScope),
    /// A person must pick user or system.
    NeedsChoice,
}

/// Decide the install scope without any I/O.
///
/// An explicit request wins. Otherwise an elevated process installs a system
/// service without asking; a non-elevated process that can answer must choose;
/// a non-elevated process that cannot answer gets the unprivileged user scope.
pub fn decide_install_scope(requested: Option<ServiceScope>, env: Environment) -> ScopeDecision {
    if let Some(scope) = requested {
        return ScopeDecision::Chosen(scope);
    }
    if env.elevated {
        return ScopeDecision::Chosen(ServiceScope::System);
    }
    if env.can_answer {
        ScopeDecision::NeedsChoice
    } else {
        ScopeDecision::Chosen(ServiceScope::User)
    }
}

/// Map a scope-prompt answer to a scope; empty means user.
///
/// # Errors
///
/// [`CliError::Config`] for an unrecognised answer.
pub fn parse_scope_answer(answer: &str) -> Result<ServiceScope> {
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

    const USER_AT_TTY: Environment = Environment {
        elevated: false,
        can_answer: true,
    };

    #[test]
    fn install_scope_needs_choice_when_caller_can_answer() {
        for (requested, elevated, can_answer, expected) in [
            (None, false, true, ScopeDecision::NeedsChoice),
            (
                None,
                false,
                false,
                ScopeDecision::Chosen(ServiceScope::User),
            ),
            (
                None,
                true,
                true,
                ScopeDecision::Chosen(ServiceScope::System),
            ),
            (
                None,
                true,
                false,
                ScopeDecision::Chosen(ServiceScope::System),
            ),
            (
                Some(ServiceScope::User),
                true,
                true,
                ScopeDecision::Chosen(ServiceScope::User),
            ),
            (
                Some(ServiceScope::System),
                false,
                true,
                ScopeDecision::Chosen(ServiceScope::System),
            ),
        ] {
            let env = Environment {
                elevated,
                can_answer,
            };
            assert_eq!(decide_install_scope(requested, env), expected);
        }
        assert_eq!(parse_scope_answer("s\n").unwrap(), ServiceScope::System);
        assert!(parse_scope_answer("maybe").is_err());
    }

    #[test]
    fn explicit_scope_is_never_overridden_or_asked() {
        for scope in [ServiceScope::System, ServiceScope::User] {
            assert_eq!(
                decide_install_scope(Some(scope), USER_AT_TTY),
                ScopeDecision::Chosen(scope)
            );
        }
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
            assert_eq!(parse_scope_answer(answer).unwrap(), expected, "{answer:?}");
        }
    }

    #[test]
    fn unknown_answer_is_a_config_error_not_a_guess() {
        let err = parse_scope_answer("maybe\n").unwrap_err();
        assert!(matches!(
            err.downcast_ref::<CliError>(),
            Some(CliError::Config(_))
        ));
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
