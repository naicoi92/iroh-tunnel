//! Shared mechanics of the uninstall/start/stop/restart operations (private).

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::Path;

use anyhow::Result;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::effective_binary::EffectiveBinary;
use super::execute::execute;
use super::native::{Native, Runner, ServiceCall, ServiceNote};
use super::platform;
use super::policy::{ensure_privilege, ensure_supported};
use super::service_binary::release_unreferenced_copy;
use super::types::{AppliedEffect, Environment, ServiceFailure, ServiceReport, ServiceScope};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::error::CliError;
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
    let root_run = platform::root_run(scope, platform::system_wide_only());
    execute(call, runner, |native, notes| {
        verb_steps(
            native,
            notes,
            verb,
            |native| guard_root_run(native, role.name(), root_run),
            |native, notes| backend_verb(verb, native, notes, role, scope),
            |native| release_unreferenced_copy(native, role.name(), root_run),
        )
    })
}

/// Run one verb's backend step between its binary guard and its cleanup.
///
/// `guard` runs before `backend` for [`Verb::Start`] and [`Verb::Restart`];
/// when it refuses, no lifecycle command is issued. After a successful
/// [`Verb::Uninstall`] backend, `release_after_uninstall` removes the role's
/// managed copy and its effect is recorded.
///
/// # Errors
///
/// The first failing step's error; effects already recorded (such as the
/// removed registration before a failed cleanup) stay in `native`.
fn verb_steps(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    verb: Verb,
    guard: impl FnOnce(&mut Native) -> Result<()>,
    backend: impl FnOnce(&mut Native, &mut dyn FnMut(&ServiceNote)) -> Result<()>,
    release_after_uninstall: impl FnOnce(&mut Native) -> Result<Option<AppliedEffect>>,
) -> Result<()> {
    if matches!(verb, Verb::Start | Verb::Restart) {
        guard(native)?;
    }
    backend(native, notes)?;
    if matches!(verb, Verb::Uninstall) {
        if let Some(effect) = release_after_uninstall(native)? {
            native.applied(effect);
        }
    }
    Ok(())
}

/// Refuse to start a root-run service whose effective binary is not
/// root-only writable. Registrations that do not run as root are not checked.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn guard_root_run(native: &mut Native, role: &str, root_run: bool) -> Result<()> {
    if !root_run {
        return Ok(());
    }
    guard_effective_binary(native, role, &|path| {
        super::trust::verify_binary(path, super::trust::TrustedOwners::RootOnly)
    })
}

/// No binary guard off Linux and macOS.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn guard_root_run(_native: &mut Native, _role: &str, _root_run: bool) -> Result<()> {
    Ok(())
}

/// Ask the service manager which binary it will run for `role` (a read-only
/// query) and check it with `verify`.
///
/// # Errors
///
/// See [`check_effective`].
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn guard_effective_binary(
    native: &mut Native,
    role: &str,
    verify: &dyn Fn(&Path) -> Result<()>,
) -> Result<()> {
    check_effective(platform::effective_binary(native, role), role, verify)
}

/// Accept a definition the manager has not loaded (the backend verb reports
/// it as before) and a binary `verify` accepts.
///
/// # Errors
///
/// [`CliError::Permission`] keeping `verify`'s reason (path, owner, mode),
/// or naming why the definition is unreadable, with reinstall guidance.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn check_effective(
    loaded: EffectiveBinary,
    role: &str,
    verify: &dyn Fn(&Path) -> Result<()>,
) -> Result<()> {
    let reason = match loaded {
        EffectiveBinary::NotLoaded => return Ok(()),
        EffectiveBinary::Binary(path) => match verify(&path) {
            Ok(()) => return Ok(()),
            Err(err) => match err.downcast_ref::<CliError>() {
                Some(CliError::Permission(msg)) => format!(
                    "the registered service binary {} is not safe to run as root: {msg}",
                    path.display()
                ),
                _ => format!(
                    "cannot check the registered service binary {}: {err:#}",
                    path.display()
                ),
            },
        },
        EffectiveBinary::Unknown(why) => format!(
            "cannot tell which binary the {role} service runs ({why}); refusing to start it"
        ),
    };
    Err(CliError::Permission(format!(
        "{reason}. To register a root-owned binary, re-run \
         `sudo iroh-tunnel {role} service install --system`."
    ))
    .into())
}

