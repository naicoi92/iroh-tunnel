//! Request, outcome and policy types of the service operations.

use std::path::PathBuf;

use anyhow::Result;

use super::native::NativeOutput;
use crate::role::Role;
use crate::Tier;

/// A change an operation made to the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppliedEffect {
    /// A unit, plist, script or task definition was written at this path.
    RegistrationWritten(PathBuf),
    /// The service was enabled to start automatically.
    Enabled,
    /// The service was started.
    Started,
    /// The service was stopped.
    Stopped,
    /// Automatic start was disabled.
    Disabled,
    /// The registration at this path was removed.
    RegistrationRemoved(PathBuf),
    /// A managed root-owned copy of the service binary was written here.
    ManagedBinaryWritten(PathBuf),
    /// The managed copy of the service binary at this path was removed.
    ManagedBinaryRemoved(PathBuf),
}

/// The executable that is running this process, which a managed copy of the
/// service binary is made from.
///
/// `display` is what messages name; `read_from` is what is opened to copy
/// the bytes. On Linux `read_from` is `/proc/self/exe`, so the copy comes
/// from the executing file even if its path was replaced after exec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningExecutable {
    /// The path shown to the user (`std::env::current_exe()`).
    pub display: PathBuf,
    /// The path the bytes are read from.
    pub read_from: PathBuf,
}

/// What a successful operation did and what the native tools printed.
#[derive(Debug)]
pub struct ServiceReport {
    /// Changes made to the host, in order.
    pub applied: Vec<AppliedEffect>,
    /// Output of the native commands that ran.
    pub native: Vec<NativeOutput>,
}

/// A failed operation, with whatever it had already changed.
#[derive(Debug)]
pub struct ServiceFailure {
    /// Changes made before the failure.
    pub applied: Vec<AppliedEffect>,
    /// Output of the native commands that ran before the failure.
    pub native: Vec<NativeOutput>,
    /// The underlying error; a [`crate::error::CliError`] for policy refusals.
    pub error: anyhow::Error,
}

impl ServiceFailure {
    /// A failure before anything reached the host.
    pub(crate) fn before_effects(error: anyhow::Error) -> Self {
        Self {
            applied: Vec::new(),
            native: Vec::new(),
            error,
        }
    }
}

impl From<ServiceFailure> for anyhow::Error {
    /// The underlying error, unchanged, so `CliError` downcasts still work.
    fn from(failure: ServiceFailure) -> Self {
        failure.error
    }
}

/// A fact about the service that may be unobtainable without failing the
/// whole report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact<T> {
    /// The fact, as determined.
    Known(T),
    /// Why the fact could not be determined.
    Unavailable(String),
}

impl<T> Fact<T> {
    pub(crate) fn from_result(result: Result<T>) -> Self {
        match result {
            Ok(value) => Fact::Known(value),
            Err(error) => Fact::Unavailable(error.to_string()),
        }
    }
}

/// Whether a unit, plist, init script or task is registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// Where the registration lives.
    Present {
        /// Where the registration lives.
        location: String,
    },
    /// No registration exists.
    Absent,
}

/// What is known about a role's service, each fact from its own source:
/// the registration from the host's files, liveness and owner from the role
/// lock, and the native tool's output carried verbatim (never parsed).
#[derive(Debug)]
pub struct ServiceStatusReport {
    /// Whether a registration exists.
    pub registration: Fact<Registration>,
    /// Whether the role runs, from the role lock.
    pub liveness: Fact<crate::ownership::Liveness>,
    /// The role's owner, from the owner record: `Known(None)` when the role is
    /// not running; `Unavailable` when it runs but the record is missing or
    /// unreadable, or liveness could not be probed.
    pub owner: Fact<Option<crate::ownership::OwnerRecord>>,
    /// Native tool output, carried verbatim (never parsed).
    pub native: Option<NativeOutput>,
}

/// Which executable the person invoked, and so the command-line form the
/// registered executable accepts and the command guidance names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceInvocation {
    /// The combined `iroh-tunnel` executable: it requires a role subcommand.
    Combined,
    /// A role-specific executable: it accepts `run` directly.
    Standalone {
        /// The executable the person invoked, named in rerun guidance.
        executable: PathBuf,
    },
}

