//! CLI glue for `service` commands: builds requests for
//! [`iroh_tunnel_host::service`], and prints notes. Every verb acts in the
//! tier of this process's privilege; nothing is asked.

use std::path::PathBuf;

use anyhow::Context as _;

use crate::cli::{Binary, ServiceAction};
use crate::render::print_service_note;
use iroh_tunnel_host::service::Environment;
use iroh_tunnel_host::service::{
    self, InstallRequest, OutputMode, RunningExecutable, ServiceCall, ServiceInvocation,
};
use iroh_tunnel_host::Role;

/// The executable running this command, the source of a managed copy.
///
/// On Linux the bytes are read from `/proc/self/exe`, which opens the
/// executing file itself even if its path was replaced after exec; it is
/// never resolved with `read_link`.
///
/// # Errors
///
/// When the running executable cannot be resolved.
fn running_executable() -> anyhow::Result<RunningExecutable> {
    let display = std::env::current_exe().context("failed to resolve current executable")?;
    let read_from = if cfg!(target_os = "linux") {
        PathBuf::from("/proc/self/exe")
    } else {
        display.clone()
    };
    Ok(RunningExecutable { display, read_from })
}

/// The privilege of the running process, which selects the service tier.
/// It reads [`Tier::current`] (privilege in production), so the test-build
/// tier override applies here as everywhere else.
///
/// [`Tier::current`]: iroh_tunnel_host::Tier::current
pub(crate) fn detect_environment() -> Environment {
    Environment {
        elevated: iroh_tunnel_host::Tier::current() == iroh_tunnel_host::Tier::System,
    }
}

/// The executable a registration runs: the invoked standalone executable,
/// or the combined one found on `PATH`, falling back to this process's.
fn resolve_tunnel_executable_with(
    invocation: &ServiceInvocation,
    on_path: impl FnOnce() -> Option<PathBuf>,
    current_exe: impl FnOnce() -> anyhow::Result<PathBuf>,
) -> anyhow::Result<PathBuf> {
    match invocation {
        ServiceInvocation::Standalone { executable } => Ok(executable.clone()),
        ServiceInvocation::Combined => match on_path() {
            Some(path) => Ok(path),
            None => current_exe(),
        },
    }
}

/// The invocation of `binary`: a standalone executable is named by this
/// process's path, resolved only for it.
///
/// # Errors
///
/// When a standalone executable cannot resolve its own path.
pub(crate) fn invocation(binary: Binary) -> anyhow::Result<ServiceInvocation> {
    if !binary.is_standalone() {
        return Ok(ServiceInvocation::Combined);
    }
    let executable = std::env::current_exe().context("failed to resolve current executable")?;
    Ok(ServiceInvocation::Standalone { executable })
}

/// A service call for the CLI: native output goes straight to the terminal and
/// notes are printed as they happen.
fn cli_call(notes: &mut dyn FnMut(&service::ServiceNote)) -> ServiceCall<'_> {
    ServiceCall {
        output: OutputMode::Inherit,
        notes,
    }
}

/// Run one `service` action in this process's tier and print notes and
/// native output. Nothing is read from stdin.
pub(crate) fn dispatch(
    role: Role,
    action: ServiceAction,
    invocation: ServiceInvocation,
) -> anyhow::Result<()> {
    let env = detect_environment();
    let mut notes = |note: &service::ServiceNote| print_service_note(note, &invocation);
    let mut call = cli_call(&mut notes);
    match action {
        ServiceAction::Install { config } => {
            install(role, config, env, &mut call, &invocation, service::install)
        }
        ServiceAction::Uninstall => service::uninstall(role, &invocation, env, &mut call)
            .map(drop)
            .map_err(Into::into),
        ServiceAction::Start => service::start(role, &invocation, env, &mut call)
            .map(drop)
            .map_err(Into::into),
        ServiceAction::Stop => service::stop(role, &invocation, env, &mut call)
            .map(drop)
            .map_err(Into::into),
        ServiceAction::Restart => service::restart(role, &invocation, env, &mut call)
            .map(drop)
            .map_err(Into::into),
        ServiceAction::Status => service::status(role, &invocation, env, &mut call)
            .map(drop)
            .map_err(Into::into),
    }
}

