//! Windows system-scope verbs and status through the Service Control Manager.
//!
//! The pure seam ([`start_system_checked`]) compiles everywhere for its tests;
//! the glue that binds it to the real [`Scm`](super::backend::windows_scm_impl::Scm)
//! exists on Windows only.

// Pure seam compiled everywhere for its tests, but only Windows consumes it.
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;

use super::backend::windows_scm::{
    not_installed, restart_with, service_name, start_with, stop_with, uninstall_with, Clock,
    ScmPort,
};
use super::native::ServiceNote;
use super::verb::Verb;

/// Re-check the binary recorded in the registration, then run `start`.
///
/// The binary may have been replaced since install (trust check timing), so
/// the CLI verifies it again before asking the SCM to launch it. `start` is not
/// called when the registration is missing or the binary fails the check.
///
/// # Errors
///
/// The service-not-installed error when `read_exe` yields `None`, or whatever
/// `read_exe`, `verify` or `start` return.
pub(crate) fn start_system_checked(
    name: &str,
    role: &str,
    read_exe: impl FnOnce() -> Result<Option<PathBuf>>,
    verify: impl FnOnce(&Path) -> Result<()>,
    start: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let exe = read_exe()?.ok_or_else(|| not_installed(name, role))?;
    verify(&exe)?;
    start()
}

/// Run `verb` over `port` for the system scope.
///
/// Start and restart re-verify the recorded binary first; restart waits for
/// the role lock between stop and start.
///
/// # Errors
///
/// Whatever the verification, the SCM operations or `wait_released` return.
pub(crate) fn system_verb_with(
    port: &mut dyn ScmPort,
    clock: &mut dyn Clock,
    verify: impl FnOnce(&Path) -> Result<()>,
    wait_released: impl FnOnce(Duration) -> Result<()>,
    notes: &mut dyn FnMut(&ServiceNote),
    verb: Verb,
    role: &str,
) -> Result<()> {
    let name = service_name(role);
    match verb {
        Verb::Start => {
            let exe = port.executable(&name)?;
            start_system_checked(
                &name,
                role,
                || Ok(exe),
                verify,
                || start_with(port, clock, role),
            )
        }
        Verb::Restart => {
            let exe = port.executable(&name)?;
            start_system_checked(
                &name,
                role,
                || Ok(exe),
                verify,
                || restart_with(port, clock, role, wait_released),
            )
        }
        Verb::Stop => {
            if stop_with(port, clock, role)? {
                notes(&ServiceNote::AlreadyStopped { name });
            }
            Ok(())
        }
        Verb::Uninstall => {
            uninstall_with(port, clock, role)?;
            notes(&ServiceNote::Uninstalled { name });
            Ok(())
        }
    }
}

#[cfg(windows)]
pub(crate) use glue::{system_registration, system_status_notes, system_verb};

#[cfg(windows)]
mod glue {
    use anyhow::Result;

    use super::super::backend::windows_scm::{
        not_installed, service_name, status_with, ScmStatus, SystemClock,
    };
    use super::super::backend::windows_scm_impl::Scm;
    use super::super::native::ServiceNote;
    use super::super::trust_windows::verify_binary;
    use super::super::types::Registration;
    use super::super::verb::Verb;
    use super::system_verb_with;
    use crate::role::Role;

    /// Run `verb` against the real SCM for the system scope.
    pub(crate) fn system_verb(
        verb: Verb,
        role: Role,
        notes: &mut dyn FnMut(&ServiceNote),
    ) -> Result<()> {
        system_verb_with(
            &mut Scm,
            &mut SystemClock::new(),
            verify_binary,
            |t| crate::ownership::wait_released(role, t),
            notes,
            verb,
            role.name(),
        )
    }

    /// The registration of the role's service; the not-installed error when
    /// absent.
    pub(crate) fn system_registration(role: Role) -> Result<Registration> {
        let id = role.name();
        match status_with(&mut Scm, id)? {
            ScmStatus::Absent => Err(not_installed(&service_name(id), id)),
            ScmStatus::Installed(state) => Ok(Registration::Present {
                location: format!("Windows service {} ({state})", service_name(id)),
            }),
        }
    }

