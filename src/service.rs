//! System-service management: install/uninstall/start/stop/restart/status for
//! systemd (Linux) and launchd (macOS).
//!
//! Implements T-12. Each platform is a cfg-gated submodule exposing the same
//! six actions; the top-level functions dispatch to whichever platform the
//! binary was built for. Based on Page 06 v5 §1.3 (service subcommand) and
//! §1.3.2/§1.3.3 (unit/plist templates).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Whether the service is installed system-wide or per-user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceScope {
    System,
    User,
}

// ---------------------------------------------------------------------------
// Public dispatchers — one per ServiceAction. They forward to the platform
// module selected at compile time.
// ---------------------------------------------------------------------------

/// `service install`: write the unit/plist, enable, and start.
pub fn install(role: &str, scope: ServiceScope, config: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        systemd::install(role, scope, config)
    }
    #[cfg(target_os = "macos")]
    {
        launchd::install(role, scope, config)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope, config);
        unsupported()
    }
}

/// `service uninstall`: stop, disable, and remove the unit/plist.
pub fn uninstall(role: &str, scope: ServiceScope) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        systemd::uninstall(role, scope)
    }
    #[cfg(target_os = "macos")]
    {
        launchd::uninstall(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        unsupported()
    }
}

/// `service start`.
pub fn start(role: &str, scope: ServiceScope) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        systemd::start(role, scope)
    }
    #[cfg(target_os = "macos")]
    {
        launchd::start(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        unsupported()
    }
}

/// `service stop`.
pub fn stop(role: &str, scope: ServiceScope) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        systemd::stop(role, scope)
    }
    #[cfg(target_os = "macos")]
    {
        launchd::stop(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        unsupported()
    }
}

/// `service restart`.
pub fn restart(role: &str, scope: ServiceScope) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        systemd::restart(role, scope)
    }
    #[cfg(target_os = "macos")]
    {
        launchd::restart(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        unsupported()
    }
}

