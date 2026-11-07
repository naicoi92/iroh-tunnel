//! Shared entry points for the three role binaries (`iroh-tunnel`,
//! `iroh-tunnel-serve`, `iroh-tunnel-access`): parse, init tracing, build the
//! runtime, dispatch and map errors to exit codes (see [`CliError`], RS-06).
//!
//! The binaries are thin wrappers so each one is a self-contained executable
//! that never needs another binary at runtime.

use std::path::{Path, PathBuf};

use crate::cli;
use crate::cli::{Binary, Cli, ConfigAction, Role, RoleCmd, ServiceArgs};
use crate::cli_args::{ProtocolArg, ServiceAddr};
use crate::config_cmd;
use crate::lifecycle;
use crate::status_cmd;
use crate::usage::{self, UsageError};
use anyhow::Context as _;
use iroh_tunnel_host::config as config_ops;
use iroh_tunnel_host::config::SetError;
use iroh_tunnel_host::error::CliError;
use iroh_tunnel_host::runtime;
use iroh_tunnel_host::update;
use iroh_tunnel_host::Role as StatusRole;
use iroh_tunnel_host::RunContext;

/// Entry point of the all-in-one `iroh-tunnel` (`it`) binary.
pub fn main_all_in_one() -> ! {
    install_migration_notice();
    execute(parse_or_exit(Binary::AllInOne), Binary::AllInOne)
}

/// Entry point of the standalone `iroh-tunnel-serve` (`its`) binary.
pub fn main_serve() -> ! {
    install_migration_notice();
    execute(parse_or_exit(Binary::Serve), Binary::Serve)
}

/// Entry point of the standalone `iroh-tunnel-access` (`ita`) binary.
pub fn main_access() -> ! {
    install_migration_notice();
    execute(parse_or_exit(Binary::Access), Binary::Access)
}

/// Show the operator the macOS legacy-configuration copy on stderr; the host
/// only reports it through this observer. Runs before any path resolution.
fn install_migration_notice() {
    iroh_tunnel_host::paths::set_migration_notice(|legacy, dest| {
        eprintln!(
            "migrated configuration {} -> {} (original kept)",
            legacy.display(),
            dest.display()
        );
    });
}

/// Parse the process arguments; usage errors print the failing command's help
/// on stderr and exit 2, `--help`/`--version` exit 0.
fn parse_or_exit(binary: Binary) -> Cli {
    use clap::error::ErrorKind;
    let argv = std::env::args_os().collect::<Vec<_>>();
    match cli::parse_cli(binary, argv.clone()) {
        Ok(parsed) => parsed,
        Err(e) => match e.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => e.exit(),
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                eprint!("{}", e.render());
                std::process::exit(2);
            }
            _ => {
                let path = usage::deepest_command_path(&cli::build_command(binary), &argv);
                std::process::exit(usage::print_usage_error(
                    binary,
                    &path,
                    &usage::clap_error_summary(&e),
                ));
            }
        },
    }
}

/// Command path of `config set` for the role of a host `SetError`.
fn config_set_path(binary: Binary, role: &str) -> Vec<String> {
    let tail = ["config", "set"].map(str::to_owned);
    if binary.is_standalone() {
        tail.to_vec()
    } else {
        std::iter::once(role.to_owned()).chain(tail).collect()
    }
}

/// Host of a quick-mode address; a bare port means loopback.
fn quick_host(addr: &ServiceAddr) -> String {
    addr.host.clone().unwrap_or_else(|| "127.0.0.1".to_owned())
}

/// Callback invoked once a configured `run` reports readiness, after the
/// CLI's own ready output. Used by the Windows service runtime.
pub(crate) type ReadyHook = Option<Box<dyn FnOnce() + Send>>;

fn execute(parsed: Cli, binary: Binary) -> ! {
    // The SCM starts the service with this marker; it owns tracing (a log
    // file, no console), so it takes over before the console tracing init.
    #[cfg(windows)]
    if std::env::var_os(iroh_tunnel_host::service::ENV_SCM).as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        crate::scm_runtime::run_as_service(parsed, binary)
    }
    cli::init_tracing(parsed.verbose, parsed.quiet);
    std::process::exit(run_to_code(parsed, binary, None, false))
}

