//! Mapping of a scope-prompt answer to a scope.

use anyhow::Result;

use super::types::ServiceScope;
use crate::error::CliError;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
