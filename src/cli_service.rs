//! CLI glue for `service` commands: builds requests, answers the structured
//! needs of [`iroh_tunnel_host::service`] with prompts, and prints notes.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::Context as _;

use crate::cli::{ScopeArgs, ServiceAction};
use crate::render::print_service_note;
use iroh_tunnel_host::error::CliError;
use iroh_tunnel_host::service::Environment;
use iroh_tunnel_host::service::{
    self, InstallNeed, InstallOutcome, InstallRequest, OutputMode, RunningExecutable, ServiceCall,
    ServiceInvocation, ServiceScope,
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

/// Probe the running process for the install-time scope decision.
pub(crate) fn detect_environment() -> Environment {
    Environment {
        elevated: service::is_elevated(),
        can_answer: is_interactive(),
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

fn resolve_tunnel_executable_with(
    invocation: ServiceInvocation,
    on_path: impl FnOnce() -> Option<PathBuf>,
    current_exe: impl FnOnce() -> anyhow::Result<PathBuf>,
) -> anyhow::Result<PathBuf> {
    match invocation {
        ServiceInvocation::Standalone => current_exe(),
        ServiceInvocation::Combined => match on_path() {
            Some(path) => Ok(path),
            None => current_exe(),
        },
    }
}

/// A service call for the CLI: native output goes straight to the terminal and
/// notes are printed as they happen.
fn cli_call(notes: &mut dyn FnMut(&service::ServiceNote)) -> ServiceCall<'_> {
    ServiceCall {
        output: OutputMode::Inherit,
        notes,
    }
}

fn explicit_scope(args: &ScopeArgs) -> Option<ServiceScope> {
    match (args.user, args.system) {
        (_, true) => Some(ServiceScope::System),
        (true, _) => Some(ServiceScope::User),
        _ => None,
    }
}

/// Run one `service` action: map flags to a request, answer the needs the
/// operation reports, and print notes and native output.
pub(crate) fn dispatch(
    role: Role,
    action: ServiceAction,
    invocation: ServiceInvocation,
) -> anyhow::Result<()> {
    let env = detect_environment();
    // Verbs on an existing service: no prompt, and the scope follows privilege
    // when it is not given.
    let existing = |args: &ScopeArgs| {
        explicit_scope(args).unwrap_or_else(|| service::default_scope(env.elevated))
    };
    let executable = std::env::current_exe().context("failed to resolve current executable")?;
    // Explicit paths are available even if registration fails after publishing
    // the copy. Default system paths may belong to another user: recover those
    // only from the registration's authoritative pin, not root's HOME.
    let config = match &action {
        ServiceAction::Install {
            config: Some(path), ..
        } => std::fs::canonicalize(path).ok(),
        _ => None,
    };
    let mut pending = None;
    let mut notes = |note: &service::ServiceNote| {
        if invocation == ServiceInvocation::Standalone {
            if matches!(note, service::ServiceNote::ManagedBinary { .. }) {
                pending = Some(note.clone());
                return;
            }
            if let service::ServiceNote::Installed {
                at: service::InstalledAt::Path(path),
                ..
            } = note
            {
                let pinned = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|body| service::Pin::find(&body));
                if let Some(managed) = pending.take() {
                    print_service_note(
                        &managed,
                        invocation,
                        &executable,
                        pinned
                            .as_ref()
                            .map(|pin| pin.config.as_path())
                            .or(config.as_deref()),
                    );
                }
            }
        }
        print_service_note(note, invocation, &executable, config.as_deref());
    };
    let mut call = cli_call(&mut notes);
    let result = match action {
        ServiceAction::Install { config, scope } => install(
            role,
            config,
            explicit_scope(&scope),
            env,
            &mut call,
            invocation,
        ),
        ServiceAction::Uninstall { scope } => service::uninstall_with_invocation(
            role,
            existing(&scope),
            env,
            &mut call,
            invocation,
            &executable,
        )
        .map(drop)
        .map_err(Into::into),
        ServiceAction::Start { scope } => service::start_with_invocation(
            role,
            existing(&scope),
            env,
            &mut call,
            invocation,
            &executable,
        )
        .map(drop)
        .map_err(Into::into),
        ServiceAction::Stop { scope } => service::stop_with_invocation(
            role,
            existing(&scope),
            env,
            &mut call,
            invocation,
            &executable,
        )
        .map(drop)
        .map_err(Into::into),
        ServiceAction::Restart { scope } => service::restart_with_invocation(
            role,
            existing(&scope),
            env,
            &mut call,
            invocation,
            &executable,
        )
        .map(drop)
        .map_err(Into::into),
        ServiceAction::Status { scope } => service::status_with_invocation(
            role,
            existing(&scope),
            env,
            &mut call,
            invocation,
            &executable,
        )
        .map(drop)
        .map_err(Into::into),
    };
    // A backend may fail after publishing the managed copy but before it can
    // announce the registration. Still report the copy before the error.
    if let Some(note) = pending {
        print_service_note(&note, invocation, &executable, config.as_deref());
    }
    result
}

/// Install, answering each [`InstallNeed`] with the prompt the CLI always had.
fn install(
    role: Role,
    config: Option<PathBuf>,
    scope: Option<ServiceScope>,
    env: Environment,
    call: &mut ServiceCall,
    invocation: ServiceInvocation,
) -> anyhow::Result<()> {
    let mut request = InstallRequest {
        invocation,
        role,
        scope,
        config,
        tunnel_executable: resolve_tunnel_executable_with(
            invocation,
            || which::which("iroh-tunnel").ok(),
            || std::env::current_exe().context("failed to resolve current executable"),
        )?,
        running_executable: running_executable()?,
        sudo_user: std::env::var("SUDO_USER").ok(),
        invoking_user: None,
        state_dir_override: std::env::var_os("IROH_TUNNEL_STATE_DIR")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
        retarget: None,
    };
    let mut input = std::io::stdin().lock();
    let mut prompt = std::io::stderr();
    loop {
        match service::install(&request, env, call)? {
            InstallOutcome::Installed(_) => return Ok(()),
            InstallOutcome::Needs(InstallNeed::ScopeChoice) => {
                request.scope = Some(ask_scope(&mut input, &mut prompt)?);
            }
            InstallOutcome::Needs(InstallNeed::InvokingUser) => {
                request.invoking_user = Some(ask_user(role, &mut input, &mut prompt)?);
            }
            InstallOutcome::Needs(InstallNeed::Retarget { previous, proposed }) => {
                request.retarget = Some(confirm_retarget(
                    previous,
                    proposed,
                    env.can_answer,
                    &mut input,
                    &mut prompt,
                )?);
            }
        }
    }
}

fn ask_scope(input: &mut impl BufRead, prompt: &mut impl Write) -> anyhow::Result<ServiceScope> {
    write!(
        prompt,
        "Install as a `user` service (no privileges, runs for your account) \
         or a `system` service (needs sudo/root)? [user]: "
    )?;
    prompt.flush()?;
    let mut answer = String::new();
    if input.read_line(&mut answer)? == 0 {
        return Err(CliError::Config(
            "no answer to the scope prompt; pass --user or --system".into(),
        )
        .into());
    }
    service::parse_scope_answer(&answer)
}

/// The user a person names for a system install. An unknown name is reported
/// by the retried install; no answer at all is reported here.
#[cfg(unix)]
fn ask_user(
    role: Role,
    input: &mut impl BufRead,
    prompt: &mut impl Write,
) -> anyhow::Result<String> {
    write!(
        prompt,
        "Which user's configuration should the system service use? (user name): "
    )?;
    prompt.flush()?;
    let mut answer = String::new();
    if input.read_line(&mut answer)? == 0 {
        return Err(service::unknown_user(
            "no answer to the user prompt",
            role.name(),
        ));
    }
    Ok(answer.trim().to_owned())
}

#[cfg(not(unix))]
fn ask_user(
    _role: Role,
    _input: &mut impl BufRead,
    _prompt: &mut impl Write,
) -> anyhow::Result<String> {
    Err(CliError::Service("system services are not supported on this platform".into()).into())
}

/// Ask before replacing a system registration that belongs to another user
/// (or whose owner cannot be told), or refuse without a terminal.
fn confirm_retarget(
    previous: Option<service::Pin>,
    proposed: service::Pin,
    can_answer: bool,
    input: &mut impl BufRead,
    prompt: &mut impl Write,
) -> anyhow::Result<service::RetargetConsent> {
    let owner = match &previous {
        Some(p) => format!(
            "user {} ({}, config {})",
            service::account_name(&p.owner),
            p.owner,
            p.config.display()
        ),
        None => "an unknown owner (no pin recorded)".to_owned(),
    };
    let question = format!(
        "A system service is already installed for {owner}. Retarget it to user {} (config {})? \
         Both users' config files are kept. [y/N]: ",
        service::account_name(&proposed.owner),
        proposed.config.display()
    );
    let declined = |why: &str| -> anyhow::Error {
        CliError::Service(format!(
            "{why}; the existing service and both users' config files are unchanged"
        ))
        .into()
    };
    if !can_answer {
        return Err(declined(&format!(
            "a system service already exists for {owner} and retargeting needs a confirmation \
             that cannot be asked without a terminal; run this install interactively"
        )));
    }
    write!(prompt, "{question}")?;
    prompt.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(service::RetargetConsent { previous, proposed })
    } else {
        Err(declined("retarget declined"))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn standalone_resolution_ignores_combined_binary_on_path() {
        let path = resolve_tunnel_executable_with(
            ServiceInvocation::Standalone,
            || panic!("combined executable must not be selected"),
            || Ok(PathBuf::from("/renamed standalone")),
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
            ServiceInvocation::Combined,
            || which::which_in("iroh-tunnel", Some(dir.path()), dir.path()).ok(),
            || panic!("current_exe must not be consulted when PATH has the binary"),
        )
        .unwrap();
        assert_eq!(found, exe);
        let fallback = resolve_tunnel_executable_with(
            ServiceInvocation::Combined,
            || None,
            || Ok(PathBuf::from("/somewhere/current")),
        )
        .unwrap();
        assert_eq!(fallback, PathBuf::from("/somewhere/current"));
    }
}
