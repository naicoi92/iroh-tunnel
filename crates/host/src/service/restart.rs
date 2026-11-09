//! `service restart`.

use super::native::system_runner;
use super::native::ServiceCall;
use super::types::{Environment, ServiceFailure, ServiceReport, ServiceScope};
use super::verb::{verb_via, Verb};
use crate::role::Role;

/// `service restart`.
///
/// # Errors
///
/// A [`ServiceFailure`]; see [`super::install`].
pub fn restart(
    role: Role,
    scope: ServiceScope,
    env: Environment,
    call: &mut ServiceCall,
) -> Result<ServiceReport, ServiceFailure> {
    let mut runner = system_runner;
    verb_via(Verb::Restart, role, scope, env, call, &mut runner)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::BATCH;
    use super::super::native::ServiceNote;
    use super::*;
    use crate::error::CliError;

    #[test]
    fn restart_is_refused_for_system_scope_without_privilege() {
        // An elevated run passes the policy and would act on a real service
        // (systemd, launchd or the SCM); only the unprivileged refusal is safe
        // to exercise.
        if crate::service::is_elevated() {
            return;
        }
        let mut notes = |_: &ServiceNote| {};
        let mut call = ServiceCall::capture(&mut notes);
        let result = restart(Role::Serve, ServiceScope::System, BATCH, &mut call);
        let err = anyhow::Error::from(result.unwrap_err());
        assert!(matches!(
            err.downcast_ref::<CliError>(),
            Some(CliError::Permission(_))
        ));
    }
}
