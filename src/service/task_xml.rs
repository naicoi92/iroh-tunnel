//! Task Scheduler registration for the Windows user-background mechanism
//! (SM-04, issue #92): a per-user logon task with an interactive token and
//! the shared [`RestartPolicy`].
//!
//! This module only renders the registration. It is compiled on every target
//! so the rendering is unit-tested everywhere; running `schtasks` lives in
//! `windows.rs`.

// Only the Windows backend and the tests call into this module.
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::Path;

use anyhow::Result;

use super::RestartPolicy;
use crate::error::CliError;

/// Task Scheduler refuses a restart interval below one minute.
const MIN_RESTART_MINUTES: u64 = 1;

/// Environment variable that pins the state directory (see `paths.rs`).
const STATE_DIR_ENV: &str = "IROH_TUNNEL_STATE_DIR";

/// The command a task runs.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct TaskAction {
    pub command: String,
    pub arguments: String,
}

/// Escape text for an XML element body.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Refuse characters that cannot be carried safely into a Windows command
/// line. `"` cannot occur in a Windows path; `%` is expanded by `cmd.exe`
/// even inside quotes.
fn ensure_plain(what: &str, path: &Path) -> Result<()> {
    let text = path.to_string_lossy();
    if text.contains('"') || text.contains('%') {
        return Err(CliError::Config(format!(
            "the {what} path {text:?} contains `\"` or `%`, which cannot be registered \
             safely in a scheduled task; choose a path without them"
        ))
        .into());
    }
    Ok(())
}

/// Build the action for `iroh-tunnel {role} run --config {config}`.
///
/// A task cannot carry environment variables, so a state directory that
/// differs from the user's default is pinned through a `cmd.exe` wrapper that
/// sets `IROH_TUNNEL_STATE_DIR` first. When `state_override` is `None` the
/// binary is started directly, so `schtasks /End` acts on the tunnel process
/// itself.
///
/// # Errors
///
/// [`CliError::Config`] when a path contains `"` or `%`.
pub(super) fn task_action(
    role: &str,
    binary: &Path,
    config: &Path,
    state_override: Option<&Path>,
) -> Result<TaskAction> {
    ensure_plain("binary", binary)?;
    ensure_plain("config", config)?;
    let run = format!(
        "\"{}\" {role} run --config \"{}\"",
        binary.display(),
        config.display()
    );
    match state_override {
        None => Ok(TaskAction {
            command: binary.to_string_lossy().into_owned(),
            arguments: format!("{role} run --config \"{}\"", config.display()),
        }),
        Some(state) => {
            ensure_plain("state", state)?;
            Ok(TaskAction {
                command: "cmd.exe".to_owned(),
                // `/s` strips exactly the outer pair of quotes.
                arguments: format!(
                    "/d /s /c \"set \"{STATE_DIR_ENV}={}\" && {run}\"",
                    state.display()
                ),
            })
        }
    }
}

/// Render the Task Scheduler XML for the current user's background task.
///
/// `user` is the `DOMAIN\name` the logon trigger and the principal are bound
/// to: a trigger for "any user" needs administrator rights, one bound to the
/// current user does not.
pub(super) fn format_task_xml(
    role: &str,
    user: &str,
    action: &TaskAction,
    policy: &RestartPolicy,
) -> String {
    let user = xml_escape(user);
    let role = xml_escape(role);
    let command = xml_escape(&action.command);
    let arguments = xml_escape(&action.arguments);
    let restart = if policy.on_failure {
        let minutes = policy.delay_secs.div_ceil(60).max(MIN_RESTART_MINUTES);
        format!(
            "    <RestartOnFailure>\n      <Interval>PT{minutes}M</Interval>\n      \
             <Count>999</Count>\n    </RestartOnFailure>\n"
        )
    } else {
        String::new()
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n\
         <Task version=\"1.4\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n\
         \x20 <RegistrationInfo>\n\
         \x20   <Description>Iroh Tunnel ({role}) in the logged-in user session</Description>\n\
         \x20 </RegistrationInfo>\n\
         \x20 <Triggers>\n\
         \x20   <LogonTrigger>\n\
         \x20     <Enabled>true</Enabled>\n\
         \x20     <UserId>{user}</UserId>\n\
         \x20   </LogonTrigger>\n\
         \x20 </Triggers>\n\
         \x20 <Principals>\n\
         \x20   <Principal id=\"Author\">\n\
         \x20     <UserId>{user}</UserId>\n\
         \x20     <LogonType>InteractiveToken</LogonType>\n\
         \x20     <RunLevel>LeastPrivilege</RunLevel>\n\
         \x20   </Principal>\n\
         \x20 </Principals>\n\
         \x20 <Settings>\n\
         \x20   <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\n\
         \x20   <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\n\
         \x20   <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\n\
         \x20   <AllowHardTerminate>true</AllowHardTerminate>\n\
         \x20   <StartWhenAvailable>false</StartWhenAvailable>\n\
         \x20   <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>\n\
         \x20   <AllowStartOnDemand>true</AllowStartOnDemand>\n\
         \x20   <Enabled>true</Enabled>\n\
         \x20   <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\n\
         {restart}\
         \x20 </Settings>\n\
         \x20 <Actions Context=\"Author\">\n\
         \x20   <Exec>\n\
         \x20     <Command>{command}</Command>\n\
         \x20     <Arguments>{arguments}</Arguments>\n\
         \x20   </Exec>\n\
         \x20 </Actions>\n\
         </Task>\n"
    )
}

