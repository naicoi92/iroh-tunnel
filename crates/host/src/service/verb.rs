//! Shared mechanics of the uninstall/start/stop/restart operations (private).

use anyhow::Result;

use super::execute::execute;
use super::native::{Native, Runner, ServiceCall, ServiceNote};
use super::policy::{ensure_privilege, ensure_supported};
use super::types::{Environment, ServiceFailure, ServiceReport, ServiceScope};
use crate::role::Role;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Verb {
    Uninstall,
    Start,
    Stop,
    Restart,
}

impl Verb {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Verb::Uninstall => "uninstall",
            Verb::Start => "start",
            Verb::Stop => "stop",
            Verb::Restart => "restart",
        }
    }
}

pub(crate) fn flag(scope: ServiceScope) -> &'static str {
    match scope {
        ServiceScope::System => "--system",
        ServiceScope::User => "--user",
    }
}

/// Platform support and privilege for a verb on an existing service, with the
/// rerun guidance the CLI always printed.
pub(crate) fn check_verb_policy(
    verb: &str,
    role: Role,
    scope: ServiceScope,
    env: Environment,
) -> Result<()> {
    ensure_supported(scope)?;
    let rerun = ["iroh-tunnel", role.name(), "service", verb, flag(scope)].map(str::to_owned);
    ensure_privilege(scope, env.elevated, &rerun)
}

/// [`uninstall`]/[`start`]/[`stop`]/[`restart`] over an injected runner.
pub(crate) fn verb_via(
    verb: Verb,
    role: Role,
    scope: ServiceScope,
    env: Environment,
    call: &mut ServiceCall,
    runner: &mut Runner<'_>,
) -> Result<ServiceReport, ServiceFailure> {
    check_verb_policy(verb.word(), role, scope, env)
        .and_then(|()| {
            if matches!(verb, Verb::Start) {
                // Refuse before the manager: the unit would otherwise start and
                // exit after systemd/launchd already reported success (RO-03).
                crate::ownership::ensure_not_running(role)?;
            }
            Ok(())
        })
        .map_err(ServiceFailure::before_effects)?;
    execute(call, runner, |native, notes| {
        backend_verb(verb, native, notes, role.name(), scope)
    })
}

#[allow(unused_variables)] // unused on targets without a service backend
fn backend_verb(
    verb: Verb,
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use super::backend::{busybox, systemd};
        if super::platform::linux_is_systemd() {
            match verb {
                Verb::Uninstall => systemd::uninstall(native, notes, role, scope),
                Verb::Start => systemd::start(native, role, scope),
                Verb::Stop => systemd::stop(native, notes, role, scope),
                Verb::Restart => systemd::restart(native, notes, role, scope),
            }
        } else {
            match verb {
                Verb::Uninstall => busybox::uninstall(native, notes, role, scope),
                Verb::Start => busybox::start(native, role, scope),
                Verb::Stop => busybox::stop(native, notes, role, scope),
                Verb::Restart => busybox::restart(native, notes, role, scope),
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        use super::backend::launchd;
        match verb {
            Verb::Uninstall => launchd::uninstall(native, notes, role, scope),
            Verb::Start => launchd::start(native, role, scope),
            Verb::Stop => launchd::stop(native, notes, role, scope),
            Verb::Restart => launchd::restart(native, notes, role, scope),
        }
    }
    #[cfg(windows)]
    {
        use super::backend::windows;
        match verb {
            Verb::Uninstall => windows::uninstall(native, notes, role, scope),
            Verb::Start => windows::start(native, role, scope),
            Verb::Stop => windows::stop(native, notes, role, scope),
            Verb::Restart => windows::restart(native, notes, role, scope),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        super::platform::unsupported()
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{blocking_runtime, ok_output, BATCH};
    #[cfg(unix)]
    use super::super::fixtures::{commands, in_isolated_home};
    #[cfg(unix)]
    use super::super::native::fake::{Events, FakeRunner};
    use super::super::native::ServiceCall;
    use super::super::native::ServiceNote;
    use super::super::status::status_via;
    use super::*;
    #[cfg(unix)]
    use crate::error::CliError;

    #[cfg(unix)]
    #[test]
    fn start_is_refused_before_manager_when_role_runs() {
        const NAME: &str = "service::verb::tests::start_is_refused_before_manager_when_role_runs";
        in_isolated_home(NAME, |_| {
            let _claim = crate::ownership::claim(Role::Serve).unwrap();
            let events = Events::default();
            let mut fake = FakeRunner::new(&events, vec![]);
            let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
            let mut notes = crate::service::native::fake::note_sink(&events);
            let mut call = ServiceCall::capture(&mut notes);
            let result = verb_via(
                Verb::Start,
                Role::Serve,
                ServiceScope::User,
                BATCH,
                &mut call,
                &mut runner,
            );
            let err = anyhow::Error::from(result.expect_err("must be refused"));
            assert!(
                matches!(err.downcast_ref::<CliError>(), Some(CliError::Conflict(_))),
                "{err:?}"
            );
            assert!(commands(&events.borrow()).is_empty());
        });
    }

    #[test]
    fn service_operation_runs_inside_spawn_blocking() {
        let report = blocking_runtime().block_on(async {
            tokio::task::spawn_blocking(|| {
                let mut runner = |c: &mut std::process::Command, _| {
                    #[allow(unused_mut)] // only Windows fills the listing
                    let mut out = ok_output(c.get_program().to_string_lossy().into_owned());
                    // Windows status first lists the tasks and refuses an
                    // unregistered role, so the listing must name the task.
                    #[cfg(windows)]
                    {
                        out.stdout = format!(
                            "\"\\iroh-tunnel-serve{}\",\"N/A\",\"Ready\"\n",
                            super::super::registration::instance_suffix()
                        )
                        .into_bytes();
                    }
                    Ok(out)
                };
                let mut notes = |_: &ServiceNote| {};
                let mut call = ServiceCall::capture(&mut notes);
                status_via(
                    Role::Serve,
                    ServiceScope::User,
                    BATCH,
                    &mut call,
                    &mut runner,
                )
            })
            .await
        });
        assert!(report.expect("no panic on the blocking worker").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn dropped_awaiter_does_not_stop_service() {
        const NAME: &str = "service::verb::tests::dropped_awaiter_does_not_stop_service";
        in_isolated_home(NAME, |_| {
            use std::sync::{Arc, Mutex};
            let calls: Arc<Mutex<Vec<String>>> = Arc::default();
            let recorded = Arc::clone(&calls);
            blocking_runtime().block_on(async {
                let handle = tokio::task::spawn_blocking(move || {
                    let mut runner = |c: &mut std::process::Command, _| {
                        let args: Vec<String> = c
                            .get_args()
                            .map(|a| a.to_string_lossy().into_owned())
                            .collect();
                        recorded.lock().unwrap().push(args.join(" "));
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        Ok(ok_output(String::new()))
                    };
                    let mut notes = |_: &ServiceNote| {};
                    let mut call = ServiceCall::capture(&mut notes);
                    verb_via(
                        Verb::Start,
                        Role::Serve,
                        ServiceScope::User,
                        BATCH,
                        &mut call,
                        &mut runner,
                    )
                });
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                drop(handle);
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            });
            let calls = calls.lock().unwrap().clone();
            assert_eq!(calls.len(), 1, "{calls:?}");
            assert!(calls[0].contains("start"), "{calls:?}");
        });
    }
}
