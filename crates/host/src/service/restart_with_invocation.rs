use super::native::{system_runner, ServiceCall};
use super::types::{Environment, ServiceFailure, ServiceReport, ServiceScope};
use super::verb::Verb;
use crate::role::Role;

/// Perform the operation using the supplied executable identity for guidance.
///
/// # Errors
///
/// A [`ServiceFailure`] when the service manager refuses or fails the operation.
pub fn restart_with_invocation(
    role: Role,
    scope: ServiceScope,
    env: Environment,
    call: &mut ServiceCall,
    invocation: super::ServiceInvocation,
    executable: &std::path::Path,
) -> Result<ServiceReport, ServiceFailure> {
    let mut runner = system_runner;
    super::verb::verb_via_with_invocation(
        Verb::Restart,
        role,
        scope,
        env,
        call,
        &mut runner,
        invocation,
        executable,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_system_refusal_names_executable() {
        let mut notes = |_: &super::super::native::ServiceNote| {};
        let mut call = ServiceCall::capture(&mut notes);
        let env = Environment {
            elevated: false,
            can_answer: false,
        };
        let error = restart_with_invocation(
            Role::Serve,
            ServiceScope::System,
            env,
            &mut call,
            super::super::ServiceInvocation::Standalone,
            std::path::Path::new("/tmp/standalone"),
        )
        .unwrap_err();
        assert!(error
            .error
            .to_string()
            .contains("/tmp/standalone service restart --system"));
    }
}