/// Encode the XML the way `schtasks /XML` expects when the declaration says
/// UTF-16: little-endian with a byte-order mark.
pub(super) fn encode_utf16_le(xml: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in xml.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY: RestartPolicy = RestartPolicy::DEFAULT;

    fn direct() -> TaskAction {
        task_action(
            "serve",
            Path::new(r"C:\Program Files\it\iroh-tunnel.exe"),
            Path::new(r"C:\Users\me\AppData\Local\iroh-tunnel\config\serve.toml"),
            None,
        )
        .unwrap()
    }

    #[test]
    fn default_state_runs_the_binary_directly_with_the_pinned_config() {
        let action = direct();
        assert_eq!(action.command, r"C:\Program Files\it\iroh-tunnel.exe");
        assert_eq!(
            action.arguments,
            r#"serve run --config "C:\Users\me\AppData\Local\iroh-tunnel\config\serve.toml""#
        );
    }

    #[test]
    fn overridden_state_dir_is_pinned_through_a_cmd_wrapper() {
        let action = task_action(
            "access",
            Path::new(r"C:\bin\iroh-tunnel.exe"),
            Path::new(r"C:\cfg\access.toml"),
            Some(Path::new(r"D:\st ate")),
        )
        .unwrap();
        assert_eq!(action.command, "cmd.exe");
        assert_eq!(
            action.arguments,
            r#"/d /s /c "set "IROH_TUNNEL_STATE_DIR=D:\st ate" && "C:\bin\iroh-tunnel.exe" access run --config "C:\cfg\access.toml"""#
        );
    }

    #[test]
    fn paths_that_cmd_would_rewrite_are_refused_as_config_errors() {
        for bad in [r"C:\100%\a.toml", "C:\\a\"b.toml"] {
            let err =
                task_action("serve", Path::new(r"C:\b.exe"), Path::new(bad), None).unwrap_err();
            assert!(
                matches!(err.downcast_ref::<CliError>(), Some(CliError::Config(_))),
                "{bad}: {err:?}"
            );
        }
        let err = task_action(
            "serve",
            Path::new(r"C:\b.exe"),
            Path::new(r"C:\c.toml"),
            Some(Path::new(r"C:\%TEMP%")),
        )
        .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<CliError>(),
            Some(CliError::Config(_))
        ));
    }

    #[test]
    fn task_runs_at_logon_of_the_current_user_with_a_limited_interactive_token() {
        let xml = format_task_xml("serve", r"HOST\me", &direct(), &POLICY);
        assert!(xml.contains("<LogonTrigger>"), "{xml}");
        assert_eq!(xml.matches(r"<UserId>HOST\me</UserId>").count(), 2, "{xml}");
        assert!(
            xml.contains("<LogonType>InteractiveToken</LogonType>"),
            "{xml}"
        );
        assert!(xml.contains("<RunLevel>LeastPrivilege</RunLevel>"), "{xml}");
        assert!(!xml.contains("HighestAvailable"), "{xml}");
    }

    #[test]
    fn task_is_never_time_limited_nor_stopped_on_battery() {
        let xml = format_task_xml("serve", r"HOST\me", &direct(), &POLICY);
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false<"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false<"));
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew<"));
    }

    #[test]
    fn default_policy_restarts_on_failure_at_the_one_minute_scheduler_minimum() {
        let xml = format_task_xml("serve", r"HOST\me", &direct(), &POLICY);
        assert!(xml.contains("<Interval>PT1M</Interval>"), "{xml}");
        assert!(xml.contains("<Count>999</Count>"), "{xml}");
    }

    #[test]
    fn restart_delay_rounds_up_to_whole_minutes() {
        let policy = RestartPolicy {
            on_failure: true,
            delay_secs: 61,
        };
        let xml = format_task_xml("serve", r"HOST\me", &direct(), &policy);
        assert!(xml.contains("<Interval>PT2M</Interval>"), "{xml}");
    }

    #[test]
    fn disabled_policy_omits_restart_on_failure() {
        let policy = RestartPolicy {
            on_failure: false,
            delay_secs: 0,
        };
        let xml = format_task_xml("serve", r"HOST\me", &direct(), &policy);
        assert!(!xml.contains("RestartOnFailure"), "{xml}");
    }

    #[test]
    fn xml_metacharacters_in_user_and_paths_are_escaped() {
        let action = task_action(
            "serve",
            Path::new(r"C:\a&b\it.exe"),
            Path::new(r"C:\<x>\s.toml"),
            None,
        )
        .unwrap();
        let xml = format_task_xml("serve", r"HOST\o&m", &action, &POLICY);
        assert!(xml.contains(r"HOST\o&amp;m"), "{xml}");
        assert!(xml.contains(r"C:\a&amp;b\it.exe"), "{xml}");
        assert!(xml.contains("&lt;x&gt;"), "{xml}");
        assert!(!xml.contains("<x>"), "{xml}");
    }

    #[test]
    fn utf16_encoding_has_a_little_endian_bom_and_round_trips() {
        let bytes = encode_utf16_le("<a>é</a>");
        assert_eq!(&bytes[..2], &[0xFF, 0xFE]);
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        assert_eq!(String::from_utf16(&units).unwrap(), "<a>é</a>");
    }
}
