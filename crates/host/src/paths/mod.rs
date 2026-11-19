//! Platform configuration and state locations, plus the macOS legacy
//! configuration migration (CFG-01, CFG-02; issue #84).
//!
//! Paths depend on the [`crate::Tier`]. The user tier (macOS and Linux) uses
//! `~/.config/iroh-tunnel` and `~/.local/state/iroh-tunnel` and ignores the XDG
//! variables; Windows uses the `LocalAppData` known folder
//! (`iroh-tunnel/config`, `iroh-tunnel/state`). The system tier uses fixed
//! machine-wide paths and never reads the home directory: Linux
//! `/etc/iroh-tunnel` and `/run/iroh-tunnel`; macOS
//! `/Library/Application Support/iroh-tunnel/{config,state}` (machine locks stay in
//! `/var/run/iroh-tunnel`);
//! Windows `%ProgramData%\iroh-tunnel\{config,state}`. Explicit overrides
//! (`--config`, `IROH_TUNNEL_STATE_DIR`) are applied by the callers before
//! these defaults.

mod default_config_path;
mod default_config_path_for;
mod default_state_dir;
mod default_state_dir_for;
mod locations;
mod set_migration_notice;

pub use default_config_path::default_config_path;
pub use default_config_path_for::default_config_path_for;
pub use default_state_dir::default_state_dir;
pub use default_state_dir_for::default_state_dir_for;
#[cfg(all(windows, test))]
pub(crate) use locations::local_app_data;
pub use set_migration_notice::{set_migration_notice, MigrationNotice};
