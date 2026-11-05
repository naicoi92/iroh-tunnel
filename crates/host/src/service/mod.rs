//! System-service management: install/uninstall/start/stop/restart/status.
//!
//! The macOS backend is launchd. On Linux the backend is chosen at RUNTIME:
//! systemd when the host boots with it (detected via `/run/systemd/system`),
//! otherwise a BusyBox/SysV init backend that installs an
//! `/etc/init.d/S??iroh-tunnel-<role>` script (embedded buildroot devices such
//! as the Sipeed NanoKVM). On Windows the backend is a per-user Task Scheduler
//! task that runs in the logged-in session (user scope only; SM-04, issue
//! #92). Backends, `Native`, the trust policy and the account lookup are
//! private; the operations (`install`, `uninstall`, `start`, `stop`,
//! `restart`, `status`) are the only way in.

#[cfg(unix)]
pub(crate) mod account;
mod account_name;
mod backend;
mod decide_install_scope;
mod default_scope;
mod execute;
#[cfg(test)]
mod fixtures;
mod install;
mod is_elevated;
mod native;
mod parse_scope_answer;
mod pin;
mod platform;
mod policy;
mod registration;
mod restart;
mod start;
mod status;
mod stop;
#[cfg(unix)]
pub(crate) mod system;
mod task_xml;
#[cfg(unix)]
pub(crate) mod trust;
mod types;
mod uninstall;
mod verb;

pub use account_name::account_name;
pub use decide_install_scope::decide_install_scope;
pub use default_scope::default_scope;
pub use install::install;
pub use is_elevated::is_elevated;
pub use native::{InstalledAt, NativeOutput, OutputMode, ServiceCall, ServiceNote};
pub use parse_scope_answer::parse_scope_answer;
pub use pin::{Pin, RetargetConsent};
pub(crate) use platform::load_config;
pub use restart::restart;
pub use start::start;
pub use status::status;
pub use stop::stop;
#[cfg(unix)]
pub use system::unknown_user;
pub use types::{
    AppliedEffect, Environment, Fact, InstallNeed, InstallOutcome, InstallRequest, Registration,
    ScopeDecision, ServiceFailure, ServiceReport, ServiceScope, ServiceStatusReport,
};
pub use uninstall::uninstall;
