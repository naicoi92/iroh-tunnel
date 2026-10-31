//! systemd (Linux) service backend.
//!
//! Manages systemd unit files under `/etc/systemd/system` (system scope) or
//! `~/.config/systemd/user` (user scope). Each action delegates to `systemctl`
//! with `--user` added automatically for the per-user domain.

use super::{instance_suffix, resolve_binary, write_registration, RestartPolicy, ServiceScope};
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
fn format_unit(role: &str, binary: &Path, config: &Path, state: &Path) -> String {
    format_unit_with(role, binary, config, state, &RestartPolicy::DEFAULT)
}

fn format_unit_with(
    role: &str,
    binary: &Path,
    config: &Path,
    state: &Path,
    policy: &RestartPolicy,
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
    format!(
        "[Unit]\n\
         Description=Iroh Tunnel ({role})\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={binary} {role} run --config {config}\n\
         Environment={state_env}\n\
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
fn run_systemctl(scope: ServiceScope, args: &[&str]) -> Result<()> {
    let mut cmd = std::process::Command::new("systemctl");
    if scope == ServiceScope::User {
        cmd.arg("--user");
    }
    cmd.args(args);
    let status = cmd
        .status()
        .with_context(|| format!("failed to run systemctl {:?}", args))?;
    if !status.success() {
        bail!("systemctl {:?} failed with {status}", args);
    }
    Ok(())
}

pub fn install(role: &str, scope: ServiceScope, config: &Path, state: &Path) -> Result<()> {
    let binary = resolve_binary()?;
    let unit = format_unit(role, &binary, config, state);
    let path = unit_path(role, scope)?;
    write_registration(&path, &unit)?;

    let name = unit_name(role);
    run_systemctl(scope, &["daemon-reload"])?;
    run_systemctl(scope, &["enable", "--now", &name])?;
    println!("Installed and started {name} at {}", path.display());
    Ok(())
}

pub fn uninstall(role: &str, scope: ServiceScope) -> Result<()> {
    let name = unit_name(role);
    let path = unit_path(role, scope)?;
    // Best-effort stop/disable: the unit may not be loaded yet.
    let _ = run_systemctl(scope, &["stop", &name]);
    let _ = run_systemctl(scope, &["disable", &name]);
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
    }
    // Refresh systemd's view of the now-removed unit. A failure here leaves
    // stale state, so surface it rather than reporting success silently.
    run_systemctl(scope, &["daemon-reload"])?;
    println!("Uninstalled {name}");
    Ok(())
}

pub fn start(role: &str, scope: ServiceScope) -> Result<()> {
    run_systemctl(scope, &["start", &unit_name(role)])
}

pub fn stop(role: &str, scope: ServiceScope) -> Result<()> {
    run_systemctl(scope, &["stop", &unit_name(role)])
}

pub fn restart(role: &str, scope: ServiceScope) -> Result<()> {
    run_systemctl(scope, &["restart", &unit_name(role)])
}

pub fn status(role: &str, scope: ServiceScope) -> Result<()> {
    // `systemctl status` prints a human-readable status; surface it
    // directly by inheriting stdio.
    let mut cmd = std::process::Command::new("systemctl");
    if scope == ServiceScope::User {
        cmd.arg("--user");
    }
    cmd.args(["status", &unit_name(role)]);
    let status = cmd.status().context("failed to run systemctl status")?;
    // systemctl status returns non-zero when the service is stopped; that
    // is informative, not a hard failure, so don't bail.
    let _ = status;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
