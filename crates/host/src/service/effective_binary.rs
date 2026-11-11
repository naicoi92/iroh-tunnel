//! What a service manager will actually run for a registered service
//! (portable parsers; the backends supply the text).
//!
//! The answer comes from the loaded definition, never from the registration
//! file on disk: a definition loaded earlier may name a different binary.

use std::path::PathBuf;

/// The binary a service manager would execute for a role's service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EffectiveBinary {
    /// The manager has no definition for the service.
    NotLoaded,
    /// The single binary the manager runs.
    Binary(PathBuf),
    /// The definition could not be read or is ambiguous; the text says why.
    Unknown(String),
}

fn unknown(why: &str) -> EffectiveBinary {
    EffectiveBinary::Unknown(why.to_owned())
}

/// Parse `systemctl show -p LoadState -p ExecStart <unit>` output.
#[cfg_attr(
    not(target_os = "linux"),
    allow(dead_code, reason = "only the Linux backends parse this")
)]
pub(crate) fn parse_systemd_show(stdout: &str) -> EffectiveBinary {
    let value = |key: &str| {
        stdout
            .lines()
            .find_map(|line| line.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
    };
    match value("LoadState") {
        Some("not-found") => return EffectiveBinary::NotLoaded,
        Some("loaded") => {}
        Some(other) => return EffectiveBinary::Unknown(format!("unit load state is {other}")),
        None => return unknown("systemctl reported no load state"),
    }
    let commands: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("ExecStart="))
        .collect();
    let [exec] = commands.as_slice() else {
        return unknown("the unit does not have exactly one ExecStart command");
    };
    if exec.matches("{ path=").count() != 1 || !exec.trim_end().ends_with('}') {
        return unknown("the unit does not have exactly one well-formed ExecStart command");
    }
    let Some((_, rest)) = exec.split_once("{ path=") else {
        return unknown("unrecognized ExecStart format");
    };
    match rest.split_once(" ;") {
        Some((path, _)) if !path.is_empty() => EffectiveBinary::Binary(PathBuf::from(path)),
        _ => unknown("unrecognized ExecStart format"),
    }
}

/// Parse `launchctl print <target>` output.
#[cfg_attr(
    not(target_os = "macos"),
    allow(dead_code, reason = "only the macOS backend parses this")
)]
pub(crate) fn parse_launchctl_print(success: bool, stdout: &str, stderr: &str) -> EffectiveBinary {
    if !success {
        return if stderr.contains("Could not find service") {
            EffectiveBinary::NotLoaded
        } else {
            unknown("launchctl print failed")
        };
    }
    let program = stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("program = "));
    if let Some(path) = program.filter(|p| !p.is_empty()) {
        return EffectiveBinary::Binary(PathBuf::from(path));
    }
    let mut lines = stdout.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "arguments = {" {
            return match lines.next().map(str::trim) {
                Some(first) if !first.is_empty() && first != "}" => {
                    EffectiveBinary::Binary(PathBuf::from(first))
                }
                _ => unknown("launchd definition has empty arguments"),
            };
        }
    }
    unknown("launchd definition names no program")
}

