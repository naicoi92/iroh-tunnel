//! launchd (macOS) service backend.
//!
//! User scope (`--user`) uses `~/Library/LaunchAgents` under the user's own GUI
//! login domain (`gui/<uid>`) and needs no privileges. System scope
//! (`--system`) uses a LaunchDaemon under `/Library/LaunchDaemons` and requires
//! root; the CLI refuses it without privilege before reaching this module.
//!
//! Uses the modern launchctl vocabulary (`bootstrap`/`bootout`/`kickstart`/
//! `kill`/`print`) with explicit domain targets, instead of the legacy
//! `load`/`unload`/`start`/`stop` which is being deprecated by Apple and which
//! does not let us disambiguate the system domain from the caller's GUI
//! domain. The legacy verbs were the root cause of the cryptic
//! `exit status: 3` reported on `service start/stop` after a `sudo ... install`:
//! `launchctl start <label>` searched the *caller's* domain, found nothing, and
//! returned 3 — even though the job was loaded in the system domain.
//!
//! Loading runs `launchctl enable <service-target>` first: launchd keeps a
//! persistent per-domain disable list, keyed by label, which survives
//! `bootout`, uninstall and reboot, and makes `bootstrap` fail with
//! `5: Input/output error`. `systemctl enable` holds the same position in the
//! systemd backend.

use super::super::effective_binary::{parse_launchctl_print, EffectiveBinary};
use super::super::native::{InstalledAt, Native, NativeOutput, ServiceNote};
use super::super::pin::Pin;
use super::super::registration::{instance_suffix, write_registration};
use super::super::types::AppliedEffect;
use super::super::types::{RestartPolicy, ServiceScope};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Real user id of the calling process.
fn uid() -> u32 {
    // SAFETY: getuid(2) has no failure mode and no preconditions.
    unsafe { libc::getuid() }
}

/// Reverse-DNS job label: `dev.iroh-tunnel.{role}` (plus the test-only
/// instance suffix, empty in production).
fn label(role: &str) -> String {
    format!("dev.iroh-tunnel.{role}{}", instance_suffix())
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

/// Escape `&`, `<`, `>` for safe inclusion in a plist `<string>` element.
/// (Paths/labels won't normally contain these, but a user `--config` path
/// could, and an unescaped value would produce invalid XML.)
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render the launchd plist for `iroh-tunnel {role} run`.
///
/// The KeepAlive block comes from the shared [`RestartPolicy`] so the
/// "restart on crash" behavior stays in sync with the systemd unit template.
#[cfg(test)]
fn format_plist(role: &str, binary: &Path, config: &Path, state: &Path) -> String {
    format_plist_with(role, binary, config, state, &RestartPolicy::DEFAULT, None)
}

fn format_plist_with(
    role: &str,
    binary: &Path,
    config: &Path,
    state: &Path,
    policy: &RestartPolicy,
    pin: Option<&Pin>,
) -> String {
    let label = xml_escape(&label(role));
    let binary = xml_escape(&binary.to_string_lossy());
    let role = xml_escape(role);
    let config = xml_escape(&config.to_string_lossy());
    let state = xml_escape(&state.to_string_lossy());
    // launchd spells restart-on-failure as KeepAlive{SuccessfulExit=false}:
    // "respawn unless the last exit was clean". When on_failure is disabled,
    // drop the KeepAlive block entirely (no respawn). delay_secs is not
    // expressible in launchd's KeepAlive dict — it's a systemd-specific knob
    // that launchd manages internally (ThrottleInterval, not user-set here).
    let keep_alive = if policy.on_failure {
        "\t<key>KeepAlive</key>\n\
         \t<dict>\n\
         \t\t<key>SuccessfulExit</key>\n\
         \t\t<false/>\n\
         \t</dict>\n"
    } else {
        ""
    };
    // A system daemon reads a user's files as root: say whose (SM-04).
    let pin_comment = pin.map_or_else(String::new, |p| format!("<!-- {} -->\n", p.record()));
    // A SID pin is rejected by `install` before this runs.
    let trust_env = pin.and_then(Pin::unix_uid).map_or_else(String::new, |uid| {
        format!(
            "\t\t<key>{}</key>\n\t\t<string>{uid}</string>\n",
            super::super::trust::ENV_TRUSTED_UID,
        )
    });
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         {pin_comment}\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
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
         \t<key>EnvironmentVariables</key>\n\
         \t<dict>\n\
         \t\t<key>IROH_TUNNEL_STATE_DIR</key>\n\
         \t\t<string>{state}</string>\n\
         {trust_env}\
         \t</dict>\n\
         \t<key>RunAtLoad</key>\n\
         \t<true/>\n\
         {keep_alive}\
         </dict>\n\
         </plist>\n"
    )
}

