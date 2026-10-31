//! Shared entry points for the three role binaries (`iroh-tunnel`,
//! `iroh-tunnel-serve`, `iroh-tunnel-access`): parse, init tracing, build the
//! runtime, dispatch and map errors to exit codes (see [`CliError`], RS-06).
//!
//! The binaries are thin wrappers so each one is a self-contained executable
//! that never needs another binary at runtime.

use std::path::{Path, PathBuf};

use crate::cli::{
    AccessCli, Cli, ConfigAction, KeygenMode, Role, RoleCmd, ScopeArgs, ServeCli, ServiceAction,
};
use crate::config_cmd;
use crate::error::CliError;
use crate::shutdown;
use crate::status::{self, StatusRole};
use crate::status_cmd;
use crate::{access, cli, serve, service};
use anyhow::Context as _;
use clap::Parser;

/// Entry point of the all-in-one `iroh-tunnel` (`it`) binary.
pub fn main_all_in_one() -> ! {
    execute(Cli::parse(), Packaging::AllInOne)
}

/// Entry point of the standalone `iroh-tunnel-serve` (`its`) binary.
pub fn main_serve() -> ! {
    execute(ServeCli::parse().into_cli(), Packaging::Standalone)
}

/// Entry point of the standalone `iroh-tunnel-access` (`ita`) binary.
pub fn main_access() -> ! {
    execute(AccessCli::parse().into_cli(), Packaging::Standalone)
}

/// Which executable is running. Service units run `<exe> <role> run`, which
/// only the all-in-one binary accepts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Packaging {
    AllInOne,
    Standalone,
}

fn execute(parsed: Cli, packaging: Packaging) -> ! {
    cli::init_tracing(parsed.verbose, parsed.quiet);

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: failed to start runtime: {e}");
            std::process::exit(1);
        }
    };

    // Role to clean up if shutdown is abandoned; only `run` commands own
    // a status file, but removal is pid-checked so other commands are safe.
    let status_role = match &parsed.role {
        Role::Serve { .. } => StatusRole::Serve,
        Role::Access { .. } => StatusRole::Access,
    };
    // Racing the run future means a forced exit first DROPS it: owned tasks
    // and the endpoint are cancelled before the process status is decided.
    let outcome = rt.block_on(async {
        tokio::select! {
            result = run(parsed, packaging) => Ok(result),
            forced = shutdown::forced_exit() => Err(forced),
        }
    });
    let outcome = match outcome {
        Ok(result) => result,
        Err(forced) => {
            // Exit statuses: 130 repeated stop after grace, 1 watchdog (see
            // CHANGELOG). Own-state cleanup is still attempted here.
            eprintln!("error: shutdown abandoned ({forced:?}), exiting");
            status::remove_own_status_file_for_exit(status_role);
            std::process::exit(forced.exit_code());
        }
    };
    match outcome {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("error: {e:#}");
            let code = match e.downcast_ref::<CliError>() {
                Some(CliError::Config(_)) => 2,
                Some(CliError::Permission(_)) => 3,
                Some(CliError::Iroh(_)) => 4,
                Some(CliError::Service(_)) => 5,
                Some(CliError::Conflict(_)) => 6,
                None => 1,
            };
            std::process::exit(code);
        }
    }
}

