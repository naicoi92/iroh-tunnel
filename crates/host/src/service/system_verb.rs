//! Windows system-scope verbs and status through the Service Control Manager.
//!
//! The pure seam ([`start_system_checked`]) compiles everywhere for its tests;
//! the glue that binds it to the real [`Scm`](super::backend::windows_scm_impl::Scm)
//! exists on Windows only.

// Pure seam compiled everywhere for its tests, but only Windows consumes it.
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::backend::windows_scm::not_installed;

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

#[cfg(windows)]
pub(crate) use glue::{system_registration, system_status_notes, system_verb};

#[cfg(windows)]
mod glue {
    use anyhow::Result;

    use super::super::backend::windows_scm::ScmPort;
    use super::super::backend::windows_scm::{
        not_installed, restart_with, service_name, start_with, status_with, stop_with,
        uninstall_with, ScmState, ScmStatus, SystemClock, LOCK_TIMEOUT,
    };
    use super::super::backend::windows_scm_impl::Scm;
    use super::super::native::ServiceNote;
    use super::super::trust_windows::verify_binary;
    use super::super::types::Registration;
    use super::super::verb::Verb;
    use super::start_system_checked;
    use crate::role::Role;

    /// Run `then` against the real SCM after re-checking the recorded binary.
    fn checked(role: Role, then: impl FnOnce() -> Result<()>) -> Result<()> {
        let name = service_name(role.name());
        start_system_checked(
            &name,
            role.name(),
            || Scm.executable(&name),
            verify_binary,
            then,
        )
    }

    /// Run `verb` against the real SCM for the system scope.
    pub(crate) fn system_verb(
        verb: Verb,
        role: Role,
        notes: &mut dyn FnMut(&ServiceNote),
    ) -> Result<()> {
        let id = role.name();
        let mut clock = SystemClock::new();
        match verb {
            Verb::Start => checked(role, || start_with(&mut Scm, &mut clock, id)),
            Verb::Restart => checked(role, || {
                restart_with(&mut Scm, &mut clock, id, |t| {
                    debug_assert_eq!(t, LOCK_TIMEOUT);
                    crate::ownership::wait_released(role, t)
                })
            }),
            Verb::Stop => {
                let was_stopped =
                    status_with(&mut Scm, id)? == ScmStatus::Installed(ScmState::Stopped);
                stop_with(&mut Scm, &mut clock, id)?;
                if was_stopped {
                    notes(&ServiceNote::AlreadyStopped {
                        name: service_name(id),
                    });
                }
                Ok(())
            }
            Verb::Uninstall => {
                uninstall_with(&mut Scm, &mut clock, id)?;
                notes(&ServiceNote::Uninstalled {
                    name: service_name(id),
                });
                Ok(())
            }
        }
    }

    /// The registration of the role's service; the not-installed error when
    /// absent.
    pub(crate) fn system_registration(role: Role) -> Result<Registration> {
        let id = role.name();
        match status_with(&mut Scm, id)? {
            ScmStatus::Absent => Err(not_installed(&service_name(id), id)),
            ScmStatus::Installed(state) => Ok(Registration::Present {
                location: format!("Windows service {} ({state:?})", service_name(id)),
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
