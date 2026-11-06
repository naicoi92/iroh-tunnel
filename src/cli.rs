//! CLI definition: clap structs for `iroh-tunnel <ROLE> <COMMAND>`.
//!
//! Based on Page 06 v5 §8 (clap struct). Only defines the parse surface;
//! dispatch lives in `main.rs`, tracing init in [`init_tracing`].

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use crate::cli_args::{AccessServiceArgs, ColorArg, ProtocolArg, ServeServiceArgs, ServiceAddr};
use anyhow::Context as _;
use iroh_tunnel_host::config::SetService;
use iroh_tunnel_host::Role as StatusRole;

/// App version resolved at build time from the git tag (see `build.rs`).
///
/// Equals the tag (e.g. `1.2.3`) when HEAD sits exactly on a `vX.Y.Z` tag,
/// otherwise `{cargo version}-dev` (e.g. `0.1.0-dev`) for local builds.
const APP_VERSION: &str = env!("APP_VERSION");

/// CLI entry point.
#[derive(Parser, Debug)]
#[command(name = "iroh-tunnel", version = APP_VERSION, about = "P2P port-forwarding tunnel via Iroh", arg_required_else_help = true, after_help = "Aliases: it = iroh-tunnel, its = iroh-tunnel-serve (Serve only), ita = iroh-tunnel-access (Access only)")]
pub struct Cli {
    /// Increase logging verbosity (-v info, -vv debug, -vvv trace). Default is info.
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Quiet mode (only errors).
    #[arg(short, long)]
    pub quiet: bool,

    /// Color output.
    #[arg(long, value_enum, default_value = "auto")]
    pub color: ColorArg,

    #[command(subcommand)]
    pub role: Role,
}

/// Top-level role: serve or access.
#[derive(Subcommand, Debug)]
pub enum Role {
    /// Serve role: publish local services into Iroh (standalone binary: iroh-tunnel-serve, alias its).
    #[command(
        args_conflicts_with_subcommands = true,
        subcommand_negates_reqs = true,
        arg_required_else_help = true
    )]
    Serve {
        #[command(flatten)]
        quick: ServeQuickArgs,
        #[command(subcommand)]
        cmd: Option<RoleCmd<ServeServiceArgs>>,
    },
    /// Access role: consume remote services to local (standalone binary: iroh-tunnel-access, alias ita).
    #[command(
        args_conflicts_with_subcommands = true,
        subcommand_negates_reqs = true,
        arg_required_else_help = true
    )]
    Access {
        #[command(flatten)]
        quick: AccessQuickArgs,
        #[command(subcommand)]
        cmd: Option<RoleCmd<AccessServiceArgs>>,
    },
    /// Update this executable (iroh-tunnel) to the latest stable or a chosen release.
    Update(UpdateArgs),
}