fn run_launchctl(native: &mut Native, args: &[&str]) -> Result<()> {
    let output = native
        .run(std::process::Command::new("launchctl").args(args))
        .with_context(|| format!("failed to run launchctl {:?}", args))?;
    if !output.success() {
        bail!("launchctl {:?} failed with {}", args, output.status_text());
    }
    Ok(())
}

/// Where the plist is (or would be).
pub(crate) fn registration_path(role: &str, scope: ServiceScope) -> Result<PathBuf> {
    plist_path(role, scope)
}

/// The installed plist's text, if any (to read its pin).
pub(crate) fn existing_registration(role: &str, scope: ServiceScope) -> Option<String> {
    super::super::registration::read_registration(&plist_path(role, scope).ok()?)
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
    let plist = format_plist_with(role, binary, config, state, &RestartPolicy::DEFAULT, pin);
    let path = plist_path(role, scope)?;
    write_registration(&path, &plist)?;
    native.applied(AppliedEffect::RegistrationWritten(path.clone()));
    load(native, notes, role, scope, &path)?;
    notes(&ServiceNote::Installed {
        name: label(role),
        at: InstalledAt::Path(path),
    });
    Ok(())
}

/// Replace any loaded job with the plist at `path`, hinting on failure.
///
/// The leading `enable` is unconditional and idempotent. Unlike the
/// best-effort `bootout`, its failure is reported and stops the load.
fn load(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
    path: &Path,
) -> Result<()> {
    let target = service_target(role, scope);
    let domain = domain_target(scope);
    let path_str = path.to_string_lossy().into_owned();

    // Clear any persistent per-domain disable flag for this label first;
    // otherwise `bootstrap` fails with `5: Input/output error`.
    run_launchctl(native, &["enable", &target])?;

    // If a job is already registered (re-install over an existing load),
    // bootout first so bootstrap doesn't fail with a confusing
    // "Input/output error" / "already loaded". This is best-effort: when
    // nothing is registered yet, launchctl prints "Boot-out failed: 3: No
    // such process" — expected, so swallow its stderr rather than alarm the
    // user.
    // Captured whatever the mode, so the expected failure text never reaches
    // the terminal.
    let _ = native.output(std::process::Command::new("launchctl").args(["bootout", &target]));

    // `bootstrap` reports concrete failures (bad plist, missing binary,
    // etc.) rather than the legacy `load`'s opaque `5: Input/output error`.
    if let Err(e) = run_launchctl(native, &["bootstrap", &domain, &path_str]) {
        notes(&ServiceNote::LaunchdBootstrapHint {
            domain,
            plist: path.to_owned(),
        });
        return Err(e).context("failed to load service into launchd");
    }
    // `bootstrap` loads the job and RunAtLoad starts it.
    native.applied(AppliedEffect::Enabled);
    native.applied(AppliedEffect::Started);
    Ok(())
}

/// Bounded poll for a job to disappear after `bootout`.
#[derive(Clone, Copy)]
struct UnloadPoll {
    /// Maximum number of `launchctl print` probes.
    checks: u32,
    /// Pause between probes.
    interval: Duration,
}

/// Production wait: 100 ms between probes, at most 5 s in total.
const UNLOAD_POLL: UnloadPoll = UnloadPoll {
    checks: 50,
    interval: Duration::from_millis(100),
};

/// Boot out `target` and wait for launchd to drop it, returning whether the
/// `bootout` itself succeeded. The wait runs only after a successful `bootout`
/// and is not an error: on timeout or an unexpected probe failure the caller
/// proceeds and the managed copy stays (fail-safe).
fn bootout_job(native: &mut Native, target: &str, poll: UnloadPoll) -> bool {
    if run_launchctl(native, &["bootout", target]).is_err() {
        return false;
    }
    wait_until_unloaded(native, target, poll);
    true
}