/// Exit code for a failed command that is not a usage error.
fn exit_code_for(e: &anyhow::Error) -> i32 {
    match e.downcast_ref::<CliError>() {
        Some(CliError::Config(_)) => 2,
        Some(CliError::Permission(_)) => 3,
        Some(CliError::Iroh(_)) => 4,
        Some(CliError::Service(_)) => 5,
        Some(CliError::Conflict(_)) => 6,
        None => 1,
    }
}

/// Wrap the CLI's ready printer so `hook` runs right after it.
fn compose_ready(print: iroh_tunnel_host::OnReady, hook: ReadyHook) -> iroh_tunnel_host::OnReady {
    Box::new(move |report| {
        // A failed print aborts the start: never report Running after it.
        print(report)?;
        if let Some(hook) = hook {
            hook();
        }
        Ok(())
    })
}

/// Forward a failure text to `log` only when running as a service: there is
/// no console under the SCM, so stderr output never reaches the service log.
fn forward_failure(to_log: bool, text: &str, log: impl FnOnce(&str)) {
    if to_log {
        log(text);
    }
}

/// Production sink: the failure text as a `tracing` error event.
fn log_failure(to_log: bool, text: &str) {
    forward_failure(to_log, text, |t| tracing::error!("{t}"));
}

/// Run the parsed command to completion and return its process exit code.
/// With `log_errors` (service mode) every failure is also written through
/// `tracing`; stdout/stderr output is identical either way.
pub(crate) fn run_to_code(parsed: Cli, binary: Binary, ready: ReadyHook, log_errors: bool) -> i32 {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: failed to start runtime: {e}");
            log_failure(log_errors, &format!("failed to start runtime: {e}"));
            return 1;
        }
    };

    // Role to clean up if shutdown is abandoned; only `run` commands own
    // a status file, but removal is pid-checked so other commands are safe.
    let role_name = match &parsed.role {
        Role::Serve { .. } => Some("serve"),
        Role::Access { .. } => Some("access"),
        Role::Update(_) => None,
    };
    let status_role = match &parsed.role {
        Role::Serve { .. } => Some(StatusRole::Serve),
        Role::Access { .. } => Some(StatusRole::Access),
        Role::Update(_) => None,
    };
    // Racing the run future means a forced exit first DROPS it: owned tasks
    // and the endpoint are cancelled before the process status is decided.
    // One drain context per invocation. Management commands never begin a
    // drain, so `forced_exit` stays pending and installs no signal handler.
    let ctx = RunContext::new();
    let outcome = rt.block_on(async {
        tokio::select! {
            result = run(parsed, binary, ctx.clone(), ready) => Ok(result),
            forced = lifecycle::forced_exit(&ctx) => Err(forced),
        }
    });
    let outcome = match outcome {
        Ok(result) => result,
        Err(forced) => {
            // Exit statuses: 130 repeated stop after grace, 1 watchdog (see
            // CHANGELOG). Own-state cleanup is still attempted here.
            eprintln!("error: shutdown abandoned ({forced:?}), exiting");
            log_failure(
                log_errors,
                &format!("shutdown abandoned ({forced:?}), exiting"),
            );
            if let Some(role) = status_role {
                runtime::remove_own_status_file_for_exit(role);
            }
            // Do not wait for blocking tasks: an abandoned shutdown must
            // never hang here.
            rt.shutdown_background();
            return forced.exit_code();
        }
    };
    match outcome {
        Ok(()) => 0,
        Err(e) => {
            if let Some(usage) = e.downcast_ref::<UsageError>() {
                log_failure(log_errors, &usage.message);
                return usage::print_usage_error(binary, &usage.command_path, &usage.message);
            }
            if let (Some(set_err), Some(role)) = (e.downcast_ref::<SetError>(), role_name) {
                log_failure(log_errors, &set_err.to_string());
                return usage::print_usage_error(
                    binary,
                    &config_set_path(binary, role),
                    &set_err.to_string(),
                );
            }
            eprintln!("error: {e:#}");
            log_failure(log_errors, &format!("{e:#}"));
            exit_code_for(&e)
        }
    }
}

