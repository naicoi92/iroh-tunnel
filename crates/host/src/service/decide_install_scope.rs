//! The pure install-scope policy.

use super::types::{Environment, ScopeDecision, ServiceScope};

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

#[cfg(test)]
mod tests {
    use super::super::parse_scope_answer::parse_scope_answer;
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
}
