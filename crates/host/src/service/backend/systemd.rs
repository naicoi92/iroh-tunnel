//! systemd (Linux) service backend.
//!
//! Manages systemd unit files under `/etc/systemd/system` (system scope) or
//! `~/.config/systemd/user` (user scope). Each action delegates to `systemctl`
//! with `--user` added automatically for the per-user domain.

use super::super::effective_binary::{parse_systemd_show, EffectiveBinary};
use super::super::native::{InstalledAt, Native, NativeOutput, ServiceNote};
use super::super::pin::Pin;
use super::super::registration::{instance_suffix, write_registration};
use super::super::types::AppliedEffect;
use super::super::types::{RestartPolicy, ServiceScope};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

fn unit_name(role: &str) -> String {
    format!("iroh-tunnel-{role}{}.service", instance_suffix())
}

/// Where the unit file lives for the given scope.
fn unit_path(role: &str, scope: ServiceScope) -> Result<PathBuf> {
    Ok(match scope {
        ServiceScope::System => PathBuf::from("/etc/systemd/system").join(unit_name(role)),
        ServiceScope::User => {
            let home = dirs::home_dir().context("no home directory")?;
            home.join(".config/systemd/user").join(unit_name(role))
        }
    })
}

/// Render the systemd unit file body for `iroh-tunnel {role} run`.
///
/// The restart directives come from the shared [`RestartPolicy`] so they stay
/// in sync with the launchd plist template.
#[cfg(test)]
fn format_unit(role: &str, binary: &Path, config: &Path, state: &Path) -> String {
    format_unit_with(role, binary, config, state, &RestartPolicy::DEFAULT, None)
}

