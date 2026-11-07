//! Platform configuration and state locations, plus the macOS legacy
//! configuration migration (CFG-01, CFG-02; issue #84).
//!
//! macOS and Linux use `~/.config/iroh-tunnel` and `~/.local/state/iroh-tunnel`
//! and ignore the XDG variables. Windows uses the `LocalAppData` known folder
//! (`iroh-tunnel/config`, `iroh-tunnel/state`). Explicit overrides (`--config`,
//! `IROH_TUNNEL_STATE_DIR`) are applied by the callers before these defaults.

mod default_config_path;
mod default_state_dir;
mod in_home;
mod locations;
mod set_migration_notice;

pub use default_config_path::default_config_path;
pub use default_state_dir::default_state_dir;
#[cfg(unix)]
pub(crate) use in_home::{config_path_in, state_dir_in};
pub(crate) use in_home::{windows_config_path_in, windows_state_path_in};
#[cfg(windows)]
pub(crate) use locations::local_app_data;
pub use set_migration_notice::{set_migration_notice, MigrationNotice};