impl ServiceInvocation {
    /// The role subcommand the registered executable needs before `run`:
    /// `Some(role)` for the combined executable, `None` for a standalone one.
    #[must_use]
    pub fn role_arg<'r>(&self, role: &'r str) -> Option<&'r str> {
        match self {
            Self::Combined => Some(role),
            Self::Standalone { .. } => None,
        }
    }
}

/// Everything `install` needs from its caller. The tier is not part of the
/// request: [`super::install`] derives it from the caller's [`Environment`].
#[derive(Debug, Clone)]
pub struct InstallRequest {
    /// The role to register.
    pub role: Role,
    /// The command-line form accepted by the selected executable. A
    /// `Standalone` executable must be `tunnel_executable`: guidance names it
    /// while the registration runs `tunnel_executable`.
    pub invocation: ServiceInvocation,
    /// Config file; `None` selects the default of the caller's tier.
    pub config: Option<PathBuf>,
    /// The selected executable the registration runs. Must be absolute.
    pub tunnel_executable: PathBuf,
    /// The executable running this install: the source of a managed copy
    /// when a root-run registration cannot trust `tunnel_executable`.
    pub running_executable: RunningExecutable,
    /// `IROH_TUNNEL_STATE_DIR`, if set; `None` selects the default of the
    /// caller's tier.
    pub state_dir_override: Option<PathBuf>,
}

/// The caller's privilege, injected so the operations are testable. The
/// tier every operation acts in is derived from it
/// ([`crate::Tier::from_elevated`]); no caller can name another tier.
#[derive(Debug, Clone, Copy)]
pub struct Environment {
    /// The process runs as root / an elevated Administrator.
    pub elevated: bool,
}

impl Environment {
    /// The tier the operations act in.
    pub(crate) fn tier(self) -> Tier {
        Tier::from_elevated(self.elevated)
    }
}

/// Restart policy shared by every platform's service template.
///
/// Both `systemd::format_unit` and `launchd::format_plist` read from this so
/// the policy has a single source of truth — the previous design hard-coded it
/// separately in each template (`Restart=on-failure`/`RestartSec=5` in the
/// unit, `KeepAlive SuccessfulExit=false` in the plist), which silently drifted
/// whenever one side was edited.
///
/// `delay_secs` is honored by systemd only; launchd manages its own throttle
/// interval internally and has no user-settable knob for it. The field stays
/// shared so the policy reads as one coherent thing, even if one platform
/// ignores the backoff.
///
/// Containment differs: `on_failure` becomes `RestartPreventExitStatus=6`
/// under systemd, but a bare `KeepAlive{SuccessfulExit=false}` under launchd,
/// which has no exit-status exemption. A macOS job refused for "already
/// running" is therefore respawned until it is booted out.
#[derive(Debug)]
pub(crate) struct RestartPolicy {
    /// Restart the service when it exits with a non-zero status.
    pub on_failure: bool,
    /// Delay between restart attempts, in seconds. Read by systemd; ignored
    /// by launchd (which uses an internal ThrottleInterval).
    pub delay_secs: u64,
}

impl RestartPolicy {
    /// The policy iroh-tunnel ships with: restart on crash, 5 s backoff.
    /// Matches the previous hard-coded values in both templates.
    pub const DEFAULT: RestartPolicy = RestartPolicy {
        on_failure: true,
        delay_secs: 5,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_combined_executable_needs_the_role_argument() {
        let standalone = ServiceInvocation::Standalone {
            executable: PathBuf::from("/opt/role bin/renamed"),
        };
        assert_eq!(ServiceInvocation::Combined.role_arg("serve"), Some("serve"));
        assert_eq!(standalone.role_arg("serve"), None);
    }

    #[test]
    fn the_tier_follows_the_privilege() {
        assert_eq!(Environment { elevated: true }.tier(), Tier::System);
        assert_eq!(Environment { elevated: false }.tier(), Tier::User);
    }

    #[test]
    fn restart_policy_default_matches_previous_hardcoded_values() {
        // Regression guard: the shared RestartPolicy must produce the same
        // values the per-platform templates used to hard-code (Restart=
        // on-failure, RestartSec=5 / KeepAlive SuccessfulExit=false).
        let p = RestartPolicy::DEFAULT;
        assert!(p.on_failure, "default policy must restart on failure");
        assert_eq!(p.delay_secs, 5, "default policy must keep the 5s backoff");
    }
}
