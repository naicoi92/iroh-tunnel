//! System-service management: install/uninstall/start/stop/restart/status.
//!
//! Implements T-12. The macOS backend is launchd. On Linux the backend is
//! chosen at RUNTIME (not compile time): systemd when the host boots with it
//! (detected via `/run/systemd/system`), otherwise a BusyBox/SysV init backend
//! that installs an `/etc/init.d/S??iroh-tunnel-<role>` script. The latter
//! covers embedded buildroot devices such as the Sipeed NanoKVM, whose busybox
//! init runs every executable `/etc/init.d/S*` script on boot. On Windows the
//! backend is a per-user Task Scheduler task that runs in the logged-in
//! session (user scope only; SM-04, issue #92).

use std::path::Path;

use anyhow::Result;

use crate::error::CliError;
#[allow(unused_imports)] // unused on targets without a service backend
use native::{system_runner, Native};
pub use scope::Environment;

pub mod account;
#[cfg(target_os = "linux")]
mod busybox;
#[cfg(target_os = "macos")]
mod launchd;
mod native;
pub use native::{InstalledAt, NativeOutput, OutputMode, ServiceCall, ServiceNote};
mod ops;
pub use ops::{
    install, restart, start, status, stop, uninstall, AppliedEffect, Fact, InstallNeed,
    InstallOutcome, InstallRequest, Registration, ServiceFailure, ServiceReport,
    ServiceStatusReport,
};
pub mod pin;
pub use pin::{Pin, RetargetConsent};
pub mod scope;
#[cfg(unix)]
pub mod system;
#[cfg(target_os = "linux")]
mod systemd;
mod task_xml;
#[cfg(unix)]
pub mod trust;
#[cfg(windows)]
mod windows;

/// Load a role config for `run`, enforcing the privileged trust policy when a
/// system service asks for it (SM-04). Plain read elsewhere.
///
/// # Errors
///
/// [`CliError::Permission`] for an unsafe path under a system service, or the
/// ordinary config errors.
pub(crate) fn load_config<C: crate::config::RoleDoc>(path: &Path) -> Result<C> {
    #[cfg(unix)]
    {
        trust::load_config(path)
    }
    #[cfg(not(unix))]
    {
        C::load(path)
    }
}

/// Restart policy shared by every platform's service template.
///
/// Both `systemd::format_unit` and `launchd::format_plist` read from this so
/// the policy has a single source of truth — the previous design hard-coded it
/// separately in each template (`Restart=on-failure`/`RestartSec=5` in the
/// unit, `KeepAlive SuccessfulExit=false` in the plist), which silently drifted
/// whenever one side was edited.
///
/// `delay_secs` is honored by systemd only; launchd manages its own throttle
/// interval internally and has no user-settable knob for it. The field stays
/// shared so the policy reads as one coherent thing, even if one platform
/// ignores the backoff.
#[allow(dead_code)] // unused on targets without a service backend
pub(crate) struct RestartPolicy {
    /// Restart the service when it exits with a non-zero status.
    pub on_failure: bool,
    /// Delay between restart attempts, in seconds. Read by systemd; ignored
    /// by launchd (which uses an internal ThrottleInterval).
    #[allow(dead_code)] // unread on macOS builds (launchd has no equivalent knob)
    pub delay_secs: u64,
}

impl RestartPolicy {
    /// The policy iroh-tunnel ships with: restart on crash, 5 s backoff.
    /// Matches the previous hard-coded values in both templates.
    #[allow(dead_code)] // unused on targets without a service backend
    pub const DEFAULT: RestartPolicy = RestartPolicy {
        on_failure: true,
        delay_secs: 5,
    };
}

/// Whether the service is installed system-wide or per-user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceScope {
    System,
    User,
}

// ---------------------------------------------------------------------------
// Public dispatchers — one per ServiceAction. They forward to the platform
// module selected at compile time. On Linux the choice between systemd and the
// BusyBox/SysV backend is made at runtime (see [`linux_is_systemd`]); on macOS
// it is always launchd. Each has the same cfg(linux)/cfg(macos)/
// cfg(not(...)) skeleton; they're kept explicit rather than factored into a
// macro because (a) `install` takes an extra `config` arg the others don't,
// (b) the unsupported fallback needs to consume the args to stay warning-free,
// and (c) six functions × three cfg arms reads more clearly than a macro that
// has to solve all of that generically.
// ---------------------------------------------------------------------------