/// Probe `target` until launchd reports it gone, at most `poll.checks` times.
///
/// Returns true only on "Could not find service". Any other probe failure stops
/// the wait at once, so it never loops on an unknown state.
fn wait_until_unloaded(native: &mut Native, target: &str, poll: UnloadPoll) -> bool {
    for attempt in 1..=poll.checks {
        match print_definition(native, target) {
            EffectiveBinary::NotLoaded => return true,
            EffectiveBinary::Binary(_) if attempt < poll.checks => {
                std::thread::sleep(poll.interval);
            }
            EffectiveBinary::Binary(_) => {}
            EffectiveBinary::Unknown(_) => return false,
        }
    }
    false
}

pub(crate) fn uninstall(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
) -> Result<()> {
    let path = plist_path(role, scope)?;
    // Best-effort bootout: the job may not be loaded. Returns only after
    // launchd has dropped the job, so the release step sees the final state.
    if bootout_job(native, &service_target(role, scope), UNLOAD_POLL) {
        native.applied(AppliedEffect::Stopped);
        native.applied(AppliedEffect::Disabled);
    }
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
        native.applied(AppliedEffect::RegistrationRemoved(path));
    }
    notes(&ServiceNote::Uninstalled { name: label(role) });
    Ok(())
}

pub(crate) fn start(native: &mut Native, role: &str, scope: ServiceScope) -> Result<()> {
    // `kickstart` starts the job now if stopped, or restarts it if running.
    run_launchctl(native, &["kickstart", &service_target(role, scope)])
        .with_context(|| not_loaded_hint(role, scope))?;
    native.applied(AppliedEffect::Started);
    Ok(())
}

pub(crate) fn stop(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
) -> Result<()> {
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
    // Always captured, even when inheriting: the stderr text decides the
    // outcome below.
    let output = native
        .output(std::process::Command::new("launchctl").args(["kill", "SIGTERM", &target]))
        .with_context(|| format!("failed to run launchctl kill SIGTERM {target}"))?;
    if output.success() {
        native.applied(AppliedEffect::Stopped);
        return Ok(());
    }
    // exit code 3 = "No process to signal" → already stopped, not an error.
    // macOS encodes launchd errors as 128 + subsystem-code; for `kill` on a
    // loaded-but-stopped job this surfaces as plain 3.
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("No process to signal") || output.status == Some(3) {
        notes(&ServiceNote::AlreadyStopped { name: label(role) });
        return Ok(());
    }
    // Genuine failure (e.g. service not loaded at all): distinguish via a
    // probe so the hint is accurate.
    if !is_loaded(native, &target)? {
        bail!("{}", not_loaded_hint(role, scope));
    }
    bail!(
        "launchctl kill SIGTERM {target} failed with {status}: {stderr}",
        status = output.status_text()
    )
}

/// What launchd will run for the role's system job, from the loaded job.
pub(crate) fn effective_binary(native: &mut Native, role: &str) -> EffectiveBinary {
    print_definition(native, &service_target(role, ServiceScope::System))
}

/// Classify `launchctl print <target>` into the definition launchd holds.
fn print_definition(native: &mut Native, target: &str) -> EffectiveBinary {
    match native.output(std::process::Command::new("launchctl").args(["print", target])) {
        Ok(out) => parse_launchctl_print(
            out.success(),
            &String::from_utf8_lossy(&out.stdout),
            &String::from_utf8_lossy(&out.stderr),
        ),
        Err(e) => EffectiveBinary::Unknown(format!("failed to run launchctl print {target}: {e}")),
    }
}

/// Whether a job is currently registered in launchd for `target` (regardless
/// of whether it has a running process). Used to distinguish "already
/// stopped" from "never installed" when interpreting a `kill` failure.
fn is_loaded(native: &mut Native, target: &str) -> Result<bool> {
    // Captured whatever the mode: the probe must stay silent.
    let output = native
        .output(std::process::Command::new("launchctl").args(["print", target]))
        .with_context(|| format!("failed to run launchctl print {target}"))?;
    Ok(output.success())
}

pub(crate) fn restart(
    native: &mut Native,
    _notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
) -> Result<()> {
    // `kickstart -k` kills any running instance before starting a fresh
    // one. Without `-k`, kickstart is a no-op when the service is already
    // running (it only satisfies the "run now" condition), so a plain
    // `kickstart` would silently fail to restart a live service.
    run_launchctl(native, &["kickstart", "-k", &service_target(role, scope)])
        .with_context(|| not_loaded_hint(role, scope))?;
    native.applied(AppliedEffect::Started);
    Ok(())
}

