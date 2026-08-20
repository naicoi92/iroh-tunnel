//! CLI definition: clap structs for `iroh-tunnel <ROLE> <COMMAND>`.
//!
//! Based on Page 06 v5 §8 (clap struct). Only defines the parse surface;
//! dispatch lives in `main.rs`, tracing init in [`init_tracing`].

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use tracing_subscriber::EnvFilter;

/// App version resolved at build time from the git tag (see `build.rs`).
///
/// Equals the tag (e.g. `1.2.3`) when HEAD sits exactly on a `vX.Y.Z` tag,
/// otherwise `{cargo version}-dev` (e.g. `0.1.0-dev`) for local builds.
const APP_VERSION: &str = env!("APP_VERSION");

/// CLI entry point.
#[derive(Parser, Debug)]
#[command(name = "iroh-tunnel", version = APP_VERSION, about = "P2P port-forwarding tunnel via Iroh")]
pub struct Cli {
    /// Increase logging verbosity (-v info, -vv debug, -vvv trace). Default is info.
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Quiet mode (only errors).
    #[arg(short, long)]
    pub quiet: bool,

    /// Color output: auto | always | never.
    #[arg(long, default_value = "auto")]
    pub color: String,

    #[command(subcommand)]
    pub role: Role,
}

/// Top-level role: serve or access.
#[derive(Subcommand, Debug)]
pub enum Role {
    /// Serve role: publish local services into Iroh.
    Serve {
        #[command(subcommand)]
        cmd: RoleCmd,
    },
    /// Access role: consume remote services to local.
    Access {
        #[command(subcommand)]
        cmd: RoleCmd,
    },
}

/// Commands available per role.
#[derive(Subcommand, Debug)]
pub enum RoleCmd {
    /// Run in foreground.
    Run {
        /// Config file path (default: OS-specific ~/.config/iroh-tunnel/{role}.toml).
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Config management (keygen/add/remove/list/show/edit/path).
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Print the role's status file as a table (`--json` for the raw file).
    Status {
        /// Print the status file verbatim instead of the table.
        #[arg(long)]
        json: bool,
    },
    /// System service management (systemd/launchd).
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

/// Config sub-actions.
#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Generate/rotate secret_key, write to config file.
    Keygen {
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Add a service to config.
    Add(AddServiceArgs),
    /// Remove a service from config by name.
    Remove {
        #[arg(long)]
        name: String,
    },
    /// List services in config.
    List,
    /// Print config file content (pretty).
    Show,
    /// Open config in $EDITOR.
    Edit,
    /// Print config file path.
    Path,
}

/// Service sub-actions.
///
/// Scope defaults to per-user (LaunchAgent / `systemctl --user`), which needs
/// no privileges and matches how iroh-tunnel is typically used on a desktop.
/// Pass `--system` to install as a system-wide daemon (LaunchDaemon /
/// `/etc/systemd/system`), which requires root and is intended for
/// server/headless hosts.
#[derive(Subcommand, Debug)]
pub enum ServiceAction {
    /// Install system service (systemd/launchd).
    Install {
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// System-wide daemon (LaunchDaemon / /etc/systemd/system). Requires root.
        #[arg(long)]
        system: bool,
    },
    /// Uninstall system service.
    Uninstall {
        #[arg(long)]
        system: bool,
    },
    /// Start system service.
    Start {
        #[arg(long)]
        system: bool,
    },
    /// Stop system service.
    Stop {
        #[arg(long)]
        system: bool,
    },
    /// Restart system service.
    Restart {
        #[arg(long)]
        system: bool,
    },
    /// Print system service status.
    Status {
        #[arg(long)]
        system: bool,
    },
}

/// Args for `config add`.
#[derive(Args, Debug)]
pub struct AddServiceArgs {
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub protocol: String,
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    #[arg(long)]
    pub port: u16,
    /// Required only for access role.
    #[arg(long)]
    pub node_id: Option<String>,
}

/// Initialize the tracing subscriber.
///
/// Precedence (highest first):
/// 1. `RUST_LOG` env var, if set — overrides everything.
/// 2. `--quiet` (`-q`) — forces `error`.
/// 3. `--verbose` (`-v`) count: 0/1=`info`, 2=`debug`, 3+=`trace`.
///
/// Default (no flags, no `RUST_LOG`) is `info`, so connection lifecycle
/// notices (peer connect/disconnect, "endpoint ready", …) surface by default.
pub fn init_tracing(verbose: u8, quiet: bool) {
    let filter = if let Ok(rust_log) = std::env::var("RUST_LOG") {
        EnvFilter::new(rust_log)
    } else if quiet {
        EnvFilter::new("error")
    } else {
        let level = match verbose {
            0 => "info",
            1 => "info",
            2 => "debug",
            _ => "trace",
        };
        EnvFilter::new(format!("iroh_tunnel={level}"))
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .init();
}
