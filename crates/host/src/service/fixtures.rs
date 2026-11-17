//! Shared fixtures of the service unit tests.

use std::path::{Path, PathBuf};

use super::install::install_via;
use super::native::fake::{Event, Events, FakeRunner};
use super::native::{NativeOutput, ServiceCall};
#[cfg(unix)]
use super::status::status_via;
#[cfg(unix)]
use super::types::ServiceStatusReport;
use super::types::{
    Environment, InstallOutcome, InstallRequest, RunningExecutable, ServiceFailure, ServiceScope,
};
use crate::ownership::{LockDirs, LockResolver};
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
        invocation: super::ServiceInvocation::Combined,
        role: Role::Serve,
        scope,
        config: None,
        tunnel_executable: tunnel_exe(),
        running_executable: RunningExecutable {
            display: tunnel_exe(),
            read_from: tunnel_exe(),
        },
        sudo_user: None,
        invoking_user: None,
        state_dir_override: None,
        retarget: None,
    }
}

#[cfg(unix)]
pub(crate) const CAN_ANSWER: Environment = Environment {
    elevated: false,
    can_answer: true,
};

pub(crate) const BATCH: Environment = Environment {
    elevated: false,
    can_answer: false,
};

/// Run `install` against a fake runner; returns the result and the events.
///
/// The lock directories are two fresh tempdirs, so no test probes the real
/// machine or per-user lock directory.
pub(crate) fn run_install(
    req: &InstallRequest,
    env: Environment,
    responses: Vec<NativeOutput>,
) -> (Result<InstallOutcome, ServiceFailure>, Vec<Event>) {
    let user = tempfile::tempdir().unwrap();
    let machine = tempfile::tempdir().unwrap();
    let resolve = |_: Option<&Path>| {
        Ok(LockDirs {
            user: user.path().to_path_buf(),
            machine: Some(machine.path().to_path_buf()),
        })
    };
    run_install_in(req, env, responses, &resolve)
}

/// [`run_install`] over an explicit lock-directory resolver.
pub(crate) fn run_install_in(
    req: &InstallRequest,
    env: Environment,
    responses: Vec<NativeOutput>,
    lock_dirs: LockResolver<'_>,
) -> (Result<InstallOutcome, ServiceFailure>, Vec<Event>) {
    let events = Events::default();
    let mut fake = FakeRunner::new(&events, responses);
    let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
    let mut notes = crate::service::native::fake::note_sink(&events);
    let mut call = ServiceCall::capture(&mut notes);
    let result = install_via(req, env, &mut call, &mut runner, lock_dirs);
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
