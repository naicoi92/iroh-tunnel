//! systemd (Linux) service backend.
//!
//! Manages systemd unit files under `/etc/systemd/system` (system scope) or
//! `~/.config/systemd/user` (user scope). Each action delegates to `systemctl`
//! with `--user` added automatically for the per-user domain.

use super::{resolve_binary, ServiceScope};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

fn unit_name(role: &str) -> String {
    format!("iroh-tunnel-{role}.service")
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
fn format_unit(role: &str, binary: &Path, config: &Path) -> String {
    let binary = binary.to_string_lossy();
    let config = config.to_string_lossy();
    format!(
        "[Unit]\n\
         Description=Iroh Tunnel ({role})\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={binary} {role} run --config {config}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
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

pub fn install(role: &str, scope: ServiceScope, config: &Path) -> Result<()> {
    let binary = resolve_binary()?;
    let unit = format_unit(role, &binary, config);
    let path = unit_path(role, scope)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(&path, &unit).with_context(|| format!("failed to write {}", path.display()))?;

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
#[allow(dead_code)]
fn _unit_name_is_stable() {
    // Compile-only guard: keep unit_name/format_unit referenced so a
    // refactor doesn't silently drop the template logic.
    let _ = unit_name("serve");
    let _ = format_unit("serve", Path::new("/x"), Path::new("/y"));
}
