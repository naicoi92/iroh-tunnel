//! The one place that picks a service backend for this host and scope.

#[cfg(unix)]
use std::path::PathBuf;

use anyhow::Result;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::super::effective_binary::EffectiveBinary;
use super::super::native::{Native, NativeOutput, ServiceNote};
use super::super::registration::RegistrationInput;
use super::super::types::ServiceScope;
use super::super::verb::Verb;
use crate::role::Role;

/// The service manager that holds a registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Backend {
    /// systemd units (Linux booted with systemd).
    #[cfg(target_os = "linux")]
    Systemd,
    /// An `/etc/init.d` script (Linux without systemd); system-wide only.
    #[cfg(target_os = "linux")]
    Busybox,
    /// launchd agents and daemons (macOS).
    #[cfg(target_os = "macos")]
    Launchd,
    /// A per-user Task Scheduler task (Windows user scope).
    #[cfg(windows)]
    WindowsTask,
    /// A Windows Service (Windows system scope).
    #[cfg(windows)]
    WindowsService,
}

impl Backend {
    /// The backend that manages `scope` registrations on this host.
    ///
    /// # Errors
    ///
    /// When this platform has no service backend.
    #[cfg_attr(
        not(windows),
        expect(unused_variables, reason = "only Windows picks by scope")
    )]
    pub(crate) fn select(scope: ServiceScope) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            // `platform::system_wide_only` is the same condition.
            Ok(if super::super::platform::linux_is_systemd() {
                Self::Systemd
            } else {
                Self::Busybox
            })
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Self::Launchd)
        }
        #[cfg(windows)]
        {
            Ok(match scope {
                ServiceScope::User => Self::WindowsTask,
                ServiceScope::System => Self::WindowsService,
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            super::super::platform::unsupported()
        }
    }

    /// Write, enable and start `reg`.
    pub(crate) fn install(
        self,
        native: &mut Native,
        notes: &mut dyn FnMut(&ServiceNote),
        reg: &RegistrationInput<'_>,
    ) -> Result<()> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Systemd => super::systemd::install(native, notes, reg),
            #[cfg(target_os = "linux")]
            Self::Busybox => super::busybox::install(native, notes, reg),
            #[cfg(target_os = "macos")]
            Self::Launchd => super::launchd::install(native, notes, reg),
            #[cfg(windows)]
            Self::WindowsTask => super::windows::install(native, notes, reg),
            #[cfg(windows)]
            Self::WindowsService => super::windows_scm::install(notes, reg),
        }
    }

    /// Run `verb` on `role`'s registration in `scope`.
    pub(crate) fn verb(
        self,
        verb: Verb,
        native: &mut Native,
        notes: &mut dyn FnMut(&ServiceNote),
        role_id: Role,
        scope: ServiceScope,
    ) -> Result<()> {
        let role = role_id.name();
        match self {
            #[cfg(target_os = "linux")]
            Self::Systemd => {
                use super::systemd;
                match verb {
                    Verb::Uninstall => systemd::uninstall(native, notes, role, scope),
                    Verb::Start => systemd::start(native, role, scope),
                    Verb::Stop => systemd::stop(native, notes, role, scope),
                    Verb::Restart => systemd::restart(native, notes, role, scope),
                }
            }
            #[cfg(target_os = "linux")]
            Self::Busybox => {
                use super::busybox;
                match verb {
                    Verb::Uninstall => busybox::uninstall(native, notes, role, scope),
                    Verb::Start => busybox::start(native, role, scope),
                    Verb::Stop => busybox::stop(native, notes, role, scope),
                    Verb::Restart => busybox::restart(native, notes, role, scope),
                }
            }
            #[cfg(target_os = "macos")]
            Self::Launchd => {
                use super::launchd;
                match verb {
                    Verb::Uninstall => launchd::uninstall(native, notes, role, scope),
                    Verb::Start => launchd::start(native, role, scope),
                    Verb::Stop => launchd::stop(native, notes, role, scope),
                    Verb::Restart => launchd::restart(native, notes, role, scope),
                }
            }
            #[cfg(windows)]
            Self::WindowsTask => {
                use super::windows;
                match verb {
                    Verb::Uninstall => windows::uninstall(native, notes, role, scope),
                    Verb::Start => windows::start(native, role, scope),
                    Verb::Stop => windows::stop(native, notes, role, scope),
                    Verb::Restart => windows::restart(native, notes, role, scope),
                }
            }
            #[cfg(windows)]
            Self::WindowsService => super::super::system_verb::system_verb(verb, role_id, notes),
        }
    }

    /// The native status of `role` in `scope`. `liveness_note` is the
    /// role's running/stopped note, which only the Windows backends print.
    #[cfg_attr(
        not(windows),
        expect(unused_variables, reason = "only the Windows backends report liveness")
    )]
    pub(crate) fn status(
        self,
        native: &mut Native,
        notes: &mut dyn FnMut(&ServiceNote),
        liveness_note: Option<&ServiceNote>,
        role_id: Role,
        scope: ServiceScope,
    ) -> Result<NativeOutput> {
        let role = role_id.name();
        match self {
            #[cfg(target_os = "linux")]
            Self::Systemd => super::systemd::status(native, notes, role, scope),
            #[cfg(target_os = "linux")]
            Self::Busybox => super::busybox::status(native, notes, role, scope),
            #[cfg(target_os = "macos")]
            Self::Launchd => super::launchd::status(native, notes, role, scope),
            #[cfg(windows)]
            Self::WindowsTask => super::windows::status(native, notes, liveness_note, role, scope),
            #[cfg(windows)]
            Self::WindowsService => {
                if let Some(note) = liveness_note {
                    notes(note);
                }
                Ok(NativeOutput::none())
            }
        }
    }

    /// Where `role`'s registration file is (or would be).
    #[cfg(unix)]
    pub(crate) fn registration_path(self, role: &str, scope: ServiceScope) -> Result<PathBuf> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Systemd => super::systemd::registration_path(role, scope),
            #[cfg(target_os = "linux")]
            Self::Busybox => Ok(super::busybox::registration_path(role)),
            #[cfg(target_os = "macos")]
            Self::Launchd => super::launchd::registration_path(role, scope),
        }
    }

    /// The text of `role`'s installed registration, if any (to read its pin).
    #[cfg(unix)]
    pub(crate) fn existing_registration(self, role: &str, scope: ServiceScope) -> Option<String> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Systemd => super::systemd::existing_registration(role, scope),
            #[cfg(target_os = "linux")]
            Self::Busybox => super::busybox::existing_registration(role),
            #[cfg(target_os = "macos")]
            Self::Launchd => super::launchd::existing_registration(role, scope),
        }
    }

    /// What the manager will run for `role`'s root-run service, from the
    /// loaded definition.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn effective_binary(self, native: &mut Native, role: &str) -> EffectiveBinary {
        match self {
            #[cfg(target_os = "linux")]
            Self::Systemd => super::systemd::effective_binary(native, role),
            #[cfg(target_os = "linux")]
            Self::Busybox => super::busybox::effective_binary(role),
            #[cfg(target_os = "macos")]
            Self::Launchd => super::launchd::effective_binary(native, role),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_backend_follows_the_init_system_for_every_scope() {
        let expected = if super::super::super::platform::linux_is_systemd() {
            Backend::Systemd
        } else {
            Backend::Busybox
        };
        for scope in [ServiceScope::User, ServiceScope::System] {
            assert_eq!(Backend::select(scope).unwrap(), expected);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_backend_is_launchd_for_every_scope() {
        for scope in [ServiceScope::User, ServiceScope::System] {
            assert_eq!(Backend::select(scope).unwrap(), Backend::Launchd);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_backend_follows_the_scope() {
        assert_eq!(
            Backend::select(ServiceScope::User).unwrap(),
            Backend::WindowsTask
        );
        assert_eq!(
            Backend::select(ServiceScope::System).unwrap(),
            Backend::WindowsService
        );
    }
}