/// Options of `update`; it replaces only the executable that is running.
#[derive(Args, Debug, Clone)]
pub struct UpdateArgs {
    /// Release to install, e.g. 0.4.2 (default: latest stable).
    #[arg(long = "to", value_name = "VERSION")]
    pub to: Option<String>,
    /// Skip the signature and checksum verification (unsafe; prints a warning).
    #[arg(short = 'i', long)]
    pub insecure: bool,
    /// Replace an executable that a package manager owns, after you accept that the
    /// package database will no longer match the file. Never applies to containers.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

/// Standalone Serve binary (`iroh-tunnel-serve`, alias `its`).
#[derive(Parser, Debug)]
#[command(name = "iroh-tunnel-serve", version = APP_VERSION, about = "Serve role: publish a local service through an Iroh tunnel (alias: its)", args_conflicts_with_subcommands = true, subcommand_negates_reqs = true, arg_required_else_help = true)]
pub struct ServeCli {
    /// Increase logging verbosity (-v info, -vv debug, -vvv trace). Default is info.
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// Quiet mode (only errors).
    #[arg(short, long)]
    pub quiet: bool,
    /// Color output.
    #[arg(long, value_enum, default_value = "auto")]
    pub color: ColorArg,
    #[command(flatten)]
    pub quick: ServeQuickArgs,
    #[command(subcommand)]
    pub cmd: Option<RoleCmd<ServeServiceArgs>>,
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
#[command(name = "iroh-tunnel-access", version = APP_VERSION, about = "Access role: expose a remote Iroh service on a local address (alias: ita)", args_conflicts_with_subcommands = true, subcommand_negates_reqs = true, arg_required_else_help = true)]
pub struct AccessCli {
    /// Increase logging verbosity (-v info, -vv debug, -vvv trace). Default is info.
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// Quiet mode (only errors).
    #[arg(short, long)]
    pub quiet: bool,
    /// Color output.
    #[arg(long, value_enum, default_value = "auto")]
    pub color: ColorArg,
    #[command(flatten)]
    pub quick: AccessQuickArgs,
    #[command(subcommand)]
    pub cmd: Option<RoleCmd<AccessServiceArgs>>,
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
    #[command(flatten)]
    pub service: ServeServiceArgs,
    /// Relay URL used by both peers.
    #[arg(short = 'r', long)]
    pub relay_url: Option<String>,
    /// File holding a persistent secret key (base64, as written by `config keygen`);
    /// keeps the public identity stable across restarts. Default: ephemeral identity.
    #[arg(short = 'k', long)]
    pub key_file: Option<PathBuf>,
}

/// Quick Access options; omit when using a role command.
#[derive(Args, Debug)]
pub struct AccessQuickArgs {
    #[command(flatten)]
    pub service: AccessServiceArgs,
    /// Relay URL used by both peers.
    #[arg(short = 'r', long)]
    pub relay_url: Option<String>,
    /// File holding a persistent secret key (base64, as written by `config keygen`);
    /// keeps the public identity stable across restarts. Default: ephemeral identity.
    #[arg(short = 'k', long)]
    pub key_file: Option<PathBuf>,
}

/// Which executable is running. Service units run `<exe> <role> run`, which
/// only the all-in-one binary accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binary {
    /// `iroh-tunnel` (`it`).
    AllInOne,
    /// `iroh-tunnel-serve` (`its`).
    Serve,
    /// `iroh-tunnel-access` (`ita`).
    Access,
}

impl Binary {
    /// True for the single-role executables.
    pub fn is_standalone(self) -> bool {
        !matches!(self, Binary::AllInOne)
    }
}

/// Make quick-mode flags required and default the protocol to tcp.
fn quick_required(cmd: clap::Command, required: &[&str]) -> clap::Command {
    let mut cmd = cmd;
    for id in required {
        cmd = cmd.mut_arg(*id, |a| a.required(true));
    }
    cmd.mut_arg("protocol", |a| a.default_value("tcp"))
}

/// `config set`: the name is required, and the protocol help states the
/// create-time default (the flag itself has no default so an update keeps it).
fn config_set(role: clap::Command) -> clap::Command {
    role.mut_subcommand("config", |config| {
        config.mut_subcommand("set", |set| {
            set.mut_arg("name", |a| a.required(true))
                .mut_arg("protocol", |a| {
                    let help = a.get_help().map(ToString::to_string).unwrap_or_default();
                    a.help(format!("{help} (default on create: tcp)"))
                })
        })
    })
}

/// Attach `Examples:` to a role root and its `config` verbs. `prefix` is the
/// invocation up to the role, e.g. `it serve` or `its`; the text differs per
/// binary, so it is set here rather than in the shared derive.
fn examples(role: clap::Command, prefix: &str, access: bool) -> clap::Command {
    let (quick, set) = if access {
        (
            format!("{prefix} -n web -l 8080 -N <NODE_ID>"),
            vec![format!("{prefix} config set -n web -l 8080 -N <NODE_ID>")],
        )
    } else {
        (
            format!("{prefix} -n web -t 3000"),
            vec![
                format!("{prefix} config set -n web -t 3000"),
                format!("{prefix} config set -n web -t 3001 -p udp"),
            ],
        )
    };
    let block = |lines: &[String]| format!("Examples:\n  {}", lines.join("\n  "));
    let rm = block(&[format!("{prefix} config rm -n web")]);
    let keygen = block(&[
        format!("{prefix} config keygen"),
        format!("{prefix} config keygen --print"),
    ]);
    role.after_help(block(&[quick]))
        .mut_subcommand("config", |config| {
            config
                .mut_subcommand("set", |c| c.after_help(block(&set)))
                .mut_subcommand("rm", |c| c.after_help(rm))
                .mut_subcommand("keygen", |c| c.after_help(keygen))
        })
}

const SERVE_QUICK: &[&str] = &["name", "target"];
const ACCESS_QUICK: &[&str] = &["name", "listen", "node_id"];

/// Build the clap command tree of `binary` with the runtime adjustments the
/// derive cannot express (required quick flags, protocol default).
pub fn build_command(binary: Binary) -> clap::Command {
    let mut cmd = match binary {
        Binary::AllInOne => Cli::command()
            .mut_subcommand("serve", |c| {
                examples(
                    config_set(quick_required(c, SERVE_QUICK)),
                    "it serve",
                    false,
                )
            })
            .mut_subcommand("access", |c| {
                examples(
                    config_set(quick_required(c, ACCESS_QUICK)),
                    "it access",
                    true,
                )
            }),
        Binary::Serve => examples(
            config_set(quick_required(ServeCli::command(), SERVE_QUICK)),
            "its",
            false,
        ),
        Binary::Access => examples(
            config_set(quick_required(AccessCli::command(), ACCESS_QUICK)),
            "ita",
            true,
        ),
    };
    cmd.build();
    cmd
}

/// Parse `argv` against the tree of `binary`; standalone results are
/// converted to the all-in-one [`Cli`] shape.
pub fn parse_cli(
    binary: Binary,
    argv: impl IntoIterator<Item = OsString>,
) -> Result<Cli, clap::Error> {
    let mut matches = build_command(binary).try_get_matches_from_mut(argv)?;
    Ok(match binary {
        Binary::AllInOne => Cli::from_arg_matches_mut(&mut matches)?,
        Binary::Serve => ServeCli::from_arg_matches_mut(&mut matches)?.into_cli(),
        Binary::Access => AccessCli::from_arg_matches_mut(&mut matches)?.into_cli(),
    })
}

/// Service flag group of one role, shared by quick mode and `config set`.
pub trait ServiceArgs: Args + FromArgMatches {
    /// The role these flags belong to.
    const ROLE: StatusRole;