async fn run(cli: Cli, binary: Binary, ctx: RunContext, ready: ReadyHook) -> anyhow::Result<()> {
    // The typed role tag comes from ONE match on the CLI enum; the dispatch
    // string is derived from it — an invalid role is unrepresentable, and
    // the two can never disagree.
    let status_role = match &cli.role {
        Role::Serve { .. } => StatusRole::Serve,
        Role::Access { .. } => StatusRole::Access,
        Role::Update(args) => {
            return run_update_cli(args, "iroh-tunnel").await;
        }
    };
    match cli.role {
        Role::Update(_) => unreachable!("handled above"),
        Role::Serve { cmd: Some(cmd), .. } => {
            dispatch_role_cmd(status_role, cmd, binary, ctx, ready).await
        }
        Role::Access { cmd: Some(cmd), .. } => {
            dispatch_role_cmd(status_role, cmd, binary, ctx, ready).await
        }
        Role::Serve { quick, cmd: None } => {
            let svc = quick.service;
            let (name, target) = svc
                .name
                .zip(svc.target)
                .context("internal: quick mode requires --name and --target")?;
            let protocol = svc.protocol.unwrap_or(ProtocolArg::Tcp).into();
            let secret_key = read_key_file(quick.key_file.as_deref())?;
            let cfg = iroh_tunnel_host::config::ServeConfig {
                node: iroh_tunnel_host::config::NodeConfig {
                    secret_key,
                    relay_urls: quick.relay_url.into_iter().collect(),
                    ..Default::default()
                },
                services: vec![iroh_tunnel_host::config::ServeService {
                    name,
                    protocol,
                    host: quick_host(&target),
                    port: target.port.get(),
                    udp: Default::default(),
                }],
            };
            runtime::run_quick_serve(
                cfg,
                ctx,
                crate::render::print_ready(),
                lifecycle::wait_for_signal(),
            )
            .await
        }
        Role::Access { quick, cmd: None } => {
            let svc = quick.service;
            let (name, listen, node_id) = svc
                .name
                .zip(svc.listen)
                .zip(svc.node_id)
                .map(|((n, l), i)| (n, l, i))
                .context("internal: quick mode requires --name, --listen and --node-id")?;
            let protocol = svc.protocol.unwrap_or(ProtocolArg::Tcp).into();
            node_id.parse::<iroh::EndpointId>().map_err(|e| {
                CliError::Config(format!("access quick mode: invalid --node-id: {e}"))
            })?;
            let secret_key = read_key_file(quick.key_file.as_deref())?;
            let cfg = iroh_tunnel_host::config::AccessConfig {
                node: iroh_tunnel_host::config::NodeConfig {
                    secret_key,
                    relay_urls: quick.relay_url.into_iter().collect(),
                    ..Default::default()
                },
                services: vec![iroh_tunnel_host::config::AccessService {
                    name,
                    node_id,
                    protocol,
                    host: quick_host(&listen),
                    port: listen.port.get(),
                    relay_urls: vec![],
                    udp: Default::default(),
                }],
            };
            runtime::run_quick_access(
                cfg,
                ctx,
                crate::render::print_ready(),
                lifecycle::wait_for_signal(),
            )
            .await
        }
    }
}