async fn run(cli: Cli, packaging: Packaging) -> anyhow::Result<()> {
    // The typed role tag comes from ONE match on the CLI enum; the dispatch
    // string is derived from it — an invalid role is unrepresentable, and
    // the two can never disagree.
    let status_role = match &cli.role {
        Role::Serve { .. } => StatusRole::Serve,
        Role::Access { .. } => StatusRole::Access,
    };
    match cli.role {
        Role::Serve { cmd: Some(cmd), .. } | Role::Access { cmd: Some(cmd), .. } => {
            dispatch_role_cmd(status_role, cmd, packaging).await
        }
        Role::Serve { quick, cmd: None } => {
            if quick.protocol != "tcp" {
                return Err(CliError::Config(
                    "serve quick mode supports only tcp protocol; use --protocol tcp".into(),
                )
                .into());
            }
            let target = quick.target.ok_or_else(|| {
                CliError::Config("serve quick mode: --target IP:PORT is required".into())
            })?;
            let secret_key = read_key_file(quick.key_file.as_deref())?;
            let cfg = crate::config::ServeConfig {
                node: crate::config::NodeConfig {
                    secret_key,
                    relay_urls: quick.relay_url.into_iter().collect(),
                    ..Default::default()
                },
                services: vec![crate::config::ServeService {
                    name: quick.name.ok_or_else(|| {
                        CliError::Config("serve quick mode: --name is required".into())
                    })?,
                    protocol: crate::config::Protocol::Tcp,
                    host: target.ip().to_string(),
                    port: target.port(),
                }],
            };
            serve::run_quick(cfg).await
        }
        Role::Access { quick, cmd: None } => {
            if quick.protocol != "tcp" {
                return Err(CliError::Config(
                    "access quick mode supports only tcp protocol; use --protocol tcp".into(),
                )
                .into());
            }
            let listen = quick.listen.ok_or_else(|| {
                CliError::Config("access quick mode: --listen IP:PORT is required".into())
            })?;
            let node_id = quick.node_id.ok_or_else(|| {
                CliError::Config("access quick mode: --node-id is required".into())
            })?;
            node_id.parse::<iroh::EndpointId>().map_err(|e| {
                CliError::Config(format!("access quick mode: invalid --node-id: {e}"))
            })?;
            let secret_key = read_key_file(quick.key_file.as_deref())?;
            let cfg = crate::config::AccessConfig {
                node: crate::config::NodeConfig {
                    secret_key,
                    relay_urls: quick.relay_url.into_iter().collect(),
                    ..Default::default()
                },
                services: vec![crate::config::AccessService {
                    name: quick.name.ok_or_else(|| {
                        CliError::Config("access quick mode: --name is required".into())
                    })?,
                    node_id,
                    protocol: crate::config::Protocol::Tcp,
                    host: listen.ip().to_string(),
                    port: listen.port(),
                    relay_urls: vec![],
                }],
            };
            access::run_quick(cfg).await
        }
    }
}

async fn dispatch_role_cmd(
    status_role: StatusRole,
    cmd: RoleCmd,
    packaging: Packaging,
) -> anyhow::Result<()> {
    let role = status_role.name();
    match cmd {
        RoleCmd::Run { config } => {
            let path = resolve_config_path(role, config)?;
            config_cmd::require_existing(role, &path)?;
            // The typed role tag exhaustively selects the run handler —
            // the compiler enforces both arms, no string fallthrough.
            match status_role {
                StatusRole::Serve => serve::run(&path).await,
                StatusRole::Access => access::run(&path).await,
            }
        }
        RoleCmd::Config { action } => dispatch_config(role, action),
        RoleCmd::Status { json } => status_cmd::run(status_role, json),
        RoleCmd::Service { .. } if packaging == Packaging::Standalone => {
            Err(CliError::Service(format!(
                "service management is not yet supported by the standalone {role} binary; \
             use `iroh-tunnel {role} service ...`"
            ))
            .into())
        }
        RoleCmd::Service { action } => dispatch_service(role, action),
    }
}

/// Resolve the config path: explicit `--config` wins, otherwise the
/// platform default from [`config_cmd::resolve_config_path`].
fn resolve_config_path(role: &str, config: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    config_cmd::resolve_config_path(role, config.as_deref())
}

/// Make `path` absolute for embedding in a system service file.
///
/// Installed services run with a different working directory, so a relative
/// `--config` path must be resolved against the current CWD before it is
/// written into the unit/plist. Falls back to joining the CWD if the path
/// can't be canonicalized (e.g. it doesn't exist yet).
fn absolutize_config_path(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    if let Ok(abs) = std::fs::canonicalize(path) {
        return Ok(abs);
    }
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    Ok(cwd.join(path))
}

fn dispatch_config(role: &str, action: ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::Keygen {
            mode: Some(KeygenMode::Random),
            ..
        } => config_cmd::keygen_random(),
        ConfigAction::Keygen {
            config,
            yes,
            mode: None,
        } => config_cmd::keygen(role, config.as_deref(), yes),
        ConfigAction::Add(args) => config_cmd::add(role, &args),
        ConfigAction::Remove { config, name } => config_cmd::remove(role, config.as_deref(), &name),
        ConfigAction::List { config } => config_cmd::list(role, config.as_deref()),
        ConfigAction::Show { config, reveal } => config_cmd::show(role, config.as_deref(), reveal),
        ConfigAction::Edit { config } => config_cmd::edit(role, config.as_deref()),
        ConfigAction::Path { config } => config_cmd::path(role, config.as_deref()),
    }
}

