//! Shared fixtures of the service unit tests.

use std::path::{Path, PathBuf};

use super::install::{install_via, InstallSeams};
use super::native::fake::{Event, Events, FakeRunner};
use super::native::{NativeOutput, ServiceCall};
use super::registration::RegistrationInput;
#[cfg(unix)]
use super::status::status_via;
#[cfg(unix)]
use super::types::ServiceStatusReport;
use super::types::{Environment, InstallRequest, RunningExecutable, ServiceFailure, ServiceReport};
use super::types::{RestartPolicy, ServiceInvocation};
use crate::role::Role;
use crate::Tier;

pub(crate) fn tunnel_exe() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\opt\x\iroh-tunnel.exe")
    } else {
        PathBuf::from("/opt/x/iroh-tunnel")
    }
}

/// The combined executable's invocation, borrowable for any lifetime.
pub(crate) static COMBINED: ServiceInvocation = ServiceInvocation::Combined;

/// A combined user registration of `role` with the default restart policy.
pub(crate) fn registration<'a>(
    role: &'a str,
    binary: &'a Path,
    config: &'a Path,
    state: &'a Path,
) -> RegistrationInput<'a> {
    RegistrationInput {
        role,
        invocation: &COMBINED,
        tier: Tier::User,
        binary,
        config,
        state,
        policy: &RestartPolicy::DEFAULT,
    }
}

pub(crate) fn request() -> InstallRequest {
    InstallRequest {
        invocation: super::ServiceInvocation::Combined,
        role: Role::Serve,
        config: None,
        tunnel_executable: tunnel_exe(),
        running_executable: RunningExecutable {
            display: tunnel_exe(),
            read_from: tunnel_exe(),
        },
        state_dir_override: None,
    }
}

/// A non-elevated caller: the user tier.
pub(crate) const BATCH: Environment = Environment { elevated: false };

/// An elevated caller: the system tier.
pub(crate) const ELEVATED: Environment = Environment { elevated: true };

/// The test environment (`IROH_TUNNEL_TEST_TRUSTED_OWNER` and nothing
/// else), the binary check `verify` and a presence step that records nothing.
pub(crate) fn seams<'a>(
    env: super::EnvLookup<'a>,
    verify: &'a dyn Fn(&Path) -> anyhow::Result<()>,
    presence: &'a dyn Fn(Role) -> anyhow::Result<()>,
) -> InstallSeams<'a> {
    InstallSeams {
        env,
        verify_binary: verify,
        ensure_presence: presence,
    }
}

/// Run `install` against a fake runner; returns the result and the events.
/// The presence step and the binary check are no-ops; the environment is
/// empty (no test owner).
pub(crate) fn run_install(
    req: &InstallRequest,
    env: Environment,
    responses: Vec<NativeOutput>,
) -> (Result<ServiceReport, ServiceFailure>, Vec<Event>) {
    let empty = |_: &str| None;
    let accept = |_: &Path| Ok(());
    let presence = |_: Role| Ok(());
    run_install_with(req, env, responses, &seams(&empty, &accept, &presence))
}

/// [`run_install`] over explicit host seams.
pub(crate) fn run_install_with(
    req: &InstallRequest,
    env: Environment,
    responses: Vec<NativeOutput>,
    seams: &InstallSeams<'_>,
) -> (Result<ServiceReport, ServiceFailure>, Vec<Event>) {
    let events = Events::default();
    let mut fake = FakeRunner::new(&events, responses);
    let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
    let mut notes = crate::service::native::fake::note_sink(&events);
    let mut call = ServiceCall::capture(&mut notes);
    let result = install_via(req, env, &mut call, &mut runner, seams);
    let log = events.borrow().clone();
    (result, log)
}

