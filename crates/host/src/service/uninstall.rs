//! `service uninstall`: stop, disable, and remove the registration.

use super::native::system_runner;
use super::native::ServiceCall;
use super::types::{Environment, ServiceFailure, ServiceReport, ServiceScope};
use super::verb::{verb_via, Verb};
use crate::role::Role;

/// `service uninstall`: stop, disable, and remove the registration.
///
/// # Errors
///
/// A [`ServiceFailure`]; see [`install`].
pub fn uninstall(
    role: Role,
    scope: ServiceScope,
    env: Environment,
    call: &mut ServiceCall,
) -> Result<ServiceReport, ServiceFailure> {
    let mut runner = system_runner;
    verb_via(Verb::Uninstall, role, scope, env, call, &mut runner)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::BATCH;
    use super::super::native::ServiceNote;
    use super::*;
    use crate::error::CliError;

    #[test]
    fn uninstall_is_refused_for_system_scope_without_privilege() {
        let mut notes = |_: &ServiceNote| {};
        let mut call = ServiceCall::capture(&mut notes);
        let result = uninstall(Role::Serve, ServiceScope::System, BATCH, &mut call);
        if !crate::service::is_elevated() {
            let err = anyhow::Error::from(result.unwrap_err());
            assert!(matches!(
                err.downcast_ref::<CliError>(),
                Some(CliError::Permission(_))
            ));
        }
    }
}