/// `service status`: print running/stopped (+ pid when available).
pub fn status(role: &str, scope: ServiceScope) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        systemd::status(role, scope)
    }
    #[cfg(target_os = "macos")]
    {
        launchd::status(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        unsupported()
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported() -> Result<()> {
    anyhow::bail!("service management is only supported on Linux (systemd) and macOS (launchd)")
}

// ---------------------------------------------------------------------------
// Resolve the iroh-tunnel binary path (shared across platforms).
// ---------------------------------------------------------------------------

/// Locate the `iroh-tunnel` executable to put in the unit/plist `ExecStart`.
///
/// Prefer `which` (so a system install is used); fall back to the current
/// executable so `cargo run -- service install` works during development.
fn resolve_binary() -> Result<PathBuf> {
    if let Ok(p) = which::which("iroh-tunnel") {
        return Ok(p);
    }
    let current = std::env::current_exe().context("failed to resolve current executable")?;
    Ok(current)
}

// ===========================================================================
// systemd (Linux)
// ===========================================================================
#[cfg(target_os = "linux")]
mod systemd {
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
        std::fs::write(&path, &unit)
            .with_context(|| format!("failed to write {}", path.display()))?;

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
}

// ===========================================================================
// launchd (macOS)
// ===========================================================================
//
// Default scope is per-user: `~/Library/LaunchAgents` under the user's own GUI
// login domain (`gui/<uid>`). This needs no privileges and matches how
// iroh-tunnel is normally used on a desktop — the same identity that owns the
// config under `$HOME` runs the service. Pass `--system` on the CLI to install
// a system-wide LaunchDaemon under `/Library/LaunchDaemons` instead (for
// servers / headless hosts; requires root).
//
// Uses the modern launchctl vocabulary (`bootstrap`/`bootout`/`kickstart`/
// `kill`/`print`) with explicit domain targets, instead of the legacy
// `load`/`unload`/`start`/`stop` which is being deprecated by Apple and which
// does not let us disambiguate the system domain from the caller's GUI
// domain. The legacy verbs were the root cause of the cryptic
// `exit status: 3` reported on `service start/stop` after a `sudo ... install`:
// `launchctl start <label>` searched the *caller's* domain, found nothing, and
// returned 3 — even though the job was loaded in the system domain.
#[cfg(target_os = "macos")]
mod launchd {
    use super::{resolve_binary, ServiceScope};
    use anyhow::{bail, Context, Result};
    use std::path::{Path, PathBuf};

    // No `libc`/`nix` dependency: declare the FFI directly. `getuid(2)` never
    // fails, so the `unsafe` is purely the Rust FFI formality.
    extern "C" {
        fn getuid() -> u32;
    }

    /// Effective user id of the calling process.
    fn uid() -> u32 {
        // SAFETY: getuid(2) has no failure mode and no preconditions.
        unsafe { getuid() }
    }

    /// Reverse-DNS job label: `dev.iroh-tunnel.{role}`.
    fn label(role: &str) -> String {
        format!("dev.iroh-tunnel.{role}")
    }

    fn plist_file(role: &str) -> String {
        format!("{}.plist", label(role))
    }

    /// Where the plist lives for the given scope.
    ///
    /// `User` → `~/Library/LaunchAgents`; `System` → `/Library/LaunchDaemons`.
    fn plist_path(role: &str, scope: ServiceScope) -> Result<PathBuf> {
        Ok(match scope {
            ServiceScope::System => PathBuf::from("/Library/LaunchDaemons").join(plist_file(role)),
            ServiceScope::User => {
                let home = dirs::home_dir().context("no home directory")?;
                home.join("Library/LaunchAgents").join(plist_file(role))
            }
        })
    }

    /// Domain target for `bootstrap`/`bootout`: `system` for System scope,
    /// `gui/<uid>` for User scope (the per-user GUI login domain).
    fn domain_target(scope: ServiceScope) -> String {
        match scope {
            ServiceScope::System => "system".to_string(),
            ServiceScope::User => format!("gui/{}", uid()),
        }
    }

    /// Fully-qualified service target for `kickstart`/`kill`/`print`:
    /// `system/<label>` or `gui/<uid>/<label>`.
    fn service_target(role: &str, scope: ServiceScope) -> String {
        format!("{}/{}", domain_target(scope), label(role))
    }

    /// Bail clearly if an action targets the system domain but the caller is
    /// not root. `launchctl` would otherwise emit an opaque error. Only used
    /// for `install`/`uninstall`: the daily verbs (`start`/`stop`/`restart`)
    /// are usable for the default User scope without privileges, and a
    /// System-scope call simply surfaces launchctl's own error if it lacks
    /// permission.
    fn require_root_for_system(scope: ServiceScope) -> Result<()> {
        if scope == ServiceScope::System && uid() != 0 {
            bail!(
                "System-scope (`--system`) install/uninstall requires root. \
                 Re-run with `sudo` (LaunchDaemon under /Library/LaunchDaemons), \
                 or drop `--system` to use the default per-user LaunchAgent \
                 (no privileges needed)."
            );
        }
        Ok(())
    }

    /// Escape `&`, `<`, `>` for safe inclusion in a plist `<string>` element.
    /// (Paths/labels won't normally contain these, but a user `--config` path
    /// could, and an unescaped value would produce invalid XML.)
    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    /// Render the launchd plist for `iroh-tunnel {role} run`.
    fn format_plist(role: &str, binary: &Path, config: &Path) -> String {
        let label = xml_escape(&label(role));
        let binary = xml_escape(&binary.to_string_lossy());
        let role = xml_escape(role);
        let config = xml_escape(&config.to_string_lossy());
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyManifest-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \t<key>Label</key>\n\
             \t<string>{label}</string>\n\
             \t<key>ProgramArguments</key>\n\
             \t<array>\n\
             \t\t<string>{binary}</string>\n\
             \t\t<string>{role}</string>\n\
             \t\t<string>run</string>\n\
             \t\t<string>--config</string>\n\
             \t\t<string>{config}</string>\n\
             \t</array>\n\
             \t<key>RunAtLoad</key>\n\
             \t<true/>\n\
             \t<key>KeepAlive</key>\n\
             \t<dict>\n\
             \t\t<key>SuccessfulExit</key>\n\
             \t\t<false/>\n\
             \t</dict>\n\
             </dict>\n\
             </plist>\n"
        )
    }

    fn run_launchctl(args: &[&str]) -> Result<()> {
        let status = std::process::Command::new("launchctl")
            .args(args)
            .status()
            .with_context(|| format!("failed to run launchctl {:?}", args))?;
        if !status.success() {
            bail!("launchctl {:?} failed with {status}", args);
        }
        Ok(())
    }

    pub fn install(role: &str, scope: ServiceScope, config: &Path) -> Result<()> {
        require_root_for_system(scope)?;
        let binary = resolve_binary()?;
        let plist = format_plist(role, &binary, config);
        let path = plist_path(role, scope)?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        std::fs::write(&path, &plist)
            .with_context(|| format!("failed to write {}", path.display()))?;

        let target = service_target(role, scope);
        let domain = domain_target(scope);
        let path_str = path.to_string_lossy().into_owned();

        // If a job is already registered (re-install over an existing load),
        // bootout first so bootstrap doesn't fail with a confusing
        // "Input/output error" / "already loaded". This is best-effort: when
        // nothing is registered yet, launchctl prints "Boot-out failed: 3: No
        // such process" — expected, so swallow its stderr rather than alarm the
        // user.
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &target])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        // `bootstrap` reports concrete failures (bad plist, missing binary,
        // etc.) rather than the legacy `load`'s opaque `5: Input/output error`.
        if let Err(e) = run_launchctl(&["bootstrap", &domain, &path_str]) {
            eprintln!(
                "Hint: run `launchctl print {target}` or \
                 `launchctl bootstrap {domain} {path_str}` for richer diagnostics."
            );
            return Err(e).context("failed to load service into launchd");
        }

        println!(
            "Installed and started {} at {}",
            label(role),
            path.display()
        );
        Ok(())
    }

    pub fn uninstall(role: &str, scope: ServiceScope) -> Result<()> {
        require_root_for_system(scope)?;
        let path = plist_path(role, scope)?;
        // Best-effort bootout: the job may not be loaded.
        let _ = run_launchctl(&["bootout", &service_target(role, scope)]);
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
        println!("Uninstalled {}", label(role));
        Ok(())
    }

    pub fn start(role: &str, scope: ServiceScope) -> Result<()> {
        // `kickstart` starts the job now if stopped, or restarts it if running.
        run_launchctl(&["kickstart", &service_target(role, scope)])
            .with_context(|| not_loaded_hint(role, scope))
    }

    pub fn stop(role: &str, scope: ServiceScope) -> Result<()> {
        // SIGTERM triggers our graceful shutdown (src/shutdown.rs). Because the
        // plist's KeepAlive only respawns on *unsuccessful* exit, a clean
        // exit-0 leaves the service stopped, as intended.
        //
        // `launchctl kill` returns exit 3 ("No process to signal") when the job
        // is loaded but has no running process — i.e. the service is already
        // stopped. Like `systemctl stop` on an inactive unit, treat that as
        // success (idempotent) rather than an error, so operators can re-run
        // `stop` safely.
        let target = service_target(role, scope);
        let output = std::process::Command::new("launchctl")
            .args(["kill", "SIGTERM", &target])
            .output()
            .with_context(|| format!("failed to run launchctl kill SIGTERM {target}"))?;
        if output.status.success() {
            return Ok(());
        }
        // exit code 3 = "No process to signal" → already stopped, not an error.
        // macOS encodes launchd errors as 128 + subsystem-code; for `kill` on a
        // loaded-but-stopped job this surfaces as plain 3.
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No process to signal") || output.status.code() == Some(3) {
            println!("{} is already stopped.", label(role));
            return Ok(());
        }
        // Genuine failure (e.g. service not loaded at all): distinguish via a
        // probe so the hint is accurate.
        if !is_loaded(&target)? {
            bail!("{}", not_loaded_hint(role, scope));
        }
        bail!(
            "launchctl kill SIGTERM {target} failed with {status}: {stderr}",
            status = output.status
        )
    }

    /// Whether a job is currently registered in launchd for `target` (regardless
    /// of whether it has a running process). Used to distinguish "already
    /// stopped" from "never installed" when interpreting a `kill` failure.
    fn is_loaded(target: &str) -> Result<bool> {
        let status = std::process::Command::new("launchctl")
            .args(["print", target])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .with_context(|| format!("failed to run launchctl print {target}"))?;
        Ok(status.success())
    }

    pub fn restart(role: &str, scope: ServiceScope) -> Result<()> {
        // `kickstart -k` kills any running instance before starting a fresh
        // one. Without `-k`, kickstart is a no-op when the service is already
        // running (it only satisfies the "run now" condition), so a plain
        // `kickstart` would silently fail to restart a live service.
        run_launchctl(&["kickstart", "-k", &service_target(role, scope)])
            .with_context(|| not_loaded_hint(role, scope))
    }

    /// Hint shown when `kickstart`/`kill` fail: either the service was never
    /// installed in this scope, or it was installed in a *different* scope.
    fn not_loaded_hint(role: &str, scope: ServiceScope) -> String {
        let (this_scope, other_flag) = match scope {
            // Default is User, so the more common mismatch is "installed as
            // system, tried without --system".
            ServiceScope::User => ("user (LaunchAgent)", "--system"),
            ServiceScope::System => ("system (LaunchDaemon)", "(default, drop --system)"),
        };
        format!(
            "no service '{0}' is loaded in the {this_scope} domain. \
             Install it first with `iroh-tunnel {role} service install`, \
             or it may be installed in the other scope (try {other_flag}).",
            label(role)
        )
    }

    pub fn status(role: &str, scope: ServiceScope) -> Result<()> {
        // `launchctl print <target>` shows pid/state/last exit code. Read-only,
        // so no root requirement is enforced (system-domain prints may still be
        // partial without root, but that's informational, not fatal).
        let mut cmd = std::process::Command::new("launchctl");
        cmd.args(["print", &service_target(role, scope)]);
        let status = cmd.status().context("failed to run launchctl print")?;
        // Non-zero typically means the job isn't loaded; informative, not fatal.
        let _ = status;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn plist_is_valid_xml_and_has_label() {
            let body = format_plist(
                "serve",
                Path::new("/usr/local/bin/iroh-tunnel"),
                Path::new("/etc/iroh-tunnel/serve.toml"),
            );
            assert!(
                body.contains("<string>dev.iroh-tunnel.serve</string>"),
                "label missing"
            );
            assert!(body.contains("<key>RunAtLoad</key>"), "RunAtLoad missing");
            assert!(
                body.contains("/usr/local/bin/iroh-tunnel"),
                "binary path missing"
            );
            assert!(
                body.contains("/etc/iroh-tunnel/serve.toml"),
                "config path missing"
            );
        }

        #[test]
        fn plist_path_user_is_under_launchagents() {
            // We can't assert the exact home in CI, but the suffix is stable.
            let p = plist_path("serve", ServiceScope::User).unwrap();
            assert!(p.ends_with("Library/LaunchAgents/dev.iroh-tunnel.serve.plist"));
        }

        #[test]
        fn plist_escapes_xml_metacharacters_in_paths() {
            // A config path containing XML metacharacters must be escaped so the
            // plist stays valid XML (regression guard for the review fix).
            let body = format_plist(
                "serve",
                Path::new("/opt/a&b"),
                Path::new("/home/u<x>/serve.toml"),
            );
            assert!(
                body.contains("/opt/a&amp;b") && !body.contains("/opt/a&b<"),
                "binary path not escaped: {body}"
            );
            assert!(
                body.contains("&lt;x&gt;") && !body.contains("u<x>"),
                "config path not escaped: {body}"
            );
        }

        #[test]
        fn service_target_uses_system_domain_for_system_scope() {
            // Regression guard for the bug that caused `exit status: 3`: the
            // service target MUST be domain-qualified so launchctl looks in the
            // right domain, not the caller's GUI domain.
            assert_eq!(
                service_target("access", ServiceScope::System),
                "system/dev.iroh-tunnel.access"
            );
        }

        #[test]
        fn service_target_uses_gui_domain_for_user_scope() {
            // User scope targets the per-user GUI domain: gui/<uid>/<label>.
            let t = service_target("serve", ServiceScope::User);
            assert!(
                t.starts_with("gui/") && t.ends_with("/dev.iroh-tunnel.serve"),
                "unexpected user-scope target: {t}"
            );
        }

        #[test]
        fn domain_target_system_is_plain_system() {
            assert_eq!(domain_target(ServiceScope::System), "system");
        }
    }
}

// Shared unit test covering the cross-platform ServiceScope + dispatch API
// shape, so the module is exercised on every platform (not just the host's).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_variants_exist() {
        // Compile + equality smoke test for the public enum.
        assert_ne!(ServiceScope::System, ServiceScope::User);
    }

    #[test]
    fn resolve_binary_returns_a_path() {
        // In tests the current exe is always resolvable, so the fallback path
        // is exercised at minimum.
        let p = resolve_binary().expect("resolve_binary should not fail in tests");
        assert!(p.is_absolute(), "binary path should be absolute");
    }
}