    /// Build the `config set` request. A missing name never gets here because
    /// [`build_command`] makes `-n` required; it is an internal error if it does.
    fn into_set(self, config: Option<PathBuf>) -> anyhow::Result<SetService>;
}

fn set_request(
    config: Option<PathBuf>,
    name: Option<String>,
    addr: Option<ServiceAddr>,
    protocol: Option<ProtocolArg>,
    node_id: Option<String>,
) -> anyhow::Result<SetService> {
    let name = name.context("internal: config set requires --name")?;
    Ok(SetService {
        config,
        name,
        host: addr.as_ref().and_then(|a| a.host.clone()),
        port: addr.map(|a| a.port.get()),
        protocol: protocol.map(Into::into),
        node_id,
    })
}

impl ServiceArgs for ServeServiceArgs {
    const ROLE: StatusRole = StatusRole::Serve;

    fn into_set(self, config: Option<PathBuf>) -> anyhow::Result<SetService> {
        set_request(config, self.name, self.target, self.protocol, None)
    }
}

impl ServiceArgs for AccessServiceArgs {
    const ROLE: StatusRole = StatusRole::Access;

    fn into_set(self, config: Option<PathBuf>) -> anyhow::Result<SetService> {
        set_request(config, self.name, self.listen, self.protocol, self.node_id)
    }
}

/// Commands available per role.
#[derive(Subcommand, Debug)]
pub enum RoleCmd<S: ServiceArgs> {
    /// Run in foreground.
    Run {
        /// Config file path (default: OS-specific ~/.config/iroh-tunnel/{role}.toml).
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Config management (set/rm/list/show/edit/path/check/keygen).
    #[command(arg_required_else_help = true)]
    Config {
        #[command(subcommand)]
        action: ConfigAction<S>,
    },
    /// Print the role's status file as a table (`--json` for the raw file).
    Status {
        /// Print the status file verbatim instead of the table.
        #[arg(long)]
        json: bool,
    },
    /// Service management: install, start, stop, restart, status, uninstall.
    #[command(arg_required_else_help = true)]
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Update this standalone executable to the latest stable or a chosen release.
    Update(UpdateArgs),
}

/// Config sub-actions.
#[derive(Subcommand, Debug)]
pub enum ConfigAction<S: ServiceArgs> {
    /// Create a service or change fields of an existing one.
    Set {
        #[command(flatten)]
        service: S,
        /// Config file path (default: OS-specific ~/.config/iroh-tunnel/{role}.toml).
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Remove a service from config by name.
    Rm {
        /// Name of the service to remove.
        #[arg(short = 'n', long)]
        name: String,
        /// Config file path (default: OS-specific ~/.config/iroh-tunnel/{role}.toml).
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// List services in config.
    List {
        /// Config file path (default: OS-specific ~/.config/iroh-tunnel/{role}.toml).
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Print config file content; `secret_key` and `relay_token` are hidden
    /// unless `--reveal` is given.
    Show {
        /// Config file path (default: OS-specific ~/.config/iroh-tunnel/{role}.toml).
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Print `secret_key` and `relay_token` instead of hiding them.
        #[arg(long)]
        reveal: bool,
    },
    /// Open config in $EDITOR; the result is validated before it is saved.
    Edit {
        /// Config file path (default: OS-specific ~/.config/iroh-tunnel/{role}.toml).
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Print config file path.
    Path {
        /// Config file path (default: OS-specific ~/.config/iroh-tunnel/{role}.toml).
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Validate the config file without changing it.
    Check {
        /// Config file path (default: OS-specific ~/.config/iroh-tunnel/{role}.toml).
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Generate/rotate secret_key and write it to the config file; use
    /// `--print` to print a key without saving.
    Keygen {
        /// Config file to write; not allowed with `--print`.
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Replace an existing key without asking; not allowed with `--print`.
        #[arg(short, long)]
        yes: bool,
        /// Print a fresh random secret key and its node ID; nothing is written.
        #[arg(long, conflicts_with_all = ["config", "yes"])]
        print: bool,
    },
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
        /// Config file to pin into the service (absolute path is stored; default: OS-specific).
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
    use std::collections::HashMap;

    fn argv(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn empty_name_is_usage_error() {
        assert!(parse_cli(Binary::Serve, argv(&["its", "-n", "", "-t", "1"])).is_err());
    }

    #[test]
    fn quick_requires_name_and_target() {
        let err = parse_cli(Binary::Serve, argv(&["its", "-n", "web"])).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
        let err = parse_cli(Binary::AllInOne, argv(&["it", "access", "-n", "web"])).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn quick_protocol_defaults_to_tcp() {
        let cli = parse_cli(Binary::Serve, argv(&["its", "-n", "web", "-t", "3000"])).unwrap();
        let Role::Serve { quick, .. } = cli.role else {
            panic!("serve role")
        };
        assert_eq!(
            quick.service.protocol,
            Some(crate::cli_args::ProtocolArg::Tcp)
        );
    }

    #[test]
    fn set_without_name_is_usage_error() {
        for (binary, args) in [
            (
                Binary::AllInOne,
                vec!["it", "serve", "config", "set", "-t", "3000"],
            ),
            (Binary::Serve, vec!["its", "config", "set", "-t", "3000"]),
            (Binary::Access, vec!["ita", "config", "set", "-l", "3000"]),
        ] {
            let err = parse_cli(binary, argv(&args)).unwrap_err();
            assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
        }
    }

    #[test]
    fn set_has_no_protocol_default_and_documents_create_default() {
        let cli = parse_cli(Binary::Serve, argv(&["its", "config", "set", "-n", "web"])).unwrap();
        let Role::Serve {
            cmd:
                Some(RoleCmd::Config {
                    action: ConfigAction::Set { service, .. },
                }),
            ..
        } = cli.role
        else {
            panic!("config set")
        };
        assert_eq!(service.protocol, None);
        let cmd = build_command(Binary::Serve);
        let set = cmd
            .find_subcommand("config")
            .unwrap()
            .find_subcommand("set")
            .unwrap();
        let help = set
            .get_arguments()
            .find(|a| a.get_id() == "protocol")
            .unwrap()
            .get_help()
            .unwrap()
            .to_string();
        assert!(help.ends_with(" (default on create: tcp)"), "{help}");
    }

    #[test]
    fn role_command_does_not_need_quick_flags() {
        assert!(parse_cli(Binary::AllInOne, argv(&["it", "serve", "config", "path"])).is_ok());
        assert!(parse_cli(Binary::Serve, argv(&["its", "config", "path"])).is_ok());
    }

    fn all_binaries() -> [Binary; 3] {
        [Binary::AllInOne, Binary::Serve, Binary::Access]
    }

    /// Visit `cmd` and every subcommand below it with its path.
    fn walk(
        cmd: &clap::Command,
        path: &mut Vec<String>,
        f: &mut dyn FnMut(&[String], &clap::Command),
    ) {
        f(path, cmd);
        for sub in cmd.get_subcommands() {
            path.push(sub.get_name().to_owned());
            walk(sub, path, f);
            path.pop();
        }
    }

    fn for_each_command(f: &mut dyn FnMut(Binary, &[String], &clap::Command)) {
        for b in all_binaries() {
            let root = build_command(b);
            walk(&root, &mut Vec::new(), &mut |p, c| f(b, p, c));
        }
    }

    fn find<'a>(root: &'a clap::Command, path: &[&str]) -> &'a clap::Command {
        path.iter().fold(root, |c, n| {
            c.find_subcommand(n).unwrap_or_else(|| panic!("no {n}"))
        })
    }

    /// Each short letter maps to one long name (table in the spec), and no
    /// command declares the same short twice.
    #[test]
    fn short_options_are_consistent_and_conflict_free() {
        let table: HashMap<char, &str> = HashMap::from([
            ('c', "config"),
            ('n', "name"),
            ('t', "target"),
            ('l', "listen"),
            ('N', "node-id"),
            ('p', "protocol"),
            ('r', "relay-url"),
            ('k', "key-file"),
            ('y', "yes"),
        ]);
        for_each_command(&mut |b, path, cmd| {
            let mut seen = HashMap::new();
            for arg in cmd.get_arguments() {
                let Some(short) = arg.get_short() else {
                    continue;
                };
                let long = arg.get_long().unwrap_or("");
                assert!(
                    seen.insert(short, long).is_none(),
                    "duplicate -{short} in {b:?} {path:?}"
                );
                // Global/update/verbosity letters outside the table are fixed.
                if let Some(expected) = table.get(&short) {
                    assert_eq!(*expected, long, "-{short} in {b:?} {path:?}");
                }
            }
        });
    }

    #[test]
    fn long_only_options() {
        for_each_command(&mut |b, path, cmd| {
            for arg in cmd.get_arguments() {
                if matches!(arg.get_long(), Some("reveal" | "print" | "system" | "user")) {
                    assert_eq!(arg.get_short(), None, "{b:?} {path:?} {:?}", arg.get_long());
                }
            }
        });
    }

    #[test]
    fn no_blank_descriptions() {
        for_each_command(&mut |b, path, cmd| {
            for arg in cmd.get_arguments() {
                if matches!(arg.get_id().as_str(), "help" | "version") {
                    continue;
                }
                let help = arg.get_help().map(ToString::to_string).unwrap_or_default();
                assert!(!help.trim().is_empty(), "{b:?} {path:?} {}", arg.get_id());
            }
        });
    }

    #[test]
    fn shared_flags_match_between_quick_and_set() {
        let strip = |s: String| s.replace(" (default on create: tcp)", "");
        for (b, role, ids) in [
            (Binary::Serve, None, &["name", "target", "protocol"][..]),
            (
                Binary::Access,
                None,
                &["name", "listen", "node_id", "protocol"][..],
            ),
            (
                Binary::AllInOne,
                Some("serve"),
                &["name", "target", "protocol"][..],
            ),
            (
                Binary::AllInOne,
                Some("access"),
                &["name", "listen", "node_id", "protocol"][..],
            ),
        ] {
            let root = build_command(b);
            let quick = match role {
                Some(r) => find(&root, &[r]),
                None => &root,
            };
            let set = find(quick, &["config", "set"]);
            for id in ids {
                let q = quick.get_arguments().find(|a| a.get_id() == *id).unwrap();
                let s = set.get_arguments().find(|a| a.get_id() == *id).unwrap();
                assert_eq!(q.get_long(), s.get_long(), "{id}");
                assert_eq!(q.get_short(), s.get_short(), "{id}");
                assert_eq!(q.get_value_names(), s.get_value_names(), "{id}");
                let qh = strip(q.get_help().unwrap().to_string());
                let sh = strip(s.get_help().unwrap().to_string());
                assert_eq!(qh, sh, "{id}");
                if *id != "protocol" {
                    assert_eq!(
                        q.get_default_values(),
                        s.get_default_values(),
                        "{id} default"
                    );
                }
            }
        }
    }

    #[test]
    fn config_set_has_no_relay_flag() {
        for_each_command(&mut |_, path, cmd| {
            if path.ends_with(&["config".to_owned(), "set".to_owned()]) {
                assert!(cmd
                    .get_arguments()
                    .all(|a| a.get_long() != Some("relay-url")));
            }
        });
    }

    #[test]
    fn config_set_requires_name() {
        let err = parse_cli(
            Binary::AllInOne,
            argv(&["it", "serve", "config", "set", "-t", "3000"]),
        )
        .unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    fn has_examples(cmd: &clap::Command) -> bool {
        cmd.get_after_help()
            .is_some_and(|h| h.to_string().contains("Examples:"))
    }

    #[test]
    fn examples_on_listed_commands() {
        for b in all_binaries() {
            let root = build_command(b);
            let roles: Vec<(&str, &clap::Command)> = match b {
                Binary::AllInOne => vec![
                    ("serve", find(&root, &["serve"])),
                    ("access", find(&root, &["access"])),
                ],
                Binary::Serve => vec![("serve", &root)],
                Binary::Access => vec![("access", &root)],
            };
            for (name, role) in roles {
                assert!(has_examples(role), "{b:?} {name} root");
                assert!(
                    has_examples(find(role, &["config", "set"])),
                    "{b:?} {name} set"
                );
                if name == "serve" {
                    assert!(has_examples(find(role, &["config", "rm"])), "{b:?} rm");
                    assert!(
                        has_examples(find(role, &["config", "keygen"])),
                        "{b:?} keygen"
                    );
                }
            }
        }
    }

    #[test]
    fn examples_use_the_binary_name() {
        let text = |c: &clap::Command| c.get_after_help().unwrap().to_string();
        let it = build_command(Binary::AllInOne);
        assert!(text(find(&it, &["serve", "config", "set"]))
            .contains("it serve config set -n web -t 3000"));
        assert!(text(find(&it, &["access"])).contains("it access -n web -l 8080 -N <NODE_ID>"));
        let its = build_command(Binary::Serve);
        assert!(text(find(&its, &["config", "keygen"])).contains("its config keygen --print"));
        let ita = build_command(Binary::Access);
        assert!(text(find(&ita, &["config", "set"]))
            .contains("ita config set -n web -l 8080 -N <NODE_ID>"));
        assert!(text(find(&ita, &["config", "set"])).contains("<NODE_ID>"));
    }

    #[test]
    fn keygen_print_conflicts() {
        let err = parse_cli(
            Binary::AllInOne,
            argv(&["it", "serve", "config", "keygen", "--print", "-y"]),
        )
        .unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn bare_invocations_request_help() {
        for (b, args) in [
            (Binary::AllInOne, vec!["it"]),
            (Binary::AllInOne, vec!["it", "serve"]),
            (Binary::AllInOne, vec!["it", "access"]),
            (Binary::Serve, vec!["its"]),
            (Binary::Access, vec!["ita"]),
            (Binary::Access, vec!["ita", "config"]),
            (Binary::AllInOne, vec!["it", "serve", "service"]),
            (Binary::AllInOne, vec!["it", "serve", "config"]),
        ] {
            let err = parse_cli(b, argv(&args)).unwrap_err();
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand,
                "{args:?}"
            );
        }
    }

    #[test]
    fn access_help_keeps_target_protection_guidance() {
        let root = build_command(Binary::Access);
        let help = find(&root, &["config", "set"])
            .clone()
            .render_help()
            .to_string();
        assert!(
            help.contains("public identity, not authorization"),
            "{help}"
        );
    }
}
