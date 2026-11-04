//! `service status`: registration, liveness and owner facts plus the native
//! tool's output.

use anyhow::Result;

use super::native::{system_runner, Native, NativeOutput, Runner, ServiceCall, ServiceNote};
use super::types::{
    Environment, Fact, Registration, ServiceFailure, ServiceScope, ServiceStatusReport,
};
use super::verb::check_verb_policy;
use crate::role::Role;

/// `service status`: the registration, liveness and owner facts, plus the
/// native tool's output (inherited by the terminal in
/// [`OutputMode::Inherit`](super::OutputMode::Inherit), returned otherwise).
///
/// # Errors
///
/// A [`ServiceFailure`] for policy refusals and when the native tool cannot
/// run. A fact that cannot be read is [`Fact::Unavailable`], not an error.
pub fn status(
    role: Role,
    scope: ServiceScope,
    env: Environment,
    call: &mut ServiceCall,
) -> Result<ServiceStatusReport, ServiceFailure> {
    let mut runner = system_runner;
    status_via(role, scope, env, call, &mut runner)
}

/// [`status`] over an injected command runner.
pub(crate) fn status_via(
    role: Role,
    scope: ServiceScope,
    env: Environment,
    call: &mut ServiceCall,
    runner: &mut Runner<'_>,
) -> Result<ServiceStatusReport, ServiceFailure> {
    check_verb_policy("status", role, scope, env).map_err(ServiceFailure::before_effects)?;
    let liveness = crate::ownership::probe(role);
    let owner = match &liveness {
        Ok(crate::ownership::Liveness::Running {
            owner: Some(owner), ..
        }) => Fact::Known(owner.clone()),
        Ok(crate::ownership::Liveness::Running { owner: None, .. }) => {
            Fact::Unavailable("the owner record is missing or unreadable".to_owned())
        }
        Ok(crate::ownership::Liveness::NotRunning) => {
            Fact::Unavailable(format!("{} is not running", role.name()))
        }
        Err(error) => Fact::Unavailable(error.to_string()),
    };
    let mut native = Native::new(call.output, runner);
    let output = backend_status(&mut native, &mut *call.notes, role.name(), scope);
    let (applied, outputs) = native.finish();
    let output = output.map_err(|error| ServiceFailure {
        applied,
        native: outputs,
        error,
    })?;
    // On Windows the task backend refuses an unregistered task, so reaching
    // here means it exists.
    #[cfg(windows)]
    let registration = Fact::Known(Registration::Present {
        location: "scheduled task".to_owned(),
    });
    #[cfg(not(windows))]
    let registration = Fact::from_result(registration_of(role.name(), scope));
    Ok(ServiceStatusReport {
        registration,
        liveness: Fact::from_result(liveness),
        owner,
        native: Some(output),
    })
}

/// Whether the platform's registration file exists (Unix backends).
#[cfg(not(windows))]
#[allow(unused_variables)] // unused on targets without a service backend
fn registration_of(role: &str, scope: ServiceScope) -> Result<Registration> {
    #[cfg(target_os = "linux")]
    let path = if super::platform::linux_is_systemd() {
        super::backend::systemd::registration_path(role, scope)?
    } else {
        super::backend::busybox::registration_path(role)
    };
    #[cfg(target_os = "macos")]
    let path = super::backend::launchd::registration_path(role, scope)?;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let path = std::path::PathBuf::new();
    Ok(if path.exists() {
        Registration::Present {
            location: path.display().to_string(),
        }
    } else {
        Registration::Absent
    })
}

#[allow(unused_variables)] // unused on targets without a service backend
fn backend_status(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
) -> Result<NativeOutput> {
    #[cfg(target_os = "linux")]
    {
        if super::platform::linux_is_systemd() {
            super::backend::systemd::status(native, notes, role, scope)
        } else {
            super::backend::busybox::status(native, notes, role, scope)
        }
    }
    #[cfg(target_os = "macos")]
    {
        super::backend::launchd::status(native, notes, role, scope)
    }
    #[cfg(windows)]
    {
        super::backend::windows::status(native, notes, role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        super::platform::unsupported()
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{in_isolated_home, run_status};
    use super::*;

    #[cfg(unix)]
    #[test]
    fn status_reports_foreground_role_without_registration() {
        const NAME: &str =
            "service::status::tests::status_reports_foreground_role_without_registration";
        in_isolated_home(NAME, |_| {
            let _claim = crate::ownership::claim(Role::Serve).unwrap();
            let report = run_status(vec![]).unwrap();
            assert!(matches!(
                report.registration,
                Fact::Known(Registration::Absent)
            ));
            assert!(matches!(
                report.liveness,
                Fact::Known(crate::ownership::Liveness::Running { .. })
            ));
        });
    }

    #[cfg(unix)]
    #[test]
    fn status_reports_missing_owner_as_unavailable() {
        const NAME: &str = "service::status::tests::status_reports_missing_owner_as_unavailable";
        in_isolated_home(NAME, |home| {
            let _claim = crate::ownership::claim(Role::Serve).unwrap();
            std::fs::remove_file(home.join(".local/state/iroh-tunnel/serve.owner.json")).unwrap();
            let report = run_status(vec![]).expect("a missing owner is not a failure");
            assert!(matches!(report.owner, Fact::Unavailable(_)), "{report:?}");
            assert!(matches!(
                report.liveness,
                Fact::Known(crate::ownership::Liveness::Running { .. })
            ));
        });
    }

    #[cfg(unix)]
    #[test]
    fn status_does_not_parse_native_prose() {
        const NAME: &str = "service::status::tests::status_does_not_parse_native_prose";
        in_isolated_home(NAME, |_| {
            // Nothing runs the role, whatever the localized native text says.
            let prose = crate::service::native::fake::output(0, "aktiv (läuft)\n", "");
            let report = run_status(vec![prose]).unwrap();
            assert!(matches!(
                report.liveness,
                Fact::Known(crate::ownership::Liveness::NotRunning)
            ));
            let native = report.native.expect("native output is returned");
            assert_eq!(native.stdout, "aktiv (läuft)\n".as_bytes());
        });
    }
}