fn format_unit_with(
    role: &str,
    binary: &Path,
    config: &Path,
    state: &Path,
    policy: &RestartPolicy,
    pin: Option<&Pin>,
) -> String {
    let binary = quote_arg(&binary.to_string_lossy(), Expand::Command);
    let config = quote_arg(&config.to_string_lossy(), Expand::Command);
    let state_env = quote_arg(
        &format!("IROH_TUNNEL_STATE_DIR={}", state.to_string_lossy()),
        Expand::Environment,
    );
    // systemd spells restart-on-failure as `Restart=on-failure` (or `no` when
    // the policy disables it). RestartSec is the backoff in seconds.
    let restart = if policy.on_failure {
        "on-failure"
    } else {
        "no"
    };
    let restart_sec = policy.delay_secs;
    // A system service reads a user's files as root: say whose (SM-04).
    let pin_comment = pin.map_or_else(String::new, |p| format!("# {}\n", p.record()));
    // A SID pin is rejected by `install` before this runs.
    let trust_env = pin.and_then(Pin::unix_uid).map_or_else(String::new, |uid| {
        let var = format!("{}={uid}", super::super::trust::ENV_TRUSTED_UID);
        format!("Environment={}\n", quote_arg(&var, Expand::Environment))
    });
    format!(
        "{pin_comment}[Unit]\n\
         Description=Iroh Tunnel ({role})\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={binary} {role} run --config {config}\n\
         Environment={state_env}\n\
         {trust_env}\
         Restart={restart}\n\
         RestartSec={restart_sec}\n\
         RestartPreventExitStatus=6\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

/// Which systemd line a quoted word goes on; they expand differently.
#[derive(Clone, Copy)]
enum Expand {
    /// `ExecStart=`: `$VAR` is substituted, so a literal `$` is doubled.
    Command,
    /// `Environment=`: no `$` substitution, so `$` stays literal.
    Environment,
}

/// Quote one word for a systemd unit line so paths with spaces or special
/// characters stay one argument. `%` is doubled on both lines because systemd
/// expands specifiers everywhere in unit files.
fn quote_arg(arg: &str, line: Expand) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@+=:,./-".contains(c));
    if plain {
        return arg.to_owned();
    }
    let mut escaped = arg
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    if matches!(line, Expand::Command) {
        escaped = escaped.replace('$', "$$");
    }
    format!("\"{escaped}\"")
}

/// Run `systemctl [--user] <args...>`, bailing on non-zero exit.
fn run_systemctl(native: &mut Native, scope: ServiceScope, args: &[&str]) -> Result<()> {
    let output = systemctl(native, scope, args)
        .with_context(|| format!("failed to run systemctl {:?}", args))?;
    if !output.success() {
        bail!("systemctl {:?} failed with {}", args, output.status_text());
    }
    Ok(())
}

/// Run `systemctl [--user] <args...>` in the caller's output mode.
fn systemctl(
    native: &mut Native,
    scope: ServiceScope,
    args: &[&str],
) -> std::io::Result<NativeOutput> {
    let mut cmd = std::process::Command::new("systemctl");
    if scope == ServiceScope::User {
        cmd.arg("--user");
    }
    cmd.args(args);
    native.run(&mut cmd)
}

/// Where the unit file is (or would be).
pub(crate) fn registration_path(role: &str, scope: ServiceScope) -> Result<PathBuf> {
    unit_path(role, scope)
}

/// What systemd will run for the role's system unit, from the loaded unit.
pub(crate) fn effective_binary(native: &mut Native, role: &str) -> EffectiveBinary {
    let unit = unit_name(role);
    let mut cmd = std::process::Command::new("systemctl");
    cmd.args(["show", "-p", "LoadState", "-p", "ExecStart", &unit]);
    match native.output(&mut cmd) {
        Ok(out) if out.success() => parse_systemd_show(&String::from_utf8_lossy(&out.stdout)),
        Ok(out) => EffectiveBinary::Unknown(format!(
            "systemctl show {unit} failed with {}",
            out.status_text()
        )),
        Err(e) => EffectiveBinary::Unknown(format!("failed to run systemctl show {unit}: {e}")),
    }
}

/// The installed unit's text, if any (to read its pin).
pub(crate) fn existing_registration(role: &str, scope: ServiceScope) -> Option<String> {
    super::super::registration::read_registration(&unit_path(role, scope).ok()?)
}

#[allow(clippy::too_many_arguments)] // one registration's inputs
pub(crate) fn install(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
    binary: &Path,
    config: &Path,
    state: &Path,
    pin: Option<&Pin>,
) -> Result<()> {
    super::super::pin::require_unix_pin(pin)?;
    let unit = format_unit_with(role, binary, config, state, &RestartPolicy::DEFAULT, pin);
    let path = unit_path(role, scope)?;
    write_registration(&path, &unit)?;
    native.applied(AppliedEffect::RegistrationWritten(path.clone()));

    enable_and_start(native, notes, scope, &unit_name(role), &path)
}

/// Reload, enable and (re)start the written unit, then announce it.
fn enable_and_start(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    scope: ServiceScope,
    name: &str,
    path: &Path,
) -> Result<()> {
    run_systemctl(native, scope, &["daemon-reload"])?;
    run_systemctl(native, scope, &["enable", name])?;
    native.applied(AppliedEffect::Enabled);
    // `restart`, not `enable --now`: a re-install over a running unit must pick
    // up the new registration instead of leaving the old process running.
    run_systemctl(native, scope, &["restart", name])?;
    native.applied(AppliedEffect::Started);
    notes(&ServiceNote::Installed {
        name: name.to_owned(),
        at: InstalledAt::Path(path.to_owned()),
    });
    Ok(())
}

pub(crate) fn uninstall(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
) -> Result<()> {
    let name = unit_name(role);
    let path = unit_path(role, scope)?;
    // Best-effort stop/disable: the unit may not be loaded yet.
    if run_systemctl(native, scope, &["stop", &name]).is_ok() {
        native.applied(AppliedEffect::Stopped);
    }
    if run_systemctl(native, scope, &["disable", &name]).is_ok() {
        native.applied(AppliedEffect::Disabled);
    }
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
        native.applied(AppliedEffect::RegistrationRemoved(path));
    }
    // Refresh systemd's view of the now-removed unit. A failure here leaves
    // stale state, so surface it rather than reporting success silently.
    run_systemctl(native, scope, &["daemon-reload"])?;
    notes(&ServiceNote::Uninstalled { name });
    Ok(())
}

pub(crate) fn start(native: &mut Native, role: &str, scope: ServiceScope) -> Result<()> {
    run_systemctl(native, scope, &["start", &unit_name(role)])?;
    native.applied(AppliedEffect::Started);
    Ok(())
}

pub(crate) fn stop(
    native: &mut Native,
    _notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
) -> Result<()> {
    run_systemctl(native, scope, &["stop", &unit_name(role)])?;
    native.applied(AppliedEffect::Stopped);
    Ok(())
}

