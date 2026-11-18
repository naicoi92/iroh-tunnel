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
pub(crate) fn detect_environment() -> Environment {
    Environment {
        elevated: service::is_elevated(),
    }
}

/// Whether a person can answer a prompt (stdin is a terminal).
#[cfg(feature = "test-utils")]
pub(crate) fn is_interactive() -> bool {
    use std::io::IsTerminal;
    // Integration tests have no terminal; this stands in for one.
    std::env::var_os("IROH_TUNNEL_TEST_INTERACTIVE").is_some() || std::io::stdin().is_terminal()
}

#[cfg(not(feature = "test-utils"))]
pub(crate) fn is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
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
/// native output. The legacy `--user`/`--system` flags are still parsed but
/// no longer select anything (removed from the CLI separately).
pub(crate) fn dispatch(
    role: Role,
    action: ServiceAction,
    invocation: ServiceInvocation,
) -> anyhow::Result<()> {
    let env = detect_environment();
    let mut notes = |note: &service::ServiceNote| print_service_note(note, &invocation);
    let mut call = cli_call(&mut notes);
    match action {
        ServiceAction::Install { config, .. } => install(role, config, env, &mut call, &invocation),
        ServiceAction::Uninstall { .. } => service::uninstall(role, &invocation, env, &mut call)
            .map(drop)
            .map_err(Into::into),
        ServiceAction::Start { .. } => service::start(role, &invocation, env, &mut call)
            .map(drop)
            .map_err(Into::into),
        ServiceAction::Stop { .. } => service::stop(role, &invocation, env, &mut call)
            .map(drop)
            .map_err(Into::into),
        ServiceAction::Restart { .. } => service::restart(role, &invocation, env, &mut call)
            .map(drop)
            .map_err(Into::into),
        ServiceAction::Status { .. } => service::status(role, &invocation, env, &mut call)
            .map(drop)
            .map_err(Into::into),
    }
}

/// Install in this process's tier.
fn install(
    role: Role,
    config: Option<PathBuf>,
    env: Environment,
    call: &mut ServiceCall,
    invocation: &ServiceInvocation,
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
    service::install(&request, env, call)?;
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
