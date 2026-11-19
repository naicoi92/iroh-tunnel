//! `service start`; refused only while the role runs in the same tier.

use super::native::{system_runner, ServiceCall};
use super::types::{Environment, ServiceFailure, ServiceInvocation, ServiceReport};
use super::verb::{verb_via, Verb};
use crate::role::Role;

/// `service start` in the caller's tier; refused while the role already
/// runs in that same tier. A role running only in the other tier is reported
/// as a [`super::ServiceNote::OtherTierRunning`] and does not refuse.
///
/// # Errors
///
/// A [`ServiceFailure`]; [`crate::error::CliError::Conflict`] when the role
/// runs in the caller's tier, [`crate::error::CliError::Permission`] for a
/// non-root caller on a BusyBox host.
pub fn start(
    role: Role,
    invocation: &ServiceInvocation,
    env: Environment,
    call: &mut ServiceCall<'_>,
) -> Result<ServiceReport, ServiceFailure> {
    let mut runner = system_runner;
    verb_via(Verb::Start, role, invocation, env, call, &mut runner)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{commands, run_verb, BATCH};
    use super::super::verb::Verb;
    use crate::error::CliError;

    /// On a BusyBox host every registration runs as root, so a non-root
    /// caller is refused before any command; elsewhere a user-tier start
    /// reaches the manager with no privilege at all.
    #[test]
    fn start_follows_the_callers_tier() {
        if !super::super::platform::system_wide_only() {
            return; // reaching the real user manager is covered by the E2E suites
        }
        let (result, log) = run_verb(Verb::Start, BATCH);
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