/// Run `verb` for serve against a fake runner; returns the result and the
/// events (commands and notes).
pub(crate) fn run_verb(
    verb: super::verb::Verb,
    env: Environment,
) -> (Result<ServiceReport, ServiceFailure>, Vec<Event>) {
    let events = Events::default();
    let mut fake = FakeRunner::new(&events, Vec::new());
    let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
    let mut notes = crate::service::native::fake::note_sink(&events);
    let mut call = ServiceCall::capture(&mut notes);
    let result = super::verb::verb_via(
        verb,
        Role::Serve,
        &ServiceInvocation::Combined,
        env,
        &mut call,
        &mut runner,
    );
    let log = events.borrow().clone();
    (result, log)
}

pub(crate) fn commands(log: &[Event]) -> Vec<&Event> {
    log.iter()
        .filter(|e| matches!(e, Event::Call { .. }))
        .collect()
}

#[cfg(unix)]
pub(crate) use crate::fixtures::in_isolated_home;

#[cfg(unix)]
pub(crate) fn write_valid_config(dir: &Path) -> PathBuf {
    let path = dir.join("serve.toml");
    std::fs::write(
        &path,
        "[node]\nrelay_urls = []\n\n[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = 9\n",
    )
    .unwrap();
    path
}

#[cfg(unix)]
pub(crate) fn run_status(
    responses: Vec<NativeOutput>,
) -> Result<ServiceStatusReport, ServiceFailure> {
    let events = Events::default();
    let mut fake = FakeRunner::new(&events, responses);
    let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
    let mut notes = crate::service::native::fake::note_sink(&events);
    let mut call = ServiceCall::capture(&mut notes);
    status_via(
        Role::Serve,
        &super::ServiceInvocation::Combined,
        BATCH,
        &mut call,
        &mut runner,
    )
}

pub(crate) fn blocking_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

pub(crate) fn ok_output(program: String) -> NativeOutput {
    let mut out = crate::service::native::fake::output(0, "", "");
    out.program = program;
    out
}

// Golden files pinning the exact bytes of every generated registration: each
// backend renders every role x invocation x tier case into one text and
// compares it with `tests/fixtures/registration/<name>`. Re-bless from the
// current code with `GOLDEN_BLESS=1 cargo test -p iroh-tunnel-host`.

/// One registration case of the golden matrix.
pub(crate) struct Case {
    pub role: &'static str,
    pub invocation: ServiceInvocation,
    pub tier: Tier,
}

impl Case {
    /// The header line separating this case in a golden file.
    pub(crate) fn header(&self) -> String {
        let invocation = match self.invocation {
            ServiceInvocation::Combined => "Combined",
            ServiceInvocation::Standalone { .. } => "Standalone",
        };
        format!(
            "==== role={} invocation={invocation} tier={:?} ====\n",
            self.role, self.tier
        )
    }
}

/// Every role × invocation × tier case, in a fixed order.
pub(crate) fn cases() -> Vec<Case> {
    let mut all = Vec::new();
    for role in ["serve", "access"] {
        let standalone = ServiceInvocation::Standalone {
            executable: PathBuf::from(format!("/opt/role bin/iroh-tunnel-{role}")),
        };
        for invocation in [ServiceInvocation::Combined, standalone] {
            for tier in [Tier::User, Tier::System] {
                all.push(Case {
                    role,
                    invocation: invocation.clone(),
                    tier,
                });
            }
        }
    }
    all
}

/// A Unix path set with characters every backend must quote or escape.
#[cfg(unix)]
pub(crate) struct UnixPaths {
    pub binary: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
}

#[cfg(unix)]
pub(crate) fn unix_paths(role: &str) -> UnixPaths {
    UnixPaths {
        binary: PathBuf::from("/opt/iroh tunnel/bin/iroh-tunnel"),
        config: PathBuf::from(format!("/home/a b/it's & <cfg>/{role} $x%i.toml")),
        state: PathBuf::from("/var/lib/iroh $state%n"),
    }
}

/// Compare `actual` with the golden file `name`, or write it under
/// `GOLDEN_BLESS=1`.
pub(crate) fn assert_golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/registration")
        .join(name);
    if std::env::var_os("GOLDEN_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {} ({e}); run with GOLDEN_BLESS=1",
            path.display()
        )
    });
    assert_eq!(actual, expected, "golden mismatch: {}", path.display());
}
