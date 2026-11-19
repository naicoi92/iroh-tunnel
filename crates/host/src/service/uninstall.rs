//! `service uninstall`: stop, disable, and remove the registration.

use super::native::{system_runner, ServiceCall};
use super::types::{Environment, ServiceFailure, ServiceInvocation, ServiceReport};
use super::verb::{verb_via, Verb};
use crate::role::Role;

/// `service uninstall` in the caller's tier: stop, disable, and remove the
/// registration.
///
/// # Errors
///
/// A [`ServiceFailure`]; see [`super::install`].
pub fn uninstall(
    role: Role,
    invocation: &ServiceInvocation,
    env: Environment,
    call: &mut ServiceCall<'_>,
) -> Result<ServiceReport, ServiceFailure> {
    let mut runner = system_runner;
    verb_via(Verb::Uninstall, role, invocation, env, call, &mut runner)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{commands, run_verb, BATCH};
    use super::super::verb::Verb;
    use crate::error::CliError;

    /// On a BusyBox host every registration runs as root, so a non-root
    /// caller is refused before any command; elsewhere a user-tier uninstall
    /// reaches the manager with no privilege at all.
    #[test]
    fn uninstall_follows_the_callers_tier() {
        if !super::super::platform::system_wide_only() {
            return; // reaching the real user manager is covered by the E2E suites
        }
        let (result, log) = run_verb(Verb::Uninstall, BATCH);
        let err = anyhow::Error::from(result.expect_err("non-root BusyBox is refused"));
        assert!(
            matches!(
                err.downcast_ref::<CliError>(),
                Some(CliError::Permission(_))
            ),
            "{err:?}"
        );
        assert!(commands(&log).is_empty(), "{log:?}");
    }
}