/// `update`: map the CLI options to a request, resolve the running
/// executable, honour its owner, then print the outcome.
async fn run_update_cli(args: &cli::UpdateArgs, exe_name: &str) -> anyhow::Result<()> {
    let request = update::UpdateRequest {
        to: args.to.clone(),
        verification: if args.insecure {
            update::Verification::Insecure
        } else {
            update::Verification::Verified
        },
        overwrite: if args.yes {
            update::OverwritePolicy::ReplacePackageOwned
        } else {
            update::OverwritePolicy::RespectOwner
        },
    };
    let target = update::current_target()?;
    let install_path = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .context("cannot locate the running executable")?;
    let base_url =
        std::env::var(update::BASE_URL_ENV).unwrap_or_else(|_| update::DEFAULT_BASE_URL.to_owned());
    let ctx = update::UpdateContext {
        base_url,
        exe_name,
        target,
        current_version: env!("APP_VERSION"),
        owner: iroh_tunnel_host::update::detect(&install_path),
        install_path,
        verifier: &update::SigstoreVerifier::release(),
    };
    match update::check_owner(&ctx, request.overwrite) {
        update::OwnerDecision::Delegate {
            guidance,
            overridable,
        } => {
            println!("{guidance}");
            if overridable {
                println!("to replace the file anyway, run again with --yes");
            }
            return Ok(());
        }
        update::OwnerDecision::Proceed {
            overriding_package: true,
        } => eprintln!(
            "warning: replacing {} although it is managed by a package manager; \
             the package database will no longer match the file",
            ctx.install_path.display()
        ),
        update::OwnerDecision::Proceed {
            overriding_package: false,
        } => {}
    }
    if request.verification == update::Verification::Insecure {
        eprintln!(
            "warning: --insecure skips signature and checksum verification; \
             you accept responsibility for the installed file"
        );
    }
    let outcome = update::update(&request, &ctx).await?;
    print!("{}", crate::render::render_update(&outcome, exe_name));
    Ok(())
}

async fn dispatch_role_cmd<S: ServiceArgs + Send + 'static>(
    status_role: StatusRole,
    cmd: RoleCmd<S>,
    binary: Binary,
    ctx: RunContext,
    ready: ReadyHook,
) -> anyhow::Result<()> {
    let role = status_role.name();
    match cmd {
        RoleCmd::Run { config } => {
            let path = resolve_config_path(role, config)?;
            // Existence only: the config is read once, by `load_config`,
            // which under a system service reads it through the trust check.
            config_ops::require_present(status_role, &path)?;
            // The typed role tag exhaustively selects the run handler —
            // the compiler enforces both arms, no string fallthrough.
            crate::test_fault::bound_run(runtime::run_foreground(
                status_role,
                &path,
                ctx,
                crate::test_fault::wrap_ready(compose_ready(crate::render::print_ready(), ready)),
                lifecycle::wait_for_signal(),
            ))
            .await
        }
        // Host operations are synchronous (and may prompt on stdin), so they
        // run on blocking workers, never on a runtime thread (RS-07).
        RoleCmd::Config { action } => {
            tokio::task::spawn_blocking(move || dispatch_config(status_role, action))
                .await
                .context("config command task failed")?
        }
        RoleCmd::Status { json } => {
            tokio::task::spawn_blocking(move || status_cmd::run(status_role, json))
                .await
                .context("status command task failed")?
        }
        RoleCmd::Update(args) => match binary.is_standalone() {
            true => {
                let exe = format!("iroh-tunnel-{role}");
                run_update_cli(&args, &exe).await
            }
            false => Err(CliError::Config(
                "update acts on the invoked executable; run `iroh-tunnel update`".into(),
            )
            .into()),
        },
        RoleCmd::Service { .. } if binary.is_standalone() => Err(CliError::Service(format!(
            "service management is not yet supported by the standalone {role} binary; \
             use `iroh-tunnel {role} service ...`"
        ))
        .into()),
        RoleCmd::Service { action } => {
            tokio::task::spawn_blocking(move || crate::cli_service::dispatch(status_role, action))
                .await
                .context("service command task failed")?
        }
    }
}

/// Resolve the config path: explicit `--config` wins, otherwise the
/// platform default from [`config_ops::resolve_config_path`].
fn resolve_config_path(role: &str, config: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    config_ops::resolve_config_path(role_named(role)?, config.as_deref())
}

/// The typed role for a CLI role name.
fn role_named(name: &str) -> anyhow::Result<StatusRole> {
    match name {
        "serve" => Ok(StatusRole::Serve),
        "access" => Ok(StatusRole::Access),
        other => anyhow::bail!("unknown role {other:?}"),
    }
}

