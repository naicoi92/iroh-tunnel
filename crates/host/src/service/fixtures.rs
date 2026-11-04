//! Shared fixtures of the service unit tests.

use std::path::{Path, PathBuf};

use super::install::install_via;
use super::native::fake::{Event, Events, FakeRunner};
use super::native::{NativeOutput, ServiceCall};
use super::status::status_via;
use super::types::{
    Environment, InstallOutcome, InstallRequest, ServiceFailure, ServiceScope, ServiceStatusReport,
};
use crate::role::Role;

pub(crate) fn tunnel_exe() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\opt\x\iroh-tunnel.exe")
    } else {
        PathBuf::from("/opt/x/iroh-tunnel")
    }
}

pub(crate) fn request(scope: Option<ServiceScope>) -> InstallRequest {
    InstallRequest {
        role: Role::Serve,
        scope,
        config: None,
        tunnel_executable: tunnel_exe(),
        sudo_user: None,
        invoking_user: None,
        state_dir_override: None,
        retarget: None,
    }
}

pub(crate) const CAN_ANSWER: Environment = Environment {
    elevated: false,
    can_answer: true,
};

pub(crate) const BATCH: Environment = Environment {
    elevated: false,
    can_answer: false,
};

/// Run `install` against a fake runner; returns the result and the events.
pub(crate) fn run_install(
    req: &InstallRequest,
    env: Environment,
    responses: Vec<NativeOutput>,
) -> (Result<InstallOutcome, ServiceFailure>, Vec<Event>) {
    let events = Events::default();
    let mut fake = FakeRunner::new(&events, responses);
    let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
    let mut notes = crate::service::native::fake::note_sink(&events);
    let mut call = ServiceCall::capture(&mut notes);
    let result = install_via(req, env, &mut call, &mut runner);
    let log = events.borrow().clone();
    (result, log)
}

pub(crate) fn commands(log: &[Event]) -> Vec<&Event> {
    log.iter()
        .filter(|e| matches!(e, Event::Call { .. }))
        .collect()
}

/// Re-run the named test in a child process whose `HOME` is a fresh
/// directory, because registration paths and role locks are keyed by
/// `HOME` and this process is multi-threaded.
pub(crate) fn in_isolated_home(name: &str, body: impl FnOnce(&Path)) {
    const CHILD_MARKER: &str = "IROH_TUNNEL_SERVICE_TEST_CHILD";
    if std::env::var_os(CHILD_MARKER).is_none() {
        let home = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env(CHILD_MARKER, "1")
            .env("HOME", home.path())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "child failed:\n{stdout}\n{stderr}");
        assert!(
            stdout.contains(&format!("IROH_CHILD_BODY_RAN {name}")),
            "child did not run the test body (0 tests matched?):\n{stdout}\n{stderr}"
        );
        return;
    }
    body(&PathBuf::from(std::env::var_os("HOME").unwrap()));
    println!("IROH_CHILD_BODY_RAN {name}");
}

pub(crate) fn write_valid_config(dir: &Path) -> PathBuf {
    let path = dir.join("serve.toml");
    std::fs::write(
        &path,
        "[node]\nrelay_urls = []\n\n[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = 9\n",
    )
    .unwrap();
    path
}

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
        ServiceScope::User,
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