/// Parse the installed BusyBox init script (`None` when it does not exist).
#[cfg_attr(
    not(target_os = "linux"),
    allow(dead_code, reason = "only the Linux backends parse this")
)]
pub(crate) fn parse_busybox_script(script: Option<&str>) -> EffectiveBinary {
    let Some(script) = script else {
        return EffectiveBinary::NotLoaded;
    };
    let daemon = script
        .lines()
        .find_map(|line| line.strip_prefix("DAEMON=\""))
        .and_then(|rest| rest.strip_suffix('"'));
    match daemon {
        Some(path) if !path.is_empty() => EffectiveBinary::Binary(PathBuf::from(path)),
        _ => unknown("init script has no DAEMON line"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Source: systemd 255 `systemctl show -p LoadState -p ExecStart`.
    const LOADED: &str = "LoadState=loaded\nExecStart={ path=/usr/local/bin/iroh-tunnel ; argv[]=/usr/local/bin/iroh-tunnel serve run --config /c ; ignore_errors=no ; start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; code=(null) ; status=0/0 }\n";
    // Source: same command, a path with a space.
    const SPACED: &str = "LoadState=loaded\nExecStart={ path=/Library/Application Support/iroh-tunnel/bin/serve/iroh-tunnel ; argv[]=/x serve run ; ignore_errors=no ; status=0/0 }\n";
    // Source: systemd 255 for a unit that does not exist.
    const NOT_FOUND: &str = "LoadState=not-found\nExecStart=\n";
    const NO_EXEC: &str = "LoadState=loaded\nExecStart=\n";
    // Source: systemd prints several commands space-separated on one line.
    const TWO_EXEC: &str = "LoadState=loaded\nExecStart={ path=/a ; argv[]=/a ; status=0/0 } { path=/b ; argv[]=/b ; status=0/0 }\n";

    // Source: macOS 15 `launchctl print system/dev.orbstack.OrbStack.privhelper`.
    const LAUNCHD_PROGRAM: &str = "system/dev.orbstack.OrbStack.privhelper = {\n\tactive count = 0\n\tpath = /Library/LaunchDaemons/dev.orbstack.OrbStack.privhelper.plist\n\ttype = LaunchDaemon\n\tstate = not running\n\n\tprogram = /Library/PrivilegedHelperTools/dev.orbstack.OrbStack.privhelper\n\targuments = {\n\t\t/Library/PrivilegedHelperTools/dev.orbstack.OrbStack.privhelper\n\t}\n";
    // Source: same layout without a `program` line (macOS 15).
    const LAUNCHD_ARGS: &str = "system/dev.iroh-tunnel.serve = {\n\tstate = running\n\n\targuments = {\n\t\t/opt/x\n\t\tserve\n\t}\n";
    // Source: macOS 15 stderr of `launchctl print` for an unknown label, exit 113.
    const LAUNCHD_MISSING: &str =
        "Could not find service \"dev.iroh-tunnel.serve\" in domain for system\n";

    fn bin(p: &str) -> EffectiveBinary {
        EffectiveBinary::Binary(PathBuf::from(p))
    }

    #[test]
    fn systemd_loaded_unit_yields_its_binary() {
        assert_eq!(
            parse_systemd_show(LOADED),
            bin("/usr/local/bin/iroh-tunnel")
        );
    }

    #[test]
    fn systemd_path_with_space_is_kept_whole() {
        assert_eq!(
            parse_systemd_show(SPACED),
            bin("/Library/Application Support/iroh-tunnel/bin/serve/iroh-tunnel")
        );
    }

    #[test]
    fn systemd_missing_unit_is_not_loaded() {
        assert_eq!(parse_systemd_show(NOT_FOUND), EffectiveBinary::NotLoaded);
    }

    #[test]
    fn systemd_loaded_without_exec_start_is_unknown() {
        assert!(matches!(
            parse_systemd_show(NO_EXEC),
            EffectiveBinary::Unknown(_)
        ));
    }

    #[test]
    fn systemd_two_commands_are_unknown() {
        assert!(matches!(
            parse_systemd_show(TWO_EXEC),
            EffectiveBinary::Unknown(_)
        ));
    }

    // Source: systemd v255 systemctl-show.c prints each command on its own line.
    const TWO_EXEC_LINES: &str = "LoadState=loaded\nExecStart={ path=/a ; argv[]=/a ; status=0/0 }\nExecStart={ path=/b ; argv[]=/b ; status=0/0 }\n";

    #[test]
    fn systemd_commands_on_separate_lines_are_unknown() {
        assert!(matches!(
            parse_systemd_show(TWO_EXEC_LINES),
            EffectiveBinary::Unknown(_)
        ));
    }

    #[test]
    fn systemd_truncated_exec_start_is_unknown() {
        assert!(matches!(
            parse_systemd_show("LoadState=loaded\nExecStart={ path=/a\n"),
            EffectiveBinary::Unknown(_)
        ));
    }

    #[test]
    fn systemd_garbage_is_unknown() {
        assert!(matches!(
            parse_systemd_show(""),
            EffectiveBinary::Unknown(_)
        ));
        assert!(matches!(
            parse_systemd_show("LoadState=masked\nExecStart=\n"),
            EffectiveBinary::Unknown(_)
        ));
    }

    #[test]
    fn launchd_program_line_wins() {
        assert_eq!(
            parse_launchctl_print(true, LAUNCHD_PROGRAM, ""),
            bin("/Library/PrivilegedHelperTools/dev.orbstack.OrbStack.privhelper")
        );
    }

    #[test]
    fn launchd_falls_back_to_first_argument() {
        assert_eq!(parse_launchctl_print(true, LAUNCHD_ARGS, ""), bin("/opt/x"));
    }

    #[test]
    fn launchd_missing_service_is_not_loaded() {
        assert_eq!(
            parse_launchctl_print(false, "", LAUNCHD_MISSING),
            EffectiveBinary::NotLoaded
        );
    }

    #[test]
    fn launchd_other_failure_is_unknown() {
        assert!(matches!(
            parse_launchctl_print(false, "", "Operation not permitted\n"),
            EffectiveBinary::Unknown(_)
        ));
    }

    #[test]
    fn launchd_success_without_program_is_unknown() {
        assert!(matches!(
            parse_launchctl_print(true, "system/x = {\n\tstate = running\n", ""),
            EffectiveBinary::Unknown(_)
        ));
    }

    #[test]
    fn busybox_absent_script_is_not_loaded() {
        assert_eq!(parse_busybox_script(None), EffectiveBinary::NotLoaded);
    }

    #[test]
    fn busybox_script_without_daemon_is_unknown() {
        assert!(matches!(
            parse_busybox_script(Some("#!/bin/sh\nexit 0\n")),
            EffectiveBinary::Unknown(_)
        ));
    }

    #[test]
    fn busybox_daemon_line_yields_binary() {
        assert_eq!(
            parse_busybox_script(Some(
                "#!/bin/sh\nDAEMON=\"/a b/iroh-tunnel\"\nROLE=\"serve\"\n"
            )),
            bin("/a b/iroh-tunnel")
        );
    }
}