pub(crate) fn restart(
    native: &mut Native,
    _notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
) -> Result<()> {
    run_systemctl(native, scope, &["restart", &unit_name(role)])?;
    native.applied(AppliedEffect::Started);
    Ok(())
}

pub(crate) fn status(
    native: &mut Native,
    _notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
) -> Result<NativeOutput> {
    // `systemctl status` prints a human-readable status; in Inherit mode it
    // reaches the terminal directly, in Capture mode it is returned.
    // It exits non-zero (3) when the service is stopped; that is informative,
    // not a hard failure, so the status is not checked.
    systemctl(native, scope, &["status", &unit_name(role)])
        .context("failed to run systemctl status")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::native::fake::{call, note_sink, output, Event, Events, FakeRunner};
    use crate::service::native::{Native, OutputMode};
    use crate::service::{InstalledAt, ServiceNote};

    #[test]
    fn capture_mode_returns_native_output() {
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, vec![output(0, "active (running)\n", "")]);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Capture, &mut runner);
        let out = status(&mut native, &mut |_| {}, "serve", ServiceScope::User).unwrap();
        assert_eq!(out.stdout, b"active (running)\n");
        assert_eq!(
            *events.borrow(),
            vec![call(
                "systemctl",
                &["--user", "status", "iroh-tunnel-serve.service"],
                OutputMode::Capture
            )]
        );
    }

    #[test]
    fn effective_binary_runs_show_for_the_system_unit() {
        let events = Events::default();
        let loaded =
            "LoadState=loaded\nExecStart={ path=/usr/bin/x ; argv[]=/usr/bin/x ; status=0/0 }\n";
        let mut fake = FakeRunner::new(&events, vec![output(0, loaded, "")]);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        assert_eq!(
            effective_binary(&mut native, "serve"),
            EffectiveBinary::Binary(PathBuf::from("/usr/bin/x"))
        );
        assert_eq!(
            *events.borrow(),
            vec![call(
                "systemctl",
                &[
                    "show",
                    "-p",
                    "LoadState",
                    "-p",
                    "ExecStart",
                    "iroh-tunnel-serve.service"
                ],
                OutputMode::Capture
            )]
        );
    }

    #[test]
    fn effective_binary_command_failure_is_unknown() {
        let mut runner = |_: &mut std::process::Command, _| {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no systemctl",
            ))
        };
        let mut native = Native::new(OutputMode::Capture, &mut runner);
        assert!(matches!(
            effective_binary(&mut native, "serve"),
            EffectiveBinary::Unknown(_)
        ));
    }

    #[test]
    fn stopped_systemd_unit_status_is_not_failure() {
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, vec![output(3, "inactive (dead)\n", "")]);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        assert!(status(&mut native, &mut |_| {}, "serve", ServiceScope::System).is_ok());
    }

    #[test]
    fn notes_reach_observer_before_native_command() {
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, vec![]);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        let mut notes = note_sink(&events);
        let path = PathBuf::from("/etc/systemd/system/iroh-tunnel-serve.service");
        enable_and_start(
            &mut native,
            &mut notes,
            ServiceScope::System,
            "iroh-tunnel-serve.service",
            &path,
        )
        .unwrap();
        let log = events.borrow();
        assert_eq!(log.len(), 4, "{log:?}");
        // Re-install over a running unit must restart it: pin the order of the
        // trailing arguments, independent of the scope flag run_systemctl adds.
        let expected_tails: [&[&str]; 3] = [
            &["daemon-reload"],
            &["enable", "iroh-tunnel-serve.service"],
            &["restart", "iroh-tunnel-serve.service"],
        ];
        for (event, tail) in log[..3].iter().zip(expected_tails) {
            let Event::Call { args, .. } = event else {
                panic!("expected a systemctl call, got {event:?}");
            };
            let tail: Vec<String> = tail.iter().map(|s| (*s).to_string()).collect();
            assert!(args.ends_with(&tail), "{args:?}");
        }
        assert_eq!(
            log[3],
            Event::Note(ServiceNote::Installed {
                name: "iroh-tunnel-serve.service".into(),
                at: InstalledAt::Path(path)
            })
        );
    }

    #[test]
    fn unit_name_is_stable() {
        assert_eq!(unit_name("serve"), "iroh-tunnel-serve.service");
        assert_eq!(unit_name("access"), "iroh-tunnel-access.service");
    }

    #[test]
    fn format_unit_uses_default_restart_policy() {
        // Regression: the shared RestartPolicy::DEFAULT must produce the same
        // Restart=/RestartSec= values the template used to hard-code.
        let body = format_unit(
            "serve",
            Path::new("/usr/bin/iroh-tunnel"),
            Path::new("/etc/cfg"),
            Path::new("/var/state"),
        );
        assert!(body.contains("Restart=on-failure"), "body: {body}");
        assert!(body.contains("RestartSec=5"), "body: {body}");
        // A refused start (exit 6, role already running) must not restart-loop.
        assert!(body.contains("RestartPreventExitStatus=6"), "body: {body}");
        assert!(body.contains("ExecStart=/usr/bin/iroh-tunnel serve run --config /etc/cfg"));
    }

    #[test]
    fn unit_exempts_the_refused_start_status() {
        // Deliberately duplicates one assertion of the test above so the
        // systemd and launchd containment guards sit side by side.
        let body = format_unit(
            "serve",
            Path::new("/usr/bin/iroh-tunnel"),
            Path::new("/etc/cfg"),
            Path::new("/var/state"),
        );
        assert!(body.contains("RestartPreventExitStatus=6"), "body: {body}");
    }

    #[test]
    fn format_unit_with_disabled_policy_omits_restart() {
        let policy = RestartPolicy {
            on_failure: false,
            delay_secs: 0,
        };
        let body = format_unit_with(
            "serve",
            Path::new("/x"),
            Path::new("/y"),
            Path::new("/z"),
            &policy,
            None,
        );
        assert!(body.contains("Restart=no"), "body: {body}");
        assert!(body.contains("RestartSec=0"), "body: {body}");
    }

    #[test]
    fn unit_pins_state_dir_and_quotes_paths_with_spaces() {
        let body = format_unit(
            "serve",
            Path::new("/usr/bin/iroh-tunnel"),
            Path::new("/home/a b/serve.toml"),
            Path::new("/home/a b/state"),
        );
        assert!(
            body.contains(
                "ExecStart=/usr/bin/iroh-tunnel serve run --config \"/home/a b/serve.toml\""
            ),
            "{body}"
        );
        assert!(
            body.contains("Environment=\"IROH_TUNNEL_STATE_DIR=/home/a b/state\""),
            "{body}"
        );
    }

    #[test]
    fn dollar_is_doubled_in_exec_start_but_literal_in_environment() {
        let body = format_unit(
            "serve",
            Path::new("/usr/bin/iroh-tunnel"),
            Path::new("/home/$u/serve.toml"),
            Path::new("/home/$u/state"),
        );
        assert!(body.contains("--config \"/home/$$u/serve.toml\""), "{body}");
        assert!(
            body.contains("Environment=\"IROH_TUNNEL_STATE_DIR=/home/$u/state\""),
            "{body}"
        );
    }

    #[test]
    fn unit_path_system_scope_is_under_etc() {
        let p = unit_path("serve", ServiceScope::System).unwrap();
        assert_eq!(
            p,
            PathBuf::from("/etc/systemd/system/iroh-tunnel-serve.service")
        );
    }

    #[test]
    fn managed_path_renders_as_one_argument_in_each_backend() {
        let copy = "/Library/Application Support/iroh-tunnel/bin/serve/iroh-tunnel";
        let body = format_unit("serve", Path::new(copy), Path::new("/c"), Path::new("/s"));
        let exec = body
            .lines()
            .find_map(|line| line.strip_prefix("ExecStart="))
            .expect("the unit has an ExecStart line");
        // The space is quoted, so systemd splits no word out of the path.
        assert_eq!(exec, format!("\"{copy}\" serve run --config /c"));
        // How systemd reports that unit once loaded.
        let shown = format!(
            "LoadState=loaded\nExecStart={{ path={copy} ; argv[]={copy} serve run --config /c ; \
             ignore_errors=no ; start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; code=(null) ; \
             status=0/0 }}\n"
        );
        assert_eq!(
            parse_systemd_show(&shown),
            EffectiveBinary::Binary(PathBuf::from(copy))
        );
    }
}