/// Run `f` with a [`Native`] in the call's output mode and the call's note
/// observer.
#[allow(dead_code)] // unused on targets without a service backend
fn with_native<T>(
    call: &mut ServiceCall,
    f: impl FnOnce(&mut Native, &mut dyn FnMut(&ServiceNote)) -> T,
) -> T {
    let mut runner = system_runner;
    let mut native = Native::new(call.output, &mut runner);
    f(&mut native, &mut *call.notes)
}

/// The text of the currently installed system registration, if any: where a
/// previous install's pin is read from to detect a retarget (SM-02).
pub fn existing_registration(role: &str, scope: ServiceScope) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            systemd::existing_registration(role, scope)
        } else {
            let _ = scope;
            busybox::existing_registration(role)
        }
    }
    #[cfg(target_os = "macos")]
    {
        launchd::existing_registration(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        None
    }
}

// ---------------------------------------------------------------------------
// Registration helpers shared by the backends.
// ---------------------------------------------------------------------------

/// Name suffix that keeps a test registration apart from any real service.
/// Empty in production builds; `test-utils` reads it from the environment.
#[allow(dead_code)] // unused on targets without a service backend
fn instance_suffix() -> String {
    #[cfg(feature = "test-utils")]
    if let Some(suffix) = std::env::var_os("IROH_TUNNEL_TEST_SERVICE_SUFFIX") {
        return suffix.to_string_lossy().into_owned();
    }
    String::new()
}

/// Write a unit/plist/script, creating its directory.
///
/// # Errors
///
/// [`CliError::Permission`] when the OS denies access. The scope and account
/// are left as requested and the message says how to proceed, because
/// retrying under another scope or account would silently broaden access.
#[allow(dead_code)] // unused on targets without a service backend
pub(super) fn write_registration(path: &Path, contents: &str) -> Result<()> {
    let denied = |e: std::io::Error, what: &str, target: &Path| -> anyhow::Error {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            CliError::Permission(format!(
                "permission denied {what} {}: {e}. The service scope and account were not \
                 changed and nothing was registered. Fix the permissions of that location and \
                 re-run; use --system (as root) only if you intend a system service",
                target.display()
            ))
            .into()
        } else {
            anyhow::Error::new(e).context(format!("failed {what} {}", target.display()))
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| denied(e, "creating", parent))?;
    }
    // Atomic and never group/world-writable whatever the umask: a root-owned
    // unit that a group could edit would be a path to root (SM-04), and a
    // crash mid-write must not leave a truncated unit.
    let temp = path.with_extension(format!("tmp{}", std::process::id()));
    let written = (|| {
        std::fs::write(&temp, contents)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if path.extension().is_none() {
                0o755
            } else {
                0o644
            };
            std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(mode))?;
        }
        std::fs::rename(&temp, path)
    })();
    written.map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        denied(e, "writing", path)
    })
}

/// The text of an installed registration. Absent is `None`; a registration
/// that exists but cannot be read counts as one with an unknown owner (empty
/// text), never as "nothing installed", so the retarget prompt is not skipped.
#[allow(dead_code)] // unused on targets without a service backend
pub(super) fn read_registration(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(String::new()),
    }
}

// ---------------------------------------------------------------------------
// Linux init-system detection.
// ---------------------------------------------------------------------------