fn dispatch_config<S: ServiceArgs>(
    role: StatusRole,
    action: ConfigAction<S>,
) -> anyhow::Result<()> {
    match action {
        ConfigAction::Keygen { print: true, .. } => config_cmd::keygen_random(),
        ConfigAction::Keygen { config, yes, .. } => {
            config_cmd::keygen_cli(role, config.as_deref(), yes)
        }
        ConfigAction::Set { service, config } => config_cmd::set(service, config),
        ConfigAction::Rm { config, name } => config_cmd::rm(role, config.as_deref(), &name),
        ConfigAction::List { config } => config_cmd::list(role, config.as_deref()),
        ConfigAction::Show { config, reveal } => config_cmd::show(role, config.as_deref(), reveal),
        ConfigAction::Edit { config } => config_cmd::edit_cli(role, config.as_deref()),
        ConfigAction::Path { config } => config_cmd::path(role, config.as_deref()),
        ConfigAction::Check { config } => config_cmd::check(role, config.as_deref()),
    }
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
    iroh_tunnel_host::config::resolve_secret_key(key).map_err(|_| {
        CliError::Config(format!(
            "--key-file {}: not a valid base64 32-byte secret key",
            path.display()
        ))
    })?;
    Ok(key.to_owned())
}

#[cfg(test)]
mod run_to_code_tests {
    use super::*;
    use iroh_tunnel_host::ReadyReport;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::Arc;

    #[test]
    fn exit_codes_map_cli_errors() {
        let cases = [
            (CliError::Config("x".into()), 2),
            (CliError::Permission("x".into()), 3),
            (CliError::Iroh("x".into()), 4),
            (CliError::Service("x".into()), 5),
            (CliError::Conflict("x".into()), 6),
        ];
        for (err, code) in cases {
            assert_eq!(exit_code_for(&anyhow::Error::new(err)), code);
        }
        assert_eq!(exit_code_for(&anyhow::anyhow!("other")), 1);
    }

    #[test]
    fn failures_reach_the_log_only_in_service_mode() {
        let seen = std::cell::RefCell::new(Vec::new());
        forward_failure(false, "foreground", |t| {
            seen.borrow_mut().push(t.to_owned())
        });
        assert!(seen.borrow().is_empty());
        forward_failure(true, "service", |t| seen.borrow_mut().push(t.to_owned()));
        assert_eq!(*seen.borrow(), vec!["service".to_owned()]);
    }

    #[test]
    fn failed_print_skips_the_hook() {
        let hook_ran = Arc::new(AtomicU8::new(0));
        let flag = hook_ran.clone();
        let print: iroh_tunnel_host::OnReady = Box::new(|_| anyhow::bail!("print failed"));
        let hook: ReadyHook = Some(Box::new(move || {
            flag.store(1, Ordering::SeqCst);
        }));
        let report = ReadyReport {
            node_id: "n".into(),
            home_relay: None,
            services: iroh_tunnel_host::ReadyServices::Serve(vec![]),
        };
        assert!(compose_ready(print, hook)(&report).is_err());
        assert_eq!(hook_ran.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn ready_hook_runs_after_print_and_propagates_result() {
        let step = Arc::new(AtomicU8::new(0));
        let (s1, s2) = (step.clone(), step.clone());
        let print: iroh_tunnel_host::OnReady = Box::new(move |_| {
            assert_eq!(s1.swap(1, Ordering::SeqCst), 0);
            Ok(())
        });
        let hook: ReadyHook = Some(Box::new(move || {
            assert_eq!(s2.swap(2, Ordering::SeqCst), 1);
        }));
        let report = ReadyReport {
            node_id: "n".into(),
            home_relay: None,
            services: iroh_tunnel_host::ReadyServices::Serve(vec![]),
        };
        compose_ready(print, hook)(&report).unwrap();
        assert_eq!(step.load(Ordering::SeqCst), 2);
    }
}
