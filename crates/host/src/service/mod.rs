//! System-service management: install/uninstall/start/stop/restart/status.
//!
//! The macOS backend is launchd. On Linux the backend is chosen at RUNTIME:
//! systemd when the host boots with it (detected via `/run/systemd/system`),
//! otherwise a BusyBox/SysV init backend that installs an
//! `/etc/init.d/S??iroh-tunnel-<role>` script (embedded buildroot devices such
//! as the Sipeed NanoKVM). On Windows the user tier is a per-user Task
//! Scheduler task that runs in the logged-in session and the system tier is a
//! Windows Service. Every operation acts in the tier of the caller's privilege
//! ([`Environment`]); none takes a scope. Backends, `Native` and the trust
//! policy are private; the operations (`install`, `uninstall`, `start`,
//! `stop`, `restart`, `status`) are the only way in.

mod backend;
mod effective_binary;
mod execute;
#[cfg(test)]
mod fixtures;
mod install;
mod is_elevated;
mod native;
mod platform;
mod policy;
mod registration;
mod restart;
mod service_binary;
mod start;
mod status;
mod stop;
// Pure Windows seams, compiled elsewhere only for their tests.
#[cfg(any(windows, test))]
mod system_verb;
#[cfg(any(windows, test))]
mod task_xml;
#[cfg(unix)]
pub(crate) mod trust;
pub(crate) mod trust_windows;
mod types;
mod uninstall;
mod verb;
// Win32 FFI helpers, compiled elsewhere only for their tests.
#[cfg(any(windows, test))]
mod win32_ffi;

mod log_file;
pub use install::install;
pub use is_elevated::is_elevated;
pub use log_file::{machine_log_dir, CappedLog};
pub use native::{InstalledAt, NativeOutput, OutputMode, ServiceCall, ServiceNote};
pub(crate) use platform::load_config;
pub use policy::shell_quote;
pub use restart::restart;
pub use start::start;
pub use status::status;
pub use stop::stop;
#[cfg(windows)]
pub(crate) use trust_windows::{create_dir_with_sddl, program_data_dir, verify_machine_dir};
pub use types::{
    AppliedEffect, Environment, Fact, InstallRequest, Registration, RunningExecutable,
    ServiceFailure, ServiceInvocation, ServiceReport, ServiceStatusReport,
};
pub use uninstall::uninstall;

/// Runs the service binary trust check on `path` (installer ACL test).
///
/// # Errors
///
/// The trust refusal for `path`.
#[cfg(all(windows, feature = "test-utils"))]
#[doc(hidden)]
pub fn __test_verify_binary(path: &std::path::Path) -> anyhow::Result<()> {
    trust_windows::verify_binary(path)
}

/// Set to `1` in the environment of a Windows Service registration so the
/// binary knows the SCM started it and enters the service dispatcher.
pub const ENV_SCM: &str = "IROH_TUNNEL_SCM";

/// Set to `1` in the environment of every Unix registration (systemd,
/// launchd, BusyBox) so `run` knows a service manager started it.
pub const ENV_SERVICE: &str = "IROH_TUNNEL_SERVICE";

/// Test builds only (`test-utils`, unit tests): one uid (Unix) or SID (Windows) added to the admin-only
/// trusted set, so trust success paths are testable without root.
#[cfg(any(test, feature = "test-utils"))]
const ENV_TEST_TRUSTED_OWNER: &str = "IROH_TUNNEL_TEST_TRUSTED_OWNER";

/// An environment lookup, injected so the policy is testable without
/// touching the process environment.
pub(crate) type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<std::ffi::OsString>;

/// The real process environment.
pub(crate) fn process_env(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name)
}

/// Whether this process runs as a system service: the system tier and the
/// service marker of its registration. A user service carries the marker
/// too but runs in the user tier, and a foreground elevated run has no
/// marker; neither is a system service.
pub(crate) fn under_system_service() -> bool {
    system_service_marked(crate::Tier::current(), &process_env)
}

/// [`under_system_service`] for an explicit tier and environment.
pub(crate) fn system_service_marked(tier: crate::Tier, env: EnvLookup<'_>) -> bool {
    tier == crate::Tier::System
        && [ENV_SERVICE, ENV_SCM]
            .iter()
            .any(|name| env(name).is_some_and(|value| value == "1"))
}

/// The test-build trusted owner from `env`; always `None` in production
/// builds, which trust admin principals only.
#[cfg_attr(
    not(any(unix, windows)),
    expect(dead_code, reason = "no trust policy on this target")
)]
pub(crate) fn test_trusted_owner(env: EnvLookup<'_>) -> Option<std::ffi::OsString> {
    #[cfg(any(test, feature = "test-utils"))]
    return env(ENV_TEST_TRUSTED_OWNER);
    #[cfg(not(any(test, feature = "test-utils")))]
    {
        let _ = env;
        None
    }
}

/// The admin-only trust policy of a system service on this platform.
#[cfg(unix)]
pub(crate) type AdminTrust = trust::TrustedOwners;
/// The admin-only trust policy of a system service on this platform.
#[cfg(windows)]
pub(crate) type AdminTrust = trust_windows::model::TrustedSet;

/// The admin-only trust policy (plus the test-build owner from `env`).
///
/// # Errors
///
/// [`crate::error::CliError::Config`] for a malformed test owner.
#[cfg(unix)]
pub(crate) fn admin_trust(env: EnvLookup<'_>) -> anyhow::Result<AdminTrust> {
    trust::admin_owners(test_trusted_owner(env).as_deref())
}

/// The admin-only trust policy (plus the test-build owner from `env`).
///
/// # Errors
///
/// [`crate::error::CliError::Config`] for a malformed test owner.
#[cfg(windows)]
pub(crate) fn admin_trust(env: EnvLookup<'_>) -> anyhow::Result<AdminTrust> {
    trust_windows::admin_set(test_trusted_owner(env).as_deref())
}

/// SCM name of the role's system service (plus the test-only instance suffix).
#[must_use]
pub fn system_service_name(role: &str) -> String {
    backend::windows_scm::service_name(role)
}
