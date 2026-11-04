//! Request, outcome and policy types of the service operations.

use std::path::PathBuf;

use anyhow::Result;

use super::native::NativeOutput;
use super::pin::{Pin, RetargetConsent};
use crate::role::Role;

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
    /// The running owner, from the owner record.
    pub owner: Fact<crate::ownership::OwnerRecord>,
    /// Native tool output, carried verbatim (never parsed).
    pub native: Option<NativeOutput>,
}

/// Everything `install` needs from its caller.
#[derive(Debug, Clone)]
pub struct InstallRequest {
    /// The role to register.
    pub role: Role,
    /// `None` lets [`scope::decide_install_scope`] decide.
    pub scope: Option<ServiceScope>,
    /// Config file; `None` selects the role default.
    pub config: Option<PathBuf>,
    /// The `iroh-tunnel` executable the registration runs. Must be absolute.
    pub tunnel_executable: PathBuf,
    /// `SUDO_USER`: an input to choosing whose files a system service uses.
    pub sudo_user: Option<String>,
    /// The user a person named when asked ([`InstallNeed::InvokingUser`]).
    pub invoking_user: Option<String>,
    /// `IROH_TUNNEL_STATE_DIR`, if set.
    pub state_dir_override: Option<PathBuf>,
    /// Consent to replace another owner's system registration.
    pub retarget: Option<RetargetConsent>,
}

/// What `install` did, or the decision it needs first.
#[derive(Debug)]
pub enum InstallOutcome {
    /// The registration was written, enabled and started.
    Installed(ServiceReport),
    /// A decision is needed first; nothing changed.
    Needs(InstallNeed),
}

/// A decision a person must make before `install` can continue. Nothing has
/// been changed when one is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallNeed {
    /// User or system service.
    ScopeChoice,
    /// Whose files a system service should use.
    InvokingUser,
    /// Replace an existing system registration that belongs to someone else
    /// (`previous: None` when its owner cannot be told).
    Retarget {
        /// The registration being replaced; `None` when its owner cannot be told.
        previous: Option<Pin>,
        /// The registration that would replace it.
        proposed: Pin,
    },
}

/// Whether the service is installed system-wide or per-user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceScope {
    /// A machine-wide service for the invoking user's files.
    System,
    /// A per-user service.
    User,
}

/// What the install-time scope decision depends on, injected so the policy is
/// a pure function of its inputs.
#[derive(Debug, Clone, Copy)]
pub struct Environment {
    /// The process runs as root / an elevated Administrator.
    pub elevated: bool,
    /// Standard input is a terminal that can answer the scope prompt.
    pub can_answer: bool,
}

/// Outcome of the pure install-scope policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeDecision {
    /// The scope is decided.
    Chosen(ServiceScope),
    /// A person must pick user or system.
    NeedsChoice,
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
#[allow(dead_code)] // unused on targets without a service backend
pub(crate) struct RestartPolicy {
    /// Restart the service when it exits with a non-zero status.
    pub on_failure: bool,
    /// Delay between restart attempts, in seconds. Read by systemd; ignored
    /// by launchd (which uses an internal ThrottleInterval).
    #[allow(dead_code)] // unread on macOS builds (launchd has no equivalent knob)
    pub delay_secs: u64,
}

impl RestartPolicy {
    /// The policy iroh-tunnel ships with: restart on crash, 5 s backoff.
    /// Matches the previous hard-coded values in both templates.
    #[allow(dead_code)] // unused on targets without a service backend
    pub const DEFAULT: RestartPolicy = RestartPolicy {
        on_failure: true,
        delay_secs: 5,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_variants_exist() {
        // Compile + equality smoke test for the public enum.
        assert_ne!(ServiceScope::System, ServiceScope::User);
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