#[allow(unused_variables)] // unused on targets without a service backend
fn backend_verb(
    verb: Verb,
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role_id: Role,
    scope: ServiceScope,
) -> Result<()> {
    let role = role_id.name();
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
        if scope == ServiceScope::System {
            return super::system_verb::system_verb(verb, role_id, notes);
        }
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

#[cfg(test)]
mod steps_tests {
    //! [`verb_steps`] ordering and the start/restart binary guard.

    use std::cell::RefCell;

    use super::super::execute::execute;
    use super::super::native::fake::{Events, FakeRunner};
    use super::super::native::{Native, OutputMode, ServiceCall, ServiceNote};
    use super::super::types::AppliedEffect;
    use super::*;

    /// The registration path a recording uninstall backend removes.
    const UNIT: &str = "/etc/systemd/system/iroh-tunnel-serve.service";
    /// The managed copy a recording release removes.
    const COPY: &str = "/m/iroh-tunnel/bin/serve/iroh-tunnel";

    /// A backend that runs one recorded `svc <verb>` command and records
    /// the effect the real backend would.
    fn recording_backend(verb: Verb, native: &mut Native) -> Result<()> {
        native.run(std::process::Command::new("svc").arg(verb.word()))?;
        native.applied(match verb {
            Verb::Uninstall => AppliedEffect::RegistrationRemoved(UNIT.into()),
            Verb::Stop => AppliedEffect::Stopped,
            Verb::Start | Verb::Restart => AppliedEffect::Started,
        });
        Ok(())
    }

    /// What one [`verb_steps`] run did.
    struct StepsRun {
        result: Result<()>,
        order: Vec<String>,
        commands: Vec<String>,
        applied: Vec<AppliedEffect>,
    }

    /// Run [`verb_steps`] for `verb` with a guard returning `guard`, the
    /// recording backend and a release returning `release`.
    fn run_steps(
        verb: Verb,
        guard: Result<(), &str>,
        release: Result<Option<AppliedEffect>, &str>,
    ) -> StepsRun {
        let order = RefCell::new(Vec::new());
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, Vec::new());
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Capture, &mut runner);
        let result = verb_steps(
            &mut native,
            &mut |_: &ServiceNote| {},
            verb,
            |_| {
                order.borrow_mut().push("guard".to_owned());
                guard.map_err(|why| CliError::Permission(why.into()).into())
            },
            |native, _| {
                order.borrow_mut().push("backend".to_owned());
                recording_backend(verb, native)
            },
            |_| {
                order.borrow_mut().push("release".to_owned());
                release.map_err(|why| CliError::Permission(why.into()).into())
            },
        );
        let (applied, _) = native.finish();
        StepsRun {
            result,
            order: order.into_inner(),
            commands: command_lines(&events),
            applied,
        }
    }

    fn command_lines(events: &Events) -> Vec<String> {
        commands(&events.borrow())
            .into_iter()
            .map(|event| match event {
                super::super::native::fake::Event::Call { program, args, .. } => {
                    std::iter::once(program.clone())
                        .chain(args.iter().cloned())
                        .collect::<Vec<_>>()
                        .join(" ")
                }
                super::super::native::fake::Event::Note(_) => unreachable!(),
            })
            .collect()
    }

    use super::super::fixtures::commands;
    use crate::error::CliError;

    #[test]
    fn start_checks_then_starts() {
        let run = run_steps(Verb::Start, Ok(()), Ok(None));
        run.result.unwrap();
        assert_eq!(run.order, ["guard", "backend"]);
        assert_eq!(run.commands, ["svc start"]);
        assert_eq!(run.applied, [AppliedEffect::Started]);
    }

    #[test]
    fn guard_runs_before_backend_and_blocks_it() {
        for verb in [Verb::Start, Verb::Restart] {
            let run = run_steps(verb, Err("unsafe binary"), Ok(None));
            let err = run.result.expect_err("a guard refusal must fail the verb");
            assert!(
                matches!(
                    err.downcast_ref::<CliError>(),
                    Some(CliError::Permission(_))
                ),
                "{err:?}"
            );
            assert_eq!(run.order, ["guard"], "{}", verb.word());
            assert!(run.commands.is_empty(), "{:?}", run.commands);
            assert!(run.applied.is_empty(), "{:?}", run.applied);
        }
    }

    #[test]
    fn stop_and_uninstall_never_run_the_guard() {
        for verb in [Verb::Stop, Verb::Uninstall] {
            let run = run_steps(verb, Err("must not run"), Ok(None));
            run.result.unwrap();
            assert!(!run.order.iter().any(|s| s == "guard"), "{:?}", run.order);
        }
    }

    #[test]
    fn uninstall_releases_copy_after_registration_removed() {
        let removed = AppliedEffect::ManagedBinaryRemoved(COPY.into());
        let run = run_steps(Verb::Uninstall, Ok(()), Ok(Some(removed.clone())));
        run.result.unwrap();
        assert_eq!(run.order, ["backend", "release"]);
        assert_eq!(
            run.applied,
            [AppliedEffect::RegistrationRemoved(UNIT.into()), removed]
        );
    }

    #[test]
    fn only_uninstall_releases_the_copy() {
        for verb in [Verb::Start, Verb::Stop, Verb::Restart] {
            let run = run_steps(verb, Ok(()), Err("must not run"));
            run.result.unwrap();
            assert!(!run.order.iter().any(|s| s == "release"), "{:?}", run.order);
        }
    }

    #[test]
    fn uninstall_cleanup_failure_keeps_registration_removed_in_applied() {
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, Vec::new());
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut notes = |_: &ServiceNote| {};
        let mut call = ServiceCall::capture(&mut notes);
        let failure = execute(&mut call, &mut runner, |native, notes| {
            verb_steps(
                native,
                notes,
                Verb::Uninstall,
                |_| panic!("uninstall has no guard"),
                |native, _| recording_backend(Verb::Uninstall, native),
                |_| Err(CliError::Permission("unsafe directory /m/iroh-tunnel/bin".into()).into()),
            )
        })
        .expect_err("a cleanup failure must fail the uninstall");
        assert_eq!(
            failure.applied,
            [AppliedEffect::RegistrationRemoved(UNIT.into())]
        );
        assert!(
            matches!(
                failure.error.downcast_ref::<CliError>(),
                Some(CliError::Permission(msg)) if msg.contains("/m/iroh-tunnel/bin")
            ),
            "{:?}",
            failure.error
        );
    }

    #[test]
    fn non_root_run_uninstall_releases_nothing_and_queries_nothing() {
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, Vec::new());
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Capture, &mut runner);
        assert_eq!(
            release_unreferenced_copy(&mut native, "serve", false).unwrap(),
            None
        );
        assert!(command_lines(&events).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn user_scope_start_skips_guard() {
        const NAME: &str = "service::verb::steps_tests::user_scope_start_skips_guard";
        super::super::fixtures::in_isolated_home(NAME, |_| {
            if super::super::platform::root_run(
                ServiceScope::User,
                super::super::platform::system_wide_only(),
            ) {
                // BusyBox init: a user-scope request is root-run and guarded.
                return;
            }
            let events = Events::default();
            let mut fake = FakeRunner::new(&events, vec![]);
            let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
            let mut notes = |_: &ServiceNote| {};
            let mut call = ServiceCall::capture(&mut notes);
            verb_via(
                Verb::Start,
                Role::Serve,
                ServiceScope::User,
                super::super::fixtures::BATCH,
                &mut call,
                &mut runner,
            )
            .unwrap();
            let lines = command_lines(&events);
            assert_eq!(lines.len(), 1, "{lines:?}");
            assert!(
                !lines[0].contains(" print ") && !lines[0].contains(" show "),
                "{lines:?}"
            );
        });
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod guard_tests {
    //! The start/restart guard over the effective binary (Linux, macOS).

    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use super::super::effective_binary::EffectiveBinary;
    use super::super::fixtures::commands;
    use super::super::native::fake::{output, Events, FakeRunner};
    use super::super::native::{Native, OutputMode, ServiceNote};
    use super::super::trust::{verify_binary, TrustedOwners};
    use super::super::types::AppliedEffect;
    use super::*;
    use crate::error::CliError;

    fn me() -> u32 {
        // SAFETY: geteuid(2) has no preconditions and cannot fail.
        unsafe { libc::geteuid() }
    }

    /// A query answer both the systemd and launchd parsers read as
    /// `Binary(path)`.
    fn loaded(path: &Path) -> String {
        let p = path.display();
        format!(
            "LoadState=loaded\nExecStart={{ path={p} ; argv[]={p} ; ignore_errors=no }}\n\
             program = {p}\n"
        )
    }

    fn permission_text(err: &anyhow::Error) -> String {
        match err.downcast_ref::<CliError>() {
            Some(CliError::Permission(msg)) => msg.clone(),
            _ => panic!("expected CliError::Permission, got {err:?}"),
        }
    }

    fn refuse_unsafe(path: &Path) -> Result<()> {
        Err(CliError::Permission(format!(
            "unsafe file: {} (owner uid 501, mode 0775)",
            path.display()
        ))
        .into())
    }

    /// Run `verb` through [`verb_steps`] with the guard over `loaded` and
    /// `verify`; returns the result and the lifecycle commands the backend
    /// issued.
    fn guarded(
        verb: Verb,
        loaded: EffectiveBinary,
        verify: &dyn Fn(&Path) -> Result<()>,
    ) -> (Result<()>, Vec<String>) {
        let issued = std::cell::RefCell::new(Vec::new());
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, Vec::new());
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Capture, &mut runner);
        let result = verb_steps(
            &mut native,
            &mut |_: &ServiceNote| {},
            verb,
            |_| check_effective(loaded, "serve", verify),
            |_, _| {
                issued.borrow_mut().push(verb.word().to_owned());
                Ok(())
            },
            |_| Ok(None),
        );
        (result, issued.into_inner())
    }

    #[test]
    fn start_refuses_unsafe_effective_binary_before_lifecycle() {
        let path = PathBuf::from("/home/alice/.cargo/bin/iroh-tunnel");
        let (result, started) = guarded(
            Verb::Start,
            EffectiveBinary::Binary(path.clone()),
            &refuse_unsafe,
        );
        let msg = permission_text(&result.expect_err("an unsafe binary must be refused"));
        assert!(msg.contains(path.to_str().unwrap()), "{msg}");
        assert!(msg.contains("owner uid 501, mode 0775"), "{msg}");
        assert!(
            msg.contains("sudo iroh-tunnel serve service install --system"),
            "{msg}"
        );
        assert!(started.is_empty(), "{started:?}");
    }

    #[test]
    fn restart_refusal_does_not_stop_the_running_service() {
        let path = PathBuf::from("/home/alice/.cargo/bin/iroh-tunnel");
        let (result, restarted) = guarded(
            Verb::Restart,
            EffectiveBinary::Binary(path.clone()),
            &refuse_unsafe,
        );
        let msg = permission_text(&result.expect_err("an unsafe binary must be refused"));
        assert!(msg.contains(path.to_str().unwrap()), "{msg}");
        assert!(msg.contains("service install --system"), "{msg}");
        assert!(restarted.is_empty(), "{restarted:?}");
    }

    /// Drive `verb` as root-run System scope through [`verb_via`] against a
    /// manager that reports an effective binary owned by the (non-root) test
    /// user, which `RootOnly` refuses. Returns the failure and the lines of
    /// every command issued.
    fn root_run_refused(name: &str, verb: Verb) -> Option<(ServiceFailure, Vec<String>, PathBuf)> {
        #[cfg(target_os = "linux")]
        if !super::super::platform::linux_is_systemd() {
            // BusyBox reads the init script from disk; no query to fake.
            return None;
        }
        if me() == 0 {
            // Root owns the file, so RootOnly would accept it.
            return None;
        }
        let out = std::cell::RefCell::new(None);
        super::super::fixtures::in_isolated_home(name, |home| {
            let path = home.canonicalize().unwrap().join("iroh-tunnel");
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            let events = Events::default();
            let mut fake = FakeRunner::new(&events, vec![output(0, &loaded(&path), "")]);
            let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
            let mut notes = |_: &ServiceNote| {};
            let mut call = super::super::native::ServiceCall::capture(&mut notes);
            let env = Environment {
                elevated: true,
                can_answer: false,
            };
            let failure = verb_via(
                verb,
                Role::Serve,
                ServiceScope::System,
                env,
                &mut call,
                &mut runner,
            )
            .expect_err("an unsafe effective binary must be refused");
            let lines = events
                .borrow()
                .iter()
                .filter_map(|e| match e {
                    super::super::native::fake::Event::Call { program, args, .. } => Some(
                        std::iter::once(program.clone())
                            .chain(args.iter().cloned())
                            .collect::<Vec<_>>()
                            .join(" "),
                    ),
                    super::super::native::fake::Event::Note(_) => None,
                })
                .collect::<Vec<_>>();
            *out.borrow_mut() = Some((failure, lines, path));
        });
        out.into_inner()
    }

    fn assert_refused_before_lifecycle(verb: Verb, name: &str) {
        let Some((failure, lines, path)) = root_run_refused(name, verb) else {
            return;
        };
        let msg = permission_text(&failure.error);
        // RootOnly refuses the first untrusted component, an ancestor of the
        // file; the refusal still names the registered binary itself.
        assert!(msg.contains(path.to_str().unwrap()), "{msg}");
        assert!(msg.contains("service install --system"), "{msg}");
        assert!(failure.applied.is_empty(), "{:?}", failure.applied);
        assert_eq!(
            lines.len(),
            1,
            "only the read-only query may run: {lines:?}"
        );
        assert!(
            lines[0].contains(" show ") || lines[0].contains(" print "),
            "{lines:?}"
        );
        for word in [
            "start",
            "restart",
            "kickstart",
            "stop",
            "bootstrap",
            "bootout",
        ] {
            assert!(
                !lines[0].split(' ').any(|w| w == word),
                "lifecycle command `{word}` issued: {lines:?}"
            );
        }
    }

    #[test]
    fn root_run_start_through_verb_via_is_refused_before_lifecycle() {
        assert_refused_before_lifecycle(
            Verb::Start,
            "service::verb::guard_tests::root_run_start_through_verb_via_is_refused_before_lifecycle",
        );
    }

    #[test]
    fn root_run_restart_through_verb_via_is_refused_before_lifecycle() {
        assert_refused_before_lifecycle(
            Verb::Restart,
            "service::verb::guard_tests::root_run_restart_through_verb_via_is_refused_before_lifecycle",
        );
    }

    #[test]
    fn trusted_effective_binary_is_started() {
        let path = PathBuf::from("/usr/local/bin/iroh-tunnel");
        let (result, started) = guarded(Verb::Start, EffectiveBinary::Binary(path), &|_| Ok(()));
        result.unwrap();
        assert_eq!(started, ["start"]);
    }

    #[test]
    fn not_loaded_falls_through_to_backend() {
        let (result, started) = guarded(Verb::Start, EffectiveBinary::NotLoaded, &|p| {
            panic!("nothing to verify, got {}", p.display())
        });
        result.unwrap();
        assert_eq!(started, ["start"]);
    }

    #[test]
    fn unknown_definition_is_permission() {
        let (result, restarted) = guarded(
            Verb::Restart,
            EffectiveBinary::Unknown("the unit has two ExecStart commands".into()),
            &|p| panic!("nothing to verify, got {}", p.display()),
        );
        let msg = permission_text(&result.expect_err("an unreadable definition must refuse"));
        assert!(msg.contains("the unit has two ExecStart commands"), "{msg}");
        assert!(
            msg.contains("sudo iroh-tunnel serve service install --system"),
            "{msg}"
        );
        assert!(restarted.is_empty(), "{restarted:?}");
    }

    #[test]
    fn directory_loosened_names_the_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let dir = root.join("bin");
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("iroh-tunnel");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        for (path, mode) in [(&root, 0o755), (&dir, 0o755), (&file, 0o755)] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let verify = |p: &Path| verify_binary(p, TrustedOwners::RootAnd(me()));
        // Accepted as installed ...
        check_effective(EffectiveBinary::Binary(file.clone()), "serve", &verify).unwrap();
        // ... refused once a directory above it is loosened.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o775)).unwrap();

        let (result, restarted) = guarded(
            Verb::Restart,
            EffectiveBinary::Binary(file.clone()),
            &verify,
        );

        let msg = permission_text(&result.expect_err("a loosened directory must refuse"));
        assert!(msg.contains(dir.to_str().unwrap()), "{msg}");
        assert!(msg.contains("0775"), "{msg}");
        assert!(msg.contains("service install --system"), "{msg}");
        assert!(restarted.is_empty(), "{restarted:?}");
    }

    #[test]
    fn guard_queries_the_loaded_definition_read_only() {
        #[cfg(target_os = "linux")]
        if !super::super::platform::linux_is_systemd() {
            // BusyBox reads the init script from disk; no command to observe.
            return;
        }
        let path = PathBuf::from("/home/alice/.cargo/bin/iroh-tunnel");
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, vec![output(0, &loaded(&path), "")]);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        let seen = std::cell::RefCell::new(Vec::new());
        let verify = |p: &Path| {
            seen.borrow_mut().push(p.to_path_buf());
            refuse_unsafe(p)
        };

        let err = guard_effective_binary(&mut native, "serve", &verify)
            .expect_err("the loaded binary is refused");

        assert_eq!(seen.into_inner(), std::slice::from_ref(&path));
        assert!(permission_text(&err).contains(path.to_str().unwrap()));
        let log = events.borrow();
        let calls = commands(&log);
        assert_eq!(calls.len(), 1, "{calls:?}");
        let super::super::native::fake::Event::Call { args, mode, .. } = calls[0] else {
            unreachable!()
        };
        assert_eq!(*mode, OutputMode::Capture);
        assert!(
            args.first().is_some_and(|a| a == "show" || a == "print"),
            "{args:?}"
        );
    }

    #[cfg(feature = "test-utils")]
    #[test]
    fn restart_never_writes_a_copy() {
        use std::os::unix::fs::MetadataExt;

        const NAME: &str = "service::verb::guard_tests::restart_never_writes_a_copy";
        super::super::fixtures::in_isolated_home(NAME, |home| {
            let base = home.canonicalize().unwrap().join("managed");
            std::fs::create_dir(&base).unwrap();
            std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::env::set_var("IROH_TUNNEL_TEST_MANAGED_BIN_DIR", &base);
            let copy = super::super::service_binary::managed_copy_path("serve");
            let source = home.join("running");
            std::fs::write(&source, b"managed bytes").unwrap();
            let binary = super::super::service_binary::ServiceBinary::Managed {
                source: super::super::types::RunningExecutable {
                    display: source.clone(),
                    read_from: source,
                },
                copy: copy.clone(),
            };
            let mine = TrustedOwners::RootAnd(me());
            super::super::service_binary::materialize(&binary, mine).unwrap();
            let snapshot = |p: &Path| {
                let meta = std::fs::symlink_metadata(p).unwrap();
                (meta.mtime(), meta.mtime_nsec(), meta.ino())
            };
            let role_dir = copy.parent().unwrap();
            let before = (snapshot(role_dir), snapshot(&copy));

            let events = Events::default();
            let mut fake = FakeRunner::new(&events, vec![output(0, &loaded(&copy), "")]);
            let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
            let mut native = Native::new(OutputMode::Capture, &mut runner);
            // BusyBox needs root to restart and the guard reads its script
            // from disk, so only the systemd and launchd hosts run this.
            #[cfg(target_os = "linux")]
            if !super::super::platform::linux_is_systemd() {
                return;
            }
            // The running binary is upgraded after the copy was made.
            std::fs::write(home.join("running"), b"upgraded bytes").unwrap();
            let result = verb_steps(
                &mut native,
                &mut |_: &ServiceNote| {},
                Verb::Restart,
                |native| guard_effective_binary(native, "serve", &|p| verify_binary(p, mine)),
                |native, notes| {
                    backend_verb(
                        Verb::Restart,
                        native,
                        notes,
                        Role::Serve,
                        ServiceScope::System,
                    )
                },
                |_| panic!("restart releases nothing"),
            );
            result.expect("restart of a trusted copy succeeds");
            let (applied, _) = native.finish();

            assert_eq!(
                applied
                    .iter()
                    .filter(|e| matches!(e, AppliedEffect::Started))
                    .count(),
                1,
                "{applied:?}"
            );
            assert!(
                !applied.iter().any(|e| matches!(
                    e,
                    AppliedEffect::ManagedBinaryWritten(_) | AppliedEffect::ManagedBinaryRemoved(_)
                )),
                "{applied:?}"
            );
            assert_eq!((snapshot(role_dir), snapshot(&copy)), before);
            assert_eq!(std::fs::read(&copy).unwrap(), b"managed bytes");
        });
    }
}
