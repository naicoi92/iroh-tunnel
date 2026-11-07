//! Measurement probe for design D1 of the `windows-system-service` change.
//!
//! Prints `EVIDENCE <label> <value>` lines describing the ACLs of the
//! directories a Windows system service would trust (the per-user config
//! chain, `C:\`, `%ProgramFiles%`, `%ProgramData%`, a freshly created
//! `%ProgramFiles%` subdirectory) and the current process token
//! (`whoami /user`, `whoami /groups /fo csv`). Only SIDs matter; localized
//! names are ignored.
//!
//! There are no assertions: a failing sub-command prints
//! `EVIDENCE <label> ERROR <status> <stderr>` and the probe continues.
//! Opt-in (`IROH_TUNNEL_E2E_SERVICE=1`) because it creates and removes a
//! directory under `%ProgramFiles%`. Run with `-- --nocapture` to see output.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Run a program and print every stdout line as evidence, or an ERROR line.
fn run(label: &str, program: &str, args: &[&str]) {
    match Command::new(program).args(args).output() {
        Ok(out) if out.status.success() => {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                println!("EVIDENCE {label} {line}");
            }
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).replace(['\r', '\n'], " ");
            println!("EVIDENCE {label} ERROR {} {}", out.status, err.trim());
        }
        Err(e) => println!("EVIDENCE {label} ERROR spawn {e}"),
    }
}

fn powershell(label: &str, script: &str) {
    run(label, "powershell", &["-NoProfile", "-Command", script]);
}

/// Print the SDDL of `path`, quoting it as a PowerShell literal.
fn sddl(label: &str, path: &Path) {
    let quoted = path.display().to_string().replace('\'', "''");
    powershell(label, &format!("(Get-Acl -LiteralPath '{quoted}').Sddl"));
}

/// Removes the probe directory on drop, including on early return or panic.
struct ProbeDir(PathBuf);

impl Drop for ProbeDir {
    fn drop(&mut self) {
        let quoted = self.0.display().to_string().replace('\'', "''");
        powershell(
            "probe-dir-cleanup",
            &format!("Remove-Item -LiteralPath '{quoted}' -Recurse -Force"),
        );
    }
}

/// Ancestors of `%LOCALAPPDATA%` from the drive root down to itself, then
/// `iroh-tunnel` and `iroh-tunnel\config` when they already exist.
fn config_chain(local_app_data: &Path) -> Vec<PathBuf> {
    let mut chain: Vec<PathBuf> = local_app_data.ancestors().map(Path::to_path_buf).collect();
    chain.retain(|p| p.file_name().is_some());
    chain.reverse();
    let app = local_app_data.join("iroh-tunnel");
    let config = app.join("config");
    chain.extend([app, config].into_iter().filter(|p| p.exists()));
    chain
}

#[test]
fn probe_windows_trust_assumptions() {
    if std::env::var("IROH_TUNNEL_E2E_SERVICE").as_deref() != Ok("1") {
        eprintln!("skipped: set IROH_TUNNEL_E2E_SERVICE=1 to probe Windows ACLs and token");
        return;
    }

    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        for (i, dir) in config_chain(Path::new(&local)).iter().enumerate() {
            sddl(&format!("config-chain[{i}] {}", dir.display()), dir);
        }
    } else {
        println!("EVIDENCE config-chain ERROR env LOCALAPPDATA unset");
    }

    sddl("drive-root C:\\", Path::new("C:\\"));
    for var in ["ProgramFiles", "ProgramData"] {
        match std::env::var_os(var) {
            Some(p) => sddl(var, Path::new(&p)),
            None => println!("EVIDENCE {var} ERROR env {var} unset"),
        }
    }

    if let Some(pf) = std::env::var_os("ProgramFiles") {
        let dir = Path::new(&pf).join(format!("iroh-tunnel-probe-{}", std::process::id()));
        let quoted = dir.display().to_string().replace('\'', "''");
        // Guard first so a partially created directory is still removed.
        let _guard = ProbeDir(dir.clone());
        powershell(
            "probe-dir-create",
            &format!("New-Item -ItemType Directory -Path '{quoted}' | Out-Null"),
        );
        sddl("probe-dir-sddl", &dir);
    }

    run("whoami-user", "whoami", &["/user"]);
    run("whoami-groups", "whoami", &["/groups", "/fo", "csv"]);
}
