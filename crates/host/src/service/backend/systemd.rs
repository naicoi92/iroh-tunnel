//! systemd (Linux) service backend.
//!
//! Manages systemd unit files under `/etc/systemd/system` (system tier) or
//! `~/.config/systemd/user` (user tier). Each action delegates to `systemctl`
//! with `--user` added automatically for the per-user domain.

use super::super::effective_binary::{parse_systemd_show, EffectiveBinary};
use super::super::native::{InstalledAt, Native, NativeOutput, ServiceNote};
use super::super::registration::{instance_suffix, write_registration, RegistrationInput};
use super::super::types::AppliedEffect;
use crate::Tier;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

fn unit_name(role: &str) -> String {
    format!("iroh-tunnel-{role}{}.service", instance_suffix())
}

/// Where the unit file lives for the given tier.
fn unit_path(role: &str, tier: Tier) -> Result<PathBuf> {
    Ok(match tier {
        Tier::System => {
            #[cfg(feature = "test-utils")]
            if let Some(dir) =
                std::env::var_os("IROH_TUNNEL_TEST_SYSTEMD_DIR").filter(|d| !d.is_empty())
            {
                return Ok(PathBuf::from(dir).join(unit_name(role)));
            }
            PathBuf::from("/etc/systemd/system").join(unit_name(role))
        }
        Tier::User => {
            let home = dirs::home_dir().context("no home directory")?;
            home.join(".config/systemd/user").join(unit_name(role))
        }
    })
}