/// Hint shown when `kickstart`/`kill` fail: either the service was never
/// installed in this scope, or it was installed in a *different* scope.
fn not_loaded_hint(role: &str, scope: ServiceScope) -> String {
    let (this_scope, other_flag) = match scope {
        // Default is User, so the more common mismatch is "installed as
        // system, tried without --system".
        ServiceScope::User => ("user (LaunchAgent)", "--system"),
        ServiceScope::System => ("system (LaunchDaemon)", "--user"),
    };
    format!(
        "no service '{0}' is loaded in the {this_scope} domain. \
         Install it first with `iroh-tunnel {role} service install`, \
         or it may be installed in the other scope (try {other_flag}).",
        label(role)
    )
}

pub(crate) fn status(
    native: &mut Native,
    _notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
) -> Result<NativeOutput> {
    // `launchctl print <target>` shows pid/state/last exit code. Read-only,
    // so no root requirement is enforced (system-domain prints may still be
    // partial without root, but that's informational, not fatal).
    // Non-zero typically means the job isn't loaded; informative, not fatal,
    // so the status is not checked.
    native
        .run(std::process::Command::new("launchctl").args(["print", &service_target(role, scope)]))
        .context("failed to run launchctl print")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::native::fake::{call, note_sink, output, Event, Events, FakeRunner};
    use crate::service::native::{Native, OutputMode};
    use crate::service::ServiceNote;

    fn loaded_print() -> NativeOutput {
        output(
            0,
            "system/dev.iroh-tunnel.serve = {\n\tstate = SIGTERMed\n\tprogram = /Library/Application Support/iroh-tunnel/bin/serve/iroh-tunnel\n}\n",
            "",
        )
    }

    fn not_found_print() -> NativeOutput {
        output(
            113,
            "",
            "Could not find service \"dev.iroh-tunnel.serve\" in domain for system\n",
        )
    }

    fn fast_poll(checks: u32) -> UnloadPoll {
        UnloadPoll {
            checks,
            interval: Duration::ZERO,
        }
    }

    fn print_call(target: &str) -> Event {
        call("launchctl", &["print", target], OutputMode::Capture)
    }

    #[test]
    fn bootout_waits_until_launchd_reports_the_job_gone() {
        let events = Events::default();
        let mut fake = FakeRunner::new(
            &events,
            vec![output(0, "", ""), loaded_print(), not_found_print()],
        );
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        let target = service_target("serve", ServiceScope::System);
        assert!(bootout_job(&mut native, &target, fast_poll(5)));
        assert_eq!(
            *events.borrow(),
            vec![
                call(
                    "launchctl",
                    &["bootout", "system/dev.iroh-tunnel.serve"],
                    OutputMode::Inherit
                ),
                print_call("system/dev.iroh-tunnel.serve"),
                print_call("system/dev.iroh-tunnel.serve"),
            ],
            "polling stops at the first not-found print"
        );
    }

    #[test]
    fn failed_bootout_skips_the_wait() {
        let events = Events::default();
        let mut fake = FakeRunner::new(
            &events,
            vec![output(3, "", "Boot-out failed: 3: No such process")],
        );
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        let target = service_target("serve", ServiceScope::System);
        assert!(!bootout_job(&mut native, &target, fast_poll(5)));
        assert_eq!(events.borrow().len(), 1, "no print after a failed bootout");
    }

    #[test]
    fn wait_stops_at_the_check_limit_and_reports_still_loaded() {
        let events = Events::default();
        let mut fake = FakeRunner::new(
            &events,
            vec![
                loaded_print(),
                loaded_print(),
                loaded_print(),
                loaded_print(),
            ],
        );
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        let target = service_target("serve", ServiceScope::System);
        assert!(!wait_until_unloaded(&mut native, &target, fast_poll(3)));
        assert_eq!(events.borrow().len(), 3, "exactly the check limit");
    }

    #[test]
    fn wait_stops_at_an_unexpected_print_failure() {
        let events = Events::default();
        let mut fake = FakeRunner::new(
            &events,
            vec![output(1, "", "Operation not permitted\n"), loaded_print()],
        );
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        let target = service_target("serve", ServiceScope::System);
        assert!(!wait_until_unloaded(&mut native, &target, fast_poll(5)));
        assert_eq!(
            events.borrow().len(),
            1,
            "no retry on an unexpected failure"
        );
    }

    #[test]
    fn effective_binary_prints_the_system_target() {
        let events = Events::default();
        let printed = "system/x = {\n\tprogram = /opt/x\n";
        let mut fake = FakeRunner::new(&events, vec![output(0, printed, "")]);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        assert_eq!(
            effective_binary(&mut native, "serve"),
            EffectiveBinary::Binary(PathBuf::from("/opt/x"))
        );
        assert_eq!(
            *events.borrow(),
            vec![call(
                "launchctl",
                &["print", "system/dev.iroh-tunnel.serve"],
                OutputMode::Capture
            )]
        );
    }

    #[test]
    fn plist_declares_apples_property_list_doctype() {
        let body = format_plist("serve", Path::new("/b"), Path::new("/c"), Path::new("/s"));
        assert!(body.contains("PropertyList-1.0.dtd"), "{body}");
        assert!(!body.contains("PropertyManifest"), "{body}");
    }

    #[test]
    fn plist_carries_no_exit_status_exemption() {
        // launchd's KeepAlive cannot exempt an exit status the way systemd's
        // RestartPreventExitStatus does; see design.md. This pins the gap so a
        // future edit cannot claim containment the platform does not provide.
        let body = format_plist("serve", Path::new("/b"), Path::new("/c"), Path::new("/s"));
        assert!(
            body.contains("<key>SuccessfulExit</key>\n\t\t<false/>"),
            "{body}"
        );
        assert!(
            !body.contains("ExitStatus") && !body.contains("OtherJobEnabled"),
            "{body}"
        );
    }

    #[test]
    fn capture_mode_returns_native_output() {
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, vec![output(0, "state = running\n", "")]);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Capture, &mut runner);
        let out = status(&mut native, &mut |_| {}, "serve", ServiceScope::System).unwrap();
        assert_eq!(out.stdout, b"state = running\n");
        assert_eq!(
            *events.borrow(),
            vec![call(
                "launchctl",
                &["print", "system/dev.iroh-tunnel.serve"],
                OutputMode::Capture
            )],
            "no note and no other command"
        );
    }

    #[test]
    fn load_enables_the_label_before_bootout_and_bootstrap() {
        let events = Events::default();
        let mut fake = FakeRunner::new(
            &events,
            vec![output(0, "", ""), output(0, "", ""), output(0, "", "")],
        );
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        load(
            &mut native,
            &mut |_| {},
            "serve",
            ServiceScope::System,
            Path::new("/tmp/x.plist"),
        )
        .unwrap();
        assert_eq!(
            *events.borrow(),
            vec![
                call(
                    "launchctl",
                    &["enable", "system/dev.iroh-tunnel.serve"],
                    OutputMode::Inherit
                ),
                call(
                    "launchctl",
                    &["bootout", "system/dev.iroh-tunnel.serve"],
                    OutputMode::Capture
                ),
                call(
                    "launchctl",
                    &["bootstrap", "system", "/tmp/x.plist"],
                    OutputMode::Inherit
                ),
            ]
        );
    }

    #[test]
    fn load_fails_without_bootstrapping_when_enable_fails() {
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, vec![output(1, "", "Operation not permitted")]);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        let err = load(
            &mut native,
            &mut |_| {},
            "serve",
            ServiceScope::User,
            Path::new("/tmp/x.plist"),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("enable"), "{err:#}");
        assert_eq!(events.borrow().len(), 1, "no command after a failed enable");
    }

    #[test]
    fn load_ignores_a_bootout_failure_and_still_bootstraps() {
        let events = Events::default();
        let mut fake = FakeRunner::new(
            &events,
            vec![
                output(0, "", ""),
                output(3, "", "Boot-out failed: 3: No such process"),
                output(0, "", ""),
            ],
        );
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        load(
            &mut native,
            &mut |_| {},
            "serve",
            ServiceScope::System,
            Path::new("/tmp/x.plist"),
        )
        .unwrap();
        assert_eq!(events.borrow().len(), 3, "bootstrap still issued");
    }

    #[test]
    fn notes_reach_observer_before_native_command() {
        let events = Events::default();
        let mut fake = FakeRunner::new(
            &events,
            vec![output(0, "", ""), output(0, "", ""), output(5, "", "")],
        );
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        let mut notes = note_sink(&events);
        let plist = PathBuf::from("/tmp/x.plist");
        let err = load(
            &mut native,
            &mut notes,
            "serve",
            ServiceScope::System,
            &plist,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("failed to load service into launchd"));
        assert_eq!(
            *events.borrow(),
            vec![
                call(
                    "launchctl",
                    &["enable", "system/dev.iroh-tunnel.serve"],
                    OutputMode::Inherit
                ),
                call(
                    "launchctl",
                    &["bootout", "system/dev.iroh-tunnel.serve"],
                    OutputMode::Capture
                ),
                call(
                    "launchctl",
                    &["bootstrap", "system", "/tmp/x.plist"],
                    OutputMode::Inherit
                ),
                Event::Note(ServiceNote::LaunchdBootstrapHint {
                    domain: "system".into(),
                    plist,
                }),
            ]
        );
    }

    #[test]
    fn stop_captures_even_when_inheriting_and_reports_already_stopped() {
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, vec![output(3, "", "No process to signal")]);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        let mut notes = note_sink(&events);
        stop(&mut native, &mut notes, "serve", ServiceScope::System).unwrap();
        assert_eq!(
            *events.borrow(),
            vec![
                call(
                    "launchctl",
                    &["kill", "SIGTERM", "system/dev.iroh-tunnel.serve"],
                    OutputMode::Capture
                ),
                Event::Note(ServiceNote::AlreadyStopped {
                    name: "dev.iroh-tunnel.serve".into()
                }),
            ]
        );
    }

    #[test]
    fn plist_is_valid_xml_and_has_label() {
        let body = format_plist(
            "serve",
            Path::new("/usr/local/bin/iroh-tunnel"),
            Path::new("/etc/iroh-tunnel/serve.toml"),
            Path::new("/var/state"),
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
    fn plist_pins_config_argument_and_state_dir_environment() {
        let body = format_plist(
            "serve",
            Path::new("/bin/it"),
            Path::new("/cfg/serve.toml"),
            Path::new("/st&ate"),
        );
        assert!(body.contains("<string>/cfg/serve.toml</string>"), "{body}");
        assert!(
            body.contains("<key>IROH_TUNNEL_STATE_DIR</key>\n\t\t<string>/st&amp;ate</string>"),
            "{body}"
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
            Path::new("/var/state"),
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

    #[test]
    fn plist_default_policy_emits_keepalive_successful_exit_false() {
        // Regression: the shared RestartPolicy::DEFAULT must keep the previous
        // "respawn unless clean exit" behavior the plist used to hard-code.
        let body = format_plist(
            "serve",
            Path::new("/usr/local/bin/iroh-tunnel"),
            Path::new("/etc/cfg"),
            Path::new("/var/state"),
        );
        assert!(
            body.contains("<key>KeepAlive</key>"),
            "KeepAlive missing with default policy: {body}"
        );
        assert!(
            body.contains("<key>SuccessfulExit</key>\n\t\t<false/>"),
            "KeepAlive SuccessfulExit=false missing: {body}"
        );
    }

    #[test]
    fn plist_disabled_policy_omits_keepalive_block() {
        let policy = RestartPolicy {
            on_failure: false,
            delay_secs: 0,
        };
        let body = format_plist_with(
            "serve",
            Path::new("/x"),
            Path::new("/y"),
            Path::new("/z"),
            &policy,
            None,
        );
        assert!(
            !body.contains("<key>KeepAlive</key>"),
            "KeepAlive should be absent when policy disables restart: {body}"
        );
        // RunAtLoad is orthogonal and should still be present.
        assert!(body.contains("<key>RunAtLoad</key>"));
    }

    #[test]
    fn managed_path_renders_as_one_argument_in_each_backend() {
        let copy = "/Library/Application Support/iroh-tunnel/bin/serve/iroh-tunnel";
        let body = format_plist("serve", Path::new(copy), Path::new("/c"), Path::new("/s"));
        // One `<string>` element is one argument, spaces included.
        assert!(
            body.contains(&format!(
                "<array>\n\t\t<string>{copy}</string>\n\t\t<string>serve</string>"
            )),
            "{body}"
        );
        // How launchd prints that job once loaded.
        let printed = format!(
            "system/dev.iroh-tunnel.serve = {{\n\tprogram = {copy}\n\targuments = {{\n\t\t{copy}\n\t\tserve\n\t}}\n}}\n"
        );
        assert_eq!(
            parse_launchctl_print(true, &printed, ""),
            EffectiveBinary::Binary(PathBuf::from(copy))
        );
    }
}