/// True when systemd is the running init (PID 1). Used to pick the systemd vs
/// BusyBox/SysV backend at runtime.
///
/// Primary signal: systemd creates `/run/systemd/system` when it manages the
/// boot. Fallback: read the target of `/proc/1/exe` (the kernel exposes PID 1's
/// executable there) and check it ends in `systemd`. Both are no-ops (return
/// false) on a non-systemd host and on non-Linux builds.
#[cfg(target_os = "linux")]
fn linux_is_systemd() -> bool {
    if std::path::Path::new("/run/systemd/system").exists() {
        return true;
    }
    std::fs::read_link("/proc/1/exe")
        .map(|p| p.ends_with("systemd"))
        .unwrap_or(false)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn unsupported<T>() -> Result<T> {
    anyhow::bail!(
        "service management is only supported on Linux (systemd/BusyBox init), macOS (launchd) \
         and Windows (user background task)"
    )
}

// Shared unit test covering the cross-platform ServiceScope + dispatch API
// shape, so the module is exercised on every platform (not just the host's).
#[cfg(test)]
mod tests {
    use super::ops::{install_via, verb_via, Verb};
    use super::*;
    use crate::error::CliError;
    use std::path::PathBuf;

    #[test]
    fn scope_variants_exist() {
        // Compile + equality smoke test for the public enum.
        assert_ne!(ServiceScope::System, ServiceScope::User);
    }

    #[test]
    fn restart_policy_default_matches_previous_hardcoded_values() {
        // Regression guard: the shared RestartPolicy must produce the same
        // values the per-platform templates used to hard-code (Restart=
        // on-failure, RestartSec=5 / KeepAlive SuccessfulExit=false).
        let p = RestartPolicy::DEFAULT;
        assert!(p.on_failure, "default policy must restart on failure");
        assert_eq!(p.delay_secs, 5, "default policy must keep the 5s backoff");
    }

    // ---- operations with policy (Task 14) --------------------------------

    use crate::service::native::fake::{Event, Events, FakeRunner};
    use crate::status::Role;

    fn tunnel_exe() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\opt\x\iroh-tunnel.exe")
        } else {
            PathBuf::from("/opt/x/iroh-tunnel")
        }
    }

    fn request(scope: Option<ServiceScope>) -> InstallRequest {
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

    const CAN_ANSWER: Environment = Environment {
        elevated: false,
        can_answer: true,
    };
    const BATCH: Environment = Environment {
        elevated: false,
        can_answer: false,
    };

    /// Run `install` against a fake runner; returns the result and the events.
    fn run_install(
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

    fn commands(log: &[Event]) -> Vec<&Event> {
        log.iter()
            .filter(|e| matches!(e, Event::Call { .. }))
            .collect()
    }

    /// Re-run the named test in a child process whose `HOME` is a fresh
    /// directory, because registration paths and role locks are keyed by
    /// `HOME` and this process is multi-threaded.
    fn in_isolated_home(name: &str, body: impl FnOnce(&Path)) {
        const CHILD_MARKER: &str = "IROH_TUNNEL_SERVICE_TEST_CHILD";
        if std::env::var_os(CHILD_MARKER).is_none() {
            let home = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", name, "--nocapture"])
                .env(CHILD_MARKER, "1")
                .env("HOME", home.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        body(&PathBuf::from(std::env::var_os("HOME").unwrap()));
    }

    fn write_valid_config(dir: &Path) -> PathBuf {
        let path = dir.join("serve.toml");
        std::fs::write(
            &path,
            "[node]\nrelay_urls = []\n\n[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = 9\n",
        )
        .unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn install_without_scope_returns_scope_choice_need() {
        let (result, log) = run_install(&request(None), CAN_ANSWER, vec![]);
        assert!(matches!(
            result,
            Ok(InstallOutcome::Needs(InstallNeed::ScopeChoice))
        ));
        assert!(commands(&log).is_empty(), "{log:?}");
    }

    #[cfg(unix)]
    #[test]
    fn unprivileged_system_install_is_refused_with_guidance() {
        let mut req = request(Some(ServiceScope::System));
        req.config = Some(PathBuf::from("/tmp/iroh-tunnel-test/serve.toml"));
        let (result, log) = run_install(&req, BATCH, vec![]);
        let err = anyhow::Error::from(result.expect_err("must be refused"));
        let Some(CliError::Permission(msg)) = err.downcast_ref::<CliError>() else {
            panic!("expected Permission, got {err:?}");
        };
        assert!(msg.contains("sudo"), "{msg}");
        assert!(msg.contains("serve service install --system"), "{msg}");
        assert!(msg.contains("/tmp/iroh-tunnel-test/serve.toml"), "{msg}");
        assert!(commands(&log).is_empty(), "{log:?}");
    }

    #[test]
    fn relative_tunnel_executable_is_rejected() {
        let mut req = request(Some(ServiceScope::User));
        req.tunnel_executable = PathBuf::from("iroh-tunnel");
        let (result, log) = run_install(&req, BATCH, vec![]);
        let err = anyhow::Error::from(result.expect_err("must be rejected"));
        assert!(
            matches!(err.downcast_ref::<CliError>(), Some(CliError::Service(_))),
            "{err:?}"
        );
        assert!(commands(&log).is_empty(), "{log:?}");
    }

    #[cfg(unix)]
    #[test]
    fn install_renders_supplied_tunnel_executable() {
        const NAME: &str = "service::tests::install_renders_supplied_tunnel_executable";
        in_isolated_home(NAME, |home| {
            #[cfg(target_os = "linux")]
            if !linux_is_systemd() {
                return; // the BusyBox backend needs real root
            }
            let config = write_valid_config(home);
            let mut req = request(Some(ServiceScope::User));
            req.config = Some(config);
            req.state_dir_override = Some(home.join("state"));
            let (result, _log) = run_install(&req, BATCH, vec![]);
            let Ok(InstallOutcome::Installed(report)) = result else {
                panic!("install failed");
            };
            let Some(AppliedEffect::RegistrationWritten(path)) = report.applied.first() else {
                panic!("no registration written: {:?}", report.applied);
            };
            let body = std::fs::read_to_string(path).unwrap();
            assert!(body.contains("/opt/x/iroh-tunnel"), "{body}");
        });
    }

    #[cfg(unix)]
    #[test]
    fn partial_install_reports_written_registration() {
        const NAME: &str = "service::tests::partial_install_reports_written_registration";
        in_isolated_home(NAME, |home| {
            #[cfg(target_os = "linux")]
            if !linux_is_systemd() {
                return;
            }
            let config = write_valid_config(home);
            let mut req = request(Some(ServiceScope::User));
            req.config = Some(config);
            req.state_dir_override = Some(home.join("state"));
            // Linux: daemon-reload ok, `enable` fails. macOS: `bootout` ok,
            // `bootstrap` fails.
            let responses = if cfg!(target_os = "linux") {
                vec![
                    crate::service::native::fake::output(0, "", ""),
                    crate::service::native::fake::output(1, "", "boom"),
                ]
            } else {
                vec![
                    crate::service::native::fake::output(0, "", ""),
                    crate::service::native::fake::output(5, "", "boom"),
                ]
            };
            let (result, log) = run_install(&req, BATCH, responses);
            let failure = result.expect_err("the manager step must fail");
            assert!(
                matches!(
                    failure.applied.as_slice(),
                    [AppliedEffect::RegistrationWritten(_)]
                ),
                "{:?}",
                failure.applied
            );
            let programs: Vec<String> = commands(&log).iter().map(|e| format!("{e:?}")).collect();
            assert!(
                !programs
                    .iter()
                    .any(|c| c.contains("\"stop\"") || c.contains("\"disable\"")),
                "no rollback command may run: {programs:?}"
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn start_is_refused_before_manager_when_role_runs() {
        const NAME: &str = "service::tests::start_is_refused_before_manager_when_role_runs";
        in_isolated_home(NAME, |_| {
            let _claim = crate::ownership::claim(Role::Serve).unwrap();
            let events = Events::default();
            let mut fake = FakeRunner::new(&events, vec![]);
            let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
            let mut notes = crate::service::native::fake::note_sink(&events);
            let mut call = ServiceCall::capture(&mut notes);
            let result = verb_via(
                Verb::Start,
                Role::Serve,
                ServiceScope::User,
                BATCH,
                &mut call,
                &mut runner,
            );
            let err = anyhow::Error::from(result.expect_err("must be refused"));
            assert!(
                matches!(err.downcast_ref::<CliError>(), Some(CliError::Conflict(_))),
                "{err:?}"
            );
            assert!(commands(&events.borrow()).is_empty());
        });
    }

    #[cfg(unix)]
    #[test]
    fn abandoned_install_at_decision_changes_nothing() {
        const NAME: &str = "service::tests::abandoned_install_at_decision_changes_nothing";
        in_isolated_home(NAME, |home| {
            let config = write_valid_config(home);
            let before = std::fs::read(&config).unwrap();
            let registration_before = existing_registration("serve", ServiceScope::System);
            let env = Environment {
                elevated: true,
                can_answer: true,
            };
            let (result, log) = run_install(&request(Some(ServiceScope::System)), env, vec![]);
            assert!(matches!(
                result,
                Ok(InstallOutcome::Needs(InstallNeed::InvokingUser))
            ));
            assert!(commands(&log).is_empty(), "{log:?}");
            assert_eq!(std::fs::read(&config).unwrap(), before);
            assert_eq!(
                existing_registration("serve", ServiceScope::System),
                registration_before
            );
        });
    }

    #[cfg(windows)]
    #[test]
    fn windows_system_scope_is_service_error() {
        let env = Environment {
            elevated: true,
            can_answer: false,
        };
        let (result, _) = run_install(&request(Some(ServiceScope::System)), env, vec![]);
        let err = anyhow::Error::from(result.expect_err("must be refused"));
        let Some(CliError::Service(msg)) = err.downcast_ref::<CliError>() else {
            panic!("expected Service, got {err:?}");
        };
        assert!(msg.contains("--user"), "{msg}");
    }

    // ---- structured status (Task 15) -------------------------------------

    fn run_status(responses: Vec<NativeOutput>) -> Result<ServiceStatusReport, ServiceFailure> {
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, responses);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut notes = crate::service::native::fake::note_sink(&events);
        let mut call = ServiceCall::capture(&mut notes);
        ops::status_via(
            Role::Serve,
            ServiceScope::User,
            BATCH,
            &mut call,
            &mut runner,
        )
    }

    #[cfg(unix)]
    #[test]
    fn status_reports_foreground_role_without_registration() {
        const NAME: &str = "service::tests::status_reports_foreground_role_without_registration";
        in_isolated_home(NAME, |_| {
            let _claim = crate::ownership::claim(Role::Serve).unwrap();
            let report = run_status(vec![]).unwrap();
            assert!(matches!(
                report.registration,
                Fact::Known(Registration::Absent)
            ));
            assert!(matches!(
                report.liveness,
                Fact::Known(crate::ownership::Liveness::Running { .. })
            ));
        });
    }

    #[cfg(unix)]
    #[test]
    fn status_reports_missing_owner_as_unavailable() {
        const NAME: &str = "service::tests::status_reports_missing_owner_as_unavailable";
        in_isolated_home(NAME, |home| {
            let _claim = crate::ownership::claim(Role::Serve).unwrap();
            std::fs::remove_file(home.join(".local/state/iroh-tunnel/serve.owner.json")).unwrap();
            let report = run_status(vec![]).expect("a missing owner is not a failure");
            assert!(matches!(report.owner, Fact::Unavailable(_)), "{report:?}");
            assert!(matches!(
                report.liveness,
                Fact::Known(crate::ownership::Liveness::Running { .. })
            ));
        });
    }

    #[cfg(unix)]
    #[test]
    fn status_does_not_parse_native_prose() {
        const NAME: &str = "service::tests::status_does_not_parse_native_prose";
        in_isolated_home(NAME, |_| {
            // Nothing runs the role, whatever the localized native text says.
            let prose = crate::service::native::fake::output(0, "aktiv (läuft)\n", "");
            let report = run_status(vec![prose]).unwrap();
            assert!(matches!(
                report.liveness,
                Fact::Known(crate::ownership::Liveness::NotRunning)
            ));
            let native = report.native.expect("native output is returned");
            assert_eq!(native.stdout, "aktiv (läuft)\n".as_bytes());
        });
    }

    // Compile-only guard: on Linux the runtime probe must exist and be bool.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_is_systemd_is_bool() {
        let _ = linux_is_systemd();
    }
}