/// The host install operation, injected so the CLI glue is testable without
/// a service manager.
type Installer = fn(
    &InstallRequest,
    Environment,
    &mut ServiceCall<'_>,
) -> Result<service::ServiceReport, service::ServiceFailure>;

/// Install in this process's tier through `installer`.
fn install(
    role: Role,
    config: Option<PathBuf>,
    env: Environment,
    call: &mut ServiceCall,
    invocation: &ServiceInvocation,
    installer: Installer,
) -> anyhow::Result<()> {
    let request = InstallRequest {
        invocation: invocation.clone(),
        role,
        config,
        tunnel_executable: resolve_tunnel_executable_with(
            invocation,
            || which::which("iroh-tunnel").ok(),
            || std::env::current_exe().context("failed to resolve current executable"),
        )?,
        running_executable: running_executable()?,
        state_dir_override: std::env::var_os("IROH_TUNNEL_STATE_DIR")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
    };
    installer(&request, env, call)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn standalone_resolution_ignores_combined_binary_on_path() {
        let path = resolve_tunnel_executable_with(
            &ServiceInvocation::Standalone {
                executable: PathBuf::from("/renamed standalone"),
            },
            || panic!("combined executable must not be selected"),
            || panic!("the invoked executable is already known"),
        )
        .unwrap();
        assert_eq!(path, PathBuf::from("/renamed standalone"));
    }

    use super::*;

    /// A host install that succeeds in the user tier without a manager.
    #[expect(
        clippy::panic_in_result_fn,
        reason = "the fixture asserts the request the CLI built"
    )]
    fn fixture_install(
        req: &InstallRequest,
        env: Environment,
        _call: &mut ServiceCall<'_>,
    ) -> Result<service::ServiceReport, service::ServiceFailure> {
        assert!(!env.elevated, "the user tier is not elevated");
        assert_eq!(req.role, Role::Serve);
        Ok(service::ServiceReport {
            applied: Vec::new(),
            native: Vec::new(),
        })
    }

    /// A non-elevated install never reads stdin: the child runs the CLI
    /// glue with stdin a pipe that is never written nor closed, so any read
    /// would block until the timeout.
    #[test]
    fn install_reads_nothing_from_stdin() {
        const NAME: &str = "cli_service::tests::install_reads_nothing_from_stdin";
        const CHILD: &str = "IROH_TUNNEL_TEST_STDIN_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let mut notes = |_: &service::ServiceNote| {};
            let mut call = cli_call(&mut notes);
            install(
                Role::Serve,
                Some(PathBuf::from("/cfg/serve.toml")),
                Environment { elevated: false },
                &mut call,
                &ServiceInvocation::Combined,
                fixture_install,
            )
            .unwrap();
            println!("IROH_STDIN_CHILD_DONE");
            return;
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--nocapture"])
            .env(CHILD, "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let _open_stdin = child.stdin.take();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                panic!("install blocked: it read stdin");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        let mut stdout = String::new();
        std::io::Read::read_to_string(&mut child.stdout.take().unwrap(), &mut stdout).unwrap();
        assert!(status.success(), "{stdout}");
        assert!(stdout.contains("IROH_STDIN_CHILD_DONE"), "{stdout}");
    }

    #[test]
    fn running_executable_reads_the_executing_file() {
        let running = running_executable().unwrap();
        assert_eq!(running.display, std::env::current_exe().unwrap());
        let expected = if cfg!(target_os = "linux") {
            PathBuf::from("/proc/self/exe")
        } else {
            running.display.clone()
        };
        assert_eq!(running.read_from, expected);
    }

    #[test]
    fn cli_resolves_tunnel_binary_from_path_first() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(if cfg!(windows) {
            "iroh-tunnel.exe"
        } else {
            "iroh-tunnel"
        });
        std::fs::write(&exe, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let found = resolve_tunnel_executable_with(
            &ServiceInvocation::Combined,
            || which::which_in("iroh-tunnel", Some(dir.path()), dir.path()).ok(),
            || panic!("current_exe must not be consulted when PATH has the binary"),
        )
        .unwrap();
        assert_eq!(found, exe);
        let fallback = resolve_tunnel_executable_with(
            &ServiceInvocation::Combined,
            || None,
            || Ok(PathBuf::from("/somewhere/current")),
        )
        .unwrap();
        assert_eq!(fallback, PathBuf::from("/somewhere/current"));
    }
}