    /// Report from the role lock whether the role runs; the SCM state alone
    /// does not cover a foreground run.
    pub(crate) fn system_status_notes(
        role: Role,
        notes: &mut dyn FnMut(&ServiceNote),
    ) -> Result<()> {
        match crate::ownership::probe(role)? {
            crate::ownership::Liveness::Running { owner, .. } => {
                notes(&ServiceNote::RoleRunning {
                    role: role.name().to_owned(),
                    owner: crate::ownership::describe_owner(owner.as_ref()),
                });
            }
            crate::ownership::Liveness::NotRunning => {
                notes(&ServiceNote::RoleNotRunning {
                    role: role.name().to_owned(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CliError;
    use std::cell::Cell;

    const EXE: &str = r"C:\bin\iroh-tunnel.exe";

    use super::super::backend::windows_scm::fake::{FakeClock, FakeScm};
    use super::super::backend::windows_scm::ScmState;
    use std::cell::RefCell;

    /// Notes collected from one `system_verb_with` run.
    fn run(
        port: &mut FakeScm,
        verb: Verb,
        verify: impl FnOnce(&Path) -> Result<()>,
        wait_released: impl FnOnce(Duration) -> Result<()>,
    ) -> (Result<()>, Vec<String>) {
        let notes = RefCell::new(Vec::new());
        let result = system_verb_with(
            port,
            &mut FakeClock::default(),
            verify,
            wait_released,
            &mut |n| notes.borrow_mut().push(format!("{n:?}")),
            verb,
            "serve",
        );
        (result, notes.into_inner())
    }

    fn registered(state: ScmState) -> FakeScm {
        let mut port = FakeScm::with(Some(state));
        port.exe = Some(PathBuf::from(EXE));
        port
    }

    #[test]
    fn stop_of_a_stopped_service_notes_already_stopped() {
        let mut port = registered(ScmState::Stopped);
        let (result, notes) = run(&mut port, Verb::Stop, |_| Ok(()), |_| Ok(()));
        result.unwrap();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("AlreadyStopped"), "{notes:?}");
    }

    #[test]
    fn stop_of_a_running_service_has_no_already_stopped_note() {
        let mut port = registered(ScmState::Running);
        let (result, notes) = run(&mut port, Verb::Stop, |_| Ok(()), |_| Ok(()));
        result.unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(port.calls.borrow().as_slice(), ["stop"]);
    }

    #[test]
    fn uninstall_notes_uninstalled() {
        let mut port = registered(ScmState::Running);
        let (result, notes) = run(&mut port, Verb::Uninstall, |_| Ok(()), |_| Ok(()));
        result.unwrap();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("Uninstalled"), "{notes:?}");
    }

    #[test]
    fn restart_verifies_the_binary_before_stopping() {
        let mut port = registered(ScmState::Running);
        let (result, _) = run(
            &mut port,
            Verb::Restart,
            |_| Err(CliError::Permission("binary is writable".into()).into()),
            |_| panic!("must not wait after a failed check"),
        );
        assert!(result.is_err());
        assert!(port.calls.borrow().is_empty(), "{:?}", port.calls);
    }

    #[test]
    fn restart_waits_for_the_lock_between_stop_and_start() {
        let mut port = registered(ScmState::Running);
        let log = std::rc::Rc::clone(&port.calls);
        let at_wait = RefCell::new(None);
        let (result, _) = run(
            &mut port,
            Verb::Restart,
            |_| Ok(()),
            |_| {
                *at_wait.borrow_mut() = Some(log.borrow().clone());
                Ok(())
            },
        );
        result.unwrap();
        assert_eq!(at_wait.into_inner().unwrap(), ["stop"]);
        assert_eq!(port.calls.borrow().as_slice(), ["stop", "start"]);
    }

    #[test]
    fn verb_start_system_checks_binary_first() {
        let started = Cell::new(false);
        let err = start_system_checked(
            "iroh-tunnel-serve",
            "serve",
            || Ok(Some(PathBuf::from(EXE))),
            |_| Err(CliError::Permission("binary is writable".into()).into()),
            || {
                started.set(true);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(!started.get(), "start must not run after a failed check");
        assert!(matches!(
            err.downcast_ref::<CliError>(),
            Some(CliError::Permission(_))
        ));
    }

    #[test]
    fn verb_start_system_verifies_the_recorded_binary_then_starts() {
        let verified = Cell::new(None);
        let started = Cell::new(false);
        start_system_checked(
            "iroh-tunnel-serve",
            "serve",
            || Ok(Some(PathBuf::from(EXE))),
            |p| {
                verified.set(Some(p.to_path_buf()));
                Ok(())
            },
            || {
                started.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(verified.take(), Some(PathBuf::from(EXE)));
        assert!(started.get());
    }

    #[test]
    fn verb_start_system_without_registration_is_not_installed() {
        let started = Cell::new(false);
        let err = start_system_checked(
            "iroh-tunnel-serve",
            "serve",
            || Ok(None),
            |_| panic!("nothing to verify"),
            || {
                started.set(true);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(!started.get());
        match err.downcast_ref::<CliError>() {
            Some(CliError::Service(m)) => {
                assert!(m.contains("service install --system"), "{m}");
            }
            other => panic!("expected Service, got {other:?}"),
        }
    }
}