fn dispatch_service(role: &str, action: ServiceAction) -> anyhow::Result<()> {
    use service::scope::{self, Environment};
    use service::ServiceScope;

    let env = Environment::detect();
    let explicit = |args: &ScopeArgs| match (args.user, args.system) {
        (_, true) => Some(ServiceScope::System),
        (true, _) => Some(ServiceScope::User),
        _ => None,
    };
    let flag = |scope: ServiceScope| match scope {
        ServiceScope::System => "--system",
        ServiceScope::User => "--user",
    };
    // Verbs on an existing service: no prompt, and the scope follows privilege
    // when it is not given.
    let existing = |verb: &str, args: &ScopeArgs| -> anyhow::Result<ServiceScope> {
        let scope = explicit(args).unwrap_or_else(|| scope::default_scope(env.elevated));
        scope::ensure_supported(scope)?;
        let rerun = ["iroh-tunnel", role, "service", verb, flag(scope)].map(str::to_owned);
        scope::ensure_privilege(scope, env.elevated, &rerun)?;
        Ok(scope)
    };
    match action {
        ServiceAction::Install { config, scope } => {
            // Windows has only the user scope, so there is nothing to ask.
            let requested = explicit(&scope).or(cfg!(windows).then_some(ServiceScope::User));
            let scope = scope::choose_install_scope(
                requested,
                env,
                &mut std::io::stdin().lock(),
                &mut std::io::stderr(),
            )?;
            scope::ensure_supported(scope)?;
            // Persist absolute paths: the installed service runs with a
            // different CWD and home, so nothing may stay relative (SM-02).
            let config = absolutize_config_path(&resolve_config_path(role, config)?)?;
            let state = pinned_state_dir()?;
            let rerun: Vec<String> = [
                "iroh-tunnel",
                role,
                "service",
                "install",
                flag(scope),
                "--config",
            ]
            .into_iter()
            .map(str::to_owned)
            .chain([config.to_string_lossy().into_owned()])
            .collect();
            scope::ensure_privilege(scope, env.elevated, &rerun)?;
            check_installable_config(role, &config)?;
            service::install(role, scope, &config, &state)
        }
        ServiceAction::Uninstall { scope } => {
            service::uninstall(role, existing("uninstall", &scope)?)
        }
        ServiceAction::Start { scope } => {
            let scope = existing("start", &scope)?;
            // Refuse here too: the unit would otherwise start and exit
            // after systemd/launchd already reported success (RO-03).
            let status_role = match role {
                "serve" => StatusRole::Serve,
                "access" => StatusRole::Access,
                other => anyhow::bail!("unknown role {other:?}"),
            };
            crate::ownership::ensure_not_running(status_role)?;
            service::start(role, scope)
        }
        ServiceAction::Stop { scope } => service::stop(role, existing("stop", &scope)?),
        ServiceAction::Restart { scope } => service::restart(role, existing("restart", &scope)?),
        ServiceAction::Status { scope } => service::status(role, existing("status", &scope)?),
    }
}

/// The state directory the installed service will write to, as an absolute
/// path: `IROH_TUNNEL_STATE_DIR` when set, otherwise the platform default.
fn pinned_state_dir() -> anyhow::Result<PathBuf> {
    let dir = match std::env::var_os("IROH_TUNNEL_STATE_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => crate::paths::default_state_dir()?,
    };
    std::path::absolute(&dir)
        .with_context(|| format!("cannot make state directory {} absolute", dir.display()))
}

/// Refuse to register a service that could only fail at start: the pinned
/// config must exist and be valid for the role.
fn check_installable_config(role: &str, config: &Path) -> anyhow::Result<()> {
    let loaded = match role {
        "serve" => crate::config::ServeConfig::load(config).map(drop),
        _ => crate::config::AccessConfig::load(config).map(drop),
    };
    loaded.map_err(|e| {
        CliError::Config(format!(
            "cannot install the service: {e:#}. Create or fix the config first \
             (`iroh-tunnel {role} config keygen --config {path}`), or pass --config",
            path = config.display()
        ))
        .into()
    })
}

/// Read the persistent secret key supplied with `-k/--key-file`.
///
/// Returns an empty string (ephemeral identity) when no file is given. The
/// file holds the base64 key written by `config keygen`. Diagnostics name the
/// path but never echo file contents (RS-10); an unusable key is a config
/// error and nothing is written back (RS-05, RS-06).
fn read_key_file(path: Option<&Path>) -> anyhow::Result<String> {
    let Some(path) = path else {
        return Ok(String::new());
    };
    let text = std::fs::read_to_string(path).map_err(|e| {
        CliError::Config(format!("--key-file {}: cannot read: {e}", path.display()))
    })?;
    let key = text.trim();
    if key.is_empty() {
        return Err(
            CliError::Config(format!("--key-file {}: file is empty", path.display())).into(),
        );
    }
    crate::config::resolve_secret_key(key).map_err(|_| {
        CliError::Config(format!(
            "--key-file {}: not a valid base64 32-byte secret key",
            path.display()
        ))
    })?;
    Ok(key.to_owned())
}
