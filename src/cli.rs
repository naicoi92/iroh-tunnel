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
#[command(name = "iroh-tunnel", version = APP_VERSION, about = "P2P port-forwarding tunnel via Iroh", after_help = "Aliases: it = iroh-tunnel, its = iroh-tunnel-serve (Serve only), ita = iroh-tunnel-access (Access only)")]
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
    /// Serve role: publish local services into Iroh (standalone binary: iroh-tunnel-serve, alias its).
    #[command(args_conflicts_with_subcommands = true)]
    Serve {
        #[command(flatten)]
        quick: ServeQuickArgs,
        #[command(subcommand)]
        cmd: Option<RoleCmd>,
    },
    /// Access role: consume remote services to local (standalone binary: iroh-tunnel-access, alias ita).
    #[command(args_conflicts_with_subcommands = true)]
    Access {
        #[command(flatten)]
        quick: AccessQuickArgs,
        #[command(subcommand)]
        cmd: Option<RoleCmd>,
    },
}

/// Standalone Serve binary (`iroh-tunnel-serve`, alias `its`).
#[derive(Parser, Debug)]
#[command(name = "iroh-tunnel-serve", version = APP_VERSION, about = "Serve role: publish a local service through an Iroh tunnel (alias: its)", args_conflicts_with_subcommands = true)]
pub struct ServeCli {
    /// Increase logging verbosity (-v info, -vv debug, -vvv trace). Default is info.
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// Quiet mode (only errors).
    #[arg(short, long)]
    pub quiet: bool,
    /// Color output: auto | always | never.
    #[arg(long, default_value = "auto")]
    pub color: String,
    #[command(flatten)]
    pub quick: ServeQuickArgs,
    #[command(subcommand)]
    pub cmd: Option<RoleCmd>,
}

impl ServeCli {
    /// Equivalent all-in-one invocation, so both paths share one dispatcher.
    pub fn into_cli(self) -> Cli {
        Cli {
            verbose: self.verbose,
            quiet: self.quiet,
            color: self.color,
            role: Role::Serve {
                quick: self.quick,
                cmd: self.cmd,
            },
        }
    }
}

/// Standalone Access binary (`iroh-tunnel-access`, alias `ita`).
#[derive(Parser, Debug)]
#[command(name = "iroh-tunnel-access", version = APP_VERSION, about = "Access role: expose a remote Iroh service on a local address (alias: ita)", args_conflicts_with_subcommands = true)]
pub struct AccessCli {
    /// Increase logging verbosity (-v info, -vv debug, -vvv trace). Default is info.
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// Quiet mode (only errors).
    #[arg(short, long)]
    pub quiet: bool,
    /// Color output: auto | always | never.
    #[arg(long, default_value = "auto")]
    pub color: String,
    #[command(flatten)]
    pub quick: AccessQuickArgs,
    #[command(subcommand)]
    pub cmd: Option<RoleCmd>,
}

impl AccessCli {
    /// Equivalent all-in-one invocation, so both paths share one dispatcher.
    pub fn into_cli(self) -> Cli {
        Cli {
            verbose: self.verbose,
            quiet: self.quiet,
            color: self.color,
            role: Role::Access {
                quick: self.quick,
                cmd: self.cmd,
            },
        }
    }
}

/// Quick Serve options; omit when using a role command.
#[derive(Args, Debug)]
pub struct ServeQuickArgs {
    /// Service name.
    #[arg(short, long)]
    pub name: Option<String>,
    /// Fixed TCP target socket address.
    #[arg(short, long)]
    pub target: Option<std::net::SocketAddr>,
    /// Protocol (TCP only in quick mode).
    #[arg(short, long, default_value = "tcp")]
    pub protocol: String,
    /// Relay URL used by both peers.
    #[arg(long)]
    pub relay_url: Option<String>,
    /// File holding a persistent secret key (base64, as written by `config keygen`);
    /// keeps the public identity stable across restarts. Default: ephemeral identity.
    #[arg(short = 'k', long)]
    pub key_file: Option<PathBuf>,
}

/// Quick Access options; omit when using a role command.
#[derive(Args, Debug)]
pub struct AccessQuickArgs {
    /// Service name.
    #[arg(short, long)]
    pub name: Option<String>,
    /// Local TCP listen socket address.
    #[arg(short, long)]
    pub listen: Option<std::net::SocketAddr>,
    /// Serve NodeId (public identity, not authorization; protect the target service).
    #[arg(short = 'N', long)]
    pub node_id: Option<String>,
    /// Protocol (TCP only in quick mode).
    #[arg(short, long, default_value = "tcp")]
    pub protocol: String,
    /// Relay URL used by both peers.
    #[arg(long)]
    pub relay_url: Option<String>,
    /// File holding a persistent secret key (base64, as written by `config keygen`);
    /// keeps the public identity stable across restarts. Default: ephemeral identity.
    #[arg(short = 'k', long)]
    pub key_file: Option<PathBuf>,
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
    /// Service management: install, start, stop, restart, status, uninstall.
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
        #[command(subcommand)]
        mode: Option<KeygenMode>,
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

/// Alternative `config keygen` modes.
#[derive(Subcommand, Debug)]
pub enum KeygenMode {
    /// Print a fresh random secret key and its node ID; nothing is written.
    Random,
}

/// Scope selection shared by every service verb.
///
/// With neither flag, `install` offers the choice to a non-elevated
/// interactive user and defaults to system for root/Administrator; the other
/// verbs act on the user service unless elevated. A system scope without
/// privilege is refused with rerun guidance, never elevated or replaced by a
/// user service.
#[derive(Args, Debug, Clone, Copy)]
pub struct ScopeArgs {
    /// Per-user service (LaunchAgent / `systemctl --user`); needs no privileges.
    #[arg(long, conflicts_with = "system")]
    pub user: bool,
    /// System-wide service (LaunchDaemon / /etc/systemd/system); needs root.
    #[arg(long)]
    pub system: bool,
}

/// Service sub-actions.
#[derive(Subcommand, Debug)]
pub enum ServiceAction {
    /// Install, enable and start the service.
    Install {
        /// Config file to pin into the service (absolute path is stored).
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Stop and remove the service.
    Uninstall {
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Start the service.
    Start {
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Stop the service.
    Stop {
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Restart the service.
    Restart {
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Print the service status.
    Status {
        #[command(flatten)]
        scope: ScopeArgs,
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use std::collections::HashMap;

    /// No command declares the same short option twice (clap's debug assert
    /// also covers this) and equivalent options use one letter across roles.
    #[test]
    fn short_options_are_consistent_and_conflict_free() {
        let mut meaning: HashMap<char, String> = HashMap::new();
        for mut command in [
            ServeCli::command(),
            AccessCli::command(),
            Cli::command().find_subcommand("serve").unwrap().clone(),
            Cli::command().find_subcommand("access").unwrap().clone(),
        ] {
            command.build();
            let mut seen = HashMap::new();
            for arg in command.get_arguments() {
                let (Some(short), Some(long)) = (arg.get_short(), arg.get_long()) else {
                    continue;
                };
                assert!(seen.insert(short, long).is_none(), "duplicate -{short}");
                let previous = meaning.entry(short).or_insert_with(|| long.to_owned());
                assert_eq!(previous, long, "-{short} means different options");
            }
        }
        for (short, long) in [('n', "name"), ('p', "protocol"), ('k', "key-file")] {
            assert_eq!(meaning[&short], long);
        }
    }
}