/// Render the systemd unit file body for `iroh-tunnel {role} run`.
///
/// The restart directives come from the shared [`RestartPolicy`] so they stay
/// in sync with the launchd plist template.
fn format_unit(reg: &RegistrationInput<'_>) -> String {
    let RegistrationInput {
        role,
        binary,
        config,
        state,
        policy,
        ..
    } = *reg;
    let role_arg = reg
        .role_arg()
        .map_or_else(String::new, |role| format!("{role} "));
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
    // The service marker tells `run` a service manager started it.
    let marker = format!("{}=1", super::super::ENV_SERVICE);
    format!(
        "[Unit]\n\
         Description=Iroh Tunnel ({role})\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={binary} {role_arg}run --config {config}\n\
         Environment={state_env}\n\
         Environment={marker}\n\
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
fn run_systemctl(native: &mut Native, tier: Tier, args: &[&str]) -> Result<()> {
    let output = systemctl(native, tier, args)
        .with_context(|| format!("failed to run systemctl {:?}", args))?;
    if !output.success() {
        bail!("systemctl {:?} failed with {}", args, output.status_text());
    }
    Ok(())
}

/// Run `systemctl [--user] <args...>` in the caller's output mode.
fn systemctl(native: &mut Native, tier: Tier, args: &[&str]) -> std::io::Result<NativeOutput> {
    let mut cmd = std::process::Command::new("systemctl");
    if tier == Tier::User {
        cmd.arg("--user");
    }
    cmd.args(args);
    native.run(&mut cmd)
}

/// Where the unit file is (or would be).
pub(crate) fn registration_path(role: &str, tier: Tier) -> Result<PathBuf> {
    unit_path(role, tier)
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

pub(crate) fn install(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    reg: &RegistrationInput<'_>,
) -> Result<()> {
    let unit = format_unit(reg);
    let path = unit_path(reg.role, reg.tier)?;
    write_registration(&path, &unit)?;
    native.applied(AppliedEffect::RegistrationWritten(path.clone()));

    enable_and_start(native, notes, reg.tier, &unit_name(reg.role), &path)
}

/// Reload, enable and (re)start the written unit, then announce it.
fn enable_and_start(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    tier: Tier,
    name: &str,
    path: &Path,
) -> Result<()> {
    run_systemctl(native, tier, &["daemon-reload"])?;
    run_systemctl(native, tier, &["enable", name])?;
    native.applied(AppliedEffect::Enabled);
    // `restart`, not `enable --now`: a re-install over a running unit must pick
    // up the new registration instead of leaving the old process running.
    run_systemctl(native, tier, &["restart", name])?;
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
    tier: Tier,
) -> Result<()> {
    let name = unit_name(role);
    let path = unit_path(role, tier)?;
    // Best-effort stop/disable: the unit may not be loaded yet.
    if run_systemctl(native, tier, &["stop", &name]).is_ok() {
        native.applied(AppliedEffect::Stopped);
    }
    if run_systemctl(native, tier, &["disable", &name]).is_ok() {
        native.applied(AppliedEffect::Disabled);
    }
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
        native.applied(AppliedEffect::RegistrationRemoved(path));
    }
    // Refresh systemd's view of the now-removed unit. A failure here leaves
    // stale state, so surface it rather than reporting success silently.
    run_systemctl(native, tier, &["daemon-reload"])?;
    notes(&ServiceNote::Uninstalled { name });
    Ok(())
}

pub(crate) fn start(native: &mut Native, role: &str, tier: Tier) -> Result<()> {
    run_systemctl(native, tier, &["start", &unit_name(role)])?;
    native.applied(AppliedEffect::Started);
    Ok(())
}

pub(crate) fn stop(
    native: &mut Native,
    _notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    tier: Tier,
) -> Result<()> {
    run_systemctl(native, tier, &["stop", &unit_name(role)])?;
    native.applied(AppliedEffect::Stopped);
    Ok(())
}

pub(crate) fn restart(
    native: &mut Native,
    _notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    tier: Tier,
) -> Result<()> {
    run_systemctl(native, tier, &["restart", &unit_name(role)])?;
    native.applied(AppliedEffect::Started);
    Ok(())
}

pub(crate) fn status(
    native: &mut Native,
    _notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    tier: Tier,
) -> Result<NativeOutput> {
    // `systemctl status` prints a human-readable status; in Inherit mode it
    // reaches the terminal directly, in Capture mode it is returned.
    // It exits non-zero (3) when the service is stopped; that is informative,
    // not a hard failure, so the status is not checked.
    systemctl(native, tier, &["status", &unit_name(role)]).context("failed to run systemctl status")
}

#[cfg(test)]
mod tests {
    #[test]
    fn unit_matches_golden_for_every_case() {
        use super::super::super::fixtures::{assert_golden, cases, unix_paths};
        let mut all = String::new();
        for case in cases() {
            let paths = unix_paths(case.role);
            all.push_str(&case.header());
            all.push_str(&format_unit(&RegistrationInput {
                invocation: &case.invocation,
                tier: case.tier,
                ..registration(case.role, &paths.binary, &paths.config, &paths.state)
            }));
        }
        assert_golden("systemd.txt", &all);
    }

    // AC1: isolate system registrations without changing the production manager.
    #[cfg(feature = "test-utils")]
    #[test]
    fn system_registration_uses_isolated_test_directory() {
        const NAME: &str =
            "service::backend::systemd::tests::system_registration_uses_isolated_test_directory";
        super::super::super::fixtures::in_isolated_home(NAME, |home| {
            let root = home.join("system units");
            std::env::set_var("IROH_TUNNEL_TEST_SYSTEMD_DIR", &root);
            assert_eq!(
                unit_path("serve", Tier::System).unwrap(),
                root.join(unit_name("serve"))
            );
        });
    }

    #[test]
    fn invocation_form_survives_managed_paths_with_spaces() {
        for role in ["serve", "access"] {
            for (invocation, expected) in [
                (ServiceInvocation::Combined, format!("{role} run")),
                (
                    ServiceInvocation::Standalone {
                        executable: PathBuf::from("/opt/role bin/renamed"),
                    },
                    "run".to_owned(),
                ),
            ] {
                let body = format_unit(&RegistrationInput {
                    invocation: &invocation,
                    ..registration(
                        role,
                        Path::new("/managed path/copy"),
                        Path::new("/config path/c"),
                        Path::new("/state"),
                    )
                });
                assert!(
                    body.contains(&format!(
                        "ExecStart=\"/managed path/copy\" {expected} --config \"/config path/c\""
                    )),
                    "{body}"
                );
            }
        }
    }

    use super::super::super::fixtures::registration;
    use super::super::super::types::{RestartPolicy, ServiceInvocation};
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
        let out = status(&mut native, &mut |_| {}, "serve", Tier::User).unwrap();
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
        assert!(status(&mut native, &mut |_| {}, "serve", Tier::System).is_ok());
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
            Tier::System,
            "iroh-tunnel-serve.service",
            &path,
        )
        .unwrap();
        let log = events.borrow();
        assert_eq!(log.len(), 4, "{log:?}");
        // Re-install over a running unit must restart it: pin the order of the
        // trailing arguments, independent of the tier flag run_systemctl adds.
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
    fn systemd_unit_has_service_marker_and_no_pin() {
        let unit = format_unit(&RegistrationInput {
            tier: Tier::System,
            ..registration(
                "serve",
                Path::new("/usr/bin/iroh-tunnel"),
                Path::new("/etc/iroh-tunnel/serve.toml"),
                Path::new("/run/iroh-tunnel"),
            )
        });
        assert!(
            unit.contains("Environment=IROH_TUNNEL_SERVICE=1\n"),
            "{unit}"
        );
        assert!(
            unit.contains("Environment=IROH_TUNNEL_STATE_DIR=/run/iroh-tunnel\n"),
            "{unit}"
        );
        assert!(!unit.contains("iroh-tunnel-pin"), "{unit}");
        assert!(!unit.contains("IROH_TUNNEL_TRUSTED_UID"), "{unit}");
    }

    #[test]
    fn reinstall_restarts_a_running_unit() {
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, vec![]);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Capture, &mut runner);
        let path = PathBuf::from("/etc/systemd/system/iroh-tunnel-serve.service");
        enable_and_start(
            &mut native,
            &mut |_| {},
            Tier::System,
            "iroh-tunnel-serve.service",
            &path,
        )
        .unwrap();
        let issued: Vec<String> = events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                Event::Call { args, .. } => Some(args.join(" ")),
                Event::Note(_) => None,
            })
            .collect();
        assert_eq!(
            issued.last().map(String::as_str),
            Some("restart iroh-tunnel-serve.service"),
            "{issued:?}"
        );
        assert!(
            !issued.iter().any(|c| c.starts_with("start ")),
            "{issued:?}"
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
        let body = format_unit(&registration(
            "serve",
            Path::new("/usr/bin/iroh-tunnel"),
            Path::new("/etc/cfg"),
            Path::new("/var/state"),
        ));
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
        let body = format_unit(&registration(
            "serve",
            Path::new("/usr/bin/iroh-tunnel"),
            Path::new("/etc/cfg"),
            Path::new("/var/state"),
        ));
        assert!(body.contains("RestartPreventExitStatus=6"), "body: {body}");
    }

    #[test]
    fn format_unit_with_disabled_policy_omits_restart() {
        let policy = RestartPolicy {
            on_failure: false,
            delay_secs: 0,
        };
        let body = format_unit(&RegistrationInput {
            policy: &policy,
            ..registration("serve", Path::new("/x"), Path::new("/y"), Path::new("/z"))
        });
        assert!(body.contains("Restart=no"), "body: {body}");
        assert!(body.contains("RestartSec=0"), "body: {body}");
    }

    #[test]
    fn unit_pins_state_dir_and_quotes_paths_with_spaces() {
        let body = format_unit(&registration(
            "serve",
            Path::new("/usr/bin/iroh-tunnel"),
            Path::new("/home/a b/serve.toml"),
            Path::new("/home/a b/state"),
        ));
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
        let body = format_unit(&registration(
            "serve",
            Path::new("/usr/bin/iroh-tunnel"),
            Path::new("/home/$u/serve.toml"),
            Path::new("/home/$u/state"),
        ));
        assert!(body.contains("--config \"/home/$$u/serve.toml\""), "{body}");
        assert!(
            body.contains("Environment=\"IROH_TUNNEL_STATE_DIR=/home/$u/state\""),
            "{body}"
        );
    }

    #[test]
    fn unit_path_system_scope_is_under_etc() {
        let p = unit_path("serve", Tier::System).unwrap();
        assert_eq!(
            p,
            PathBuf::from("/etc/systemd/system/iroh-tunnel-serve.service")
        );
    }

    #[test]
    fn managed_path_renders_as_one_argument_in_each_backend() {
        let copy = "/Library/Application Support/iroh-tunnel/bin/serve/iroh-tunnel";
        let body = format_unit(&registration(
            "serve",
            Path::new(copy),
            Path::new("/c"),
            Path::new("/s"),
        ));
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
