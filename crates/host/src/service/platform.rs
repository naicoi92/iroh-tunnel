//! Platform selection helpers of the service backends (private).

use std::path::Path;

use anyhow::Result;

#[cfg(unix)]
use super::types::ServiceScope;

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
        super::trust::load_config(path)
    }
    #[cfg(windows)]
    {
        windows_load_config(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        C::load(path)
    }
}

/// Windows twin of `trust::load_config`: under a system service the state
/// directory and the config are verified and the config is parsed only from
/// the verified handle. The service never writes into the user's file, so a
/// missing key is an error; with a non-empty key `resolve_and_save_key` has
/// nothing to persist.
#[cfg(windows)]
fn windows_load_config<C: crate::config::RoleDoc>(path: &Path) -> Result<C> {
    use super::trust_windows::{
        model::TrustedSet, read_verified, trusted_sid_from_env, verify_dir,
    };
    use crate::error::CliError;
    use crate::status::writer::ENV_STATE_DIR;

    let Some(sid) = trusted_sid_from_env()? else {
        if std::env::var_os(super::ENV_SCM).is_some_and(|v| v == "1") {
            return Err(CliError::Permission(
                "a Windows system service started without IROH_TUNNEL_TRUSTED_SID; the \
                 registration predates the trust policy. Re-install it with `iroh-tunnel <role> \
                 service install --system`"
                    .to_owned(),
            )
            .into());
        }
        return C::load(path);
    };
    let trusted = TrustedSet::for_user_files(&sid);
    if let Some(state) = std::env::var_os(ENV_STATE_DIR).filter(|d| !d.is_empty()) {
        verify_dir(Path::new(&state), &trusted)?;
    }
    let text = read_verified(path, &trusted)?;
    let mut cfg = C::from_toml(&text, path)?;
    if cfg.node_mut().secret_key.is_empty() {
        return Err(CliError::Config(empty_key_text(path)).into());
    }
    Ok(cfg)
}

/// The refusal for a keyless config under a Windows system service.
#[cfg_attr(not(windows), allow(dead_code))]
fn empty_key_text(path: &Path) -> String {
    format!(
        "{0} has no secret_key, and a system service does not write into a user's file. Run \
         `iroh-tunnel <role> config keygen --config {0}` as that user, then restart",
        path.display()
    )
}

/// The text of the currently installed system registration, if any: where a
/// previous install's pin is read from to detect a retarget (SM-02). Only
/// system installs read it, and those exist on Unix only.
#[cfg(unix)]
pub(crate) fn existing_registration(role: &str, scope: ServiceScope) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            super::backend::systemd::existing_registration(role, scope)
        } else {
            let _ = scope;
            super::backend::busybox::existing_registration(role)
        }
    }
    #[cfg(target_os = "macos")]
    {
        super::backend::launchd::existing_registration(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        None
    }
}

/// True when systemd is the running init (PID 1). Used to pick the systemd vs
/// BusyBox/SysV backend at runtime.
///
/// Primary signal: systemd creates `/run/systemd/system` when it manages the
/// boot. Fallback: read the target of `/proc/1/exe` (the kernel exposes PID 1's
/// executable there) and check it ends in `systemd`. Both are no-ops (return
/// false) on a non-systemd host and on non-Linux builds.
#[cfg(target_os = "linux")]
pub(crate) fn linux_is_systemd() -> bool {
    if std::path::Path::new("/run/systemd/system").exists() {
        return true;
    }
    std::fs::read_link("/proc/1/exe")
        .map(|p| p.ends_with("systemd"))
        .unwrap_or(false)
}

/// Whether every registration on this host is system-wide: a Linux host whose
/// init is not systemd (BusyBox), which has no per-user services.
///
/// `backend_install` picks its backend on the same condition.
#[cfg(target_os = "linux")]
pub(crate) fn system_wide_only() -> bool {
    !linux_is_systemd()
}

/// Whether every registration on this host is system-wide; only Linux without
/// systemd is, so never here.
#[cfg(not(target_os = "linux"))]
pub(crate) fn system_wide_only() -> bool {
    false
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn unsupported<T>() -> Result<T> {
    anyhow::bail!(
        "service management is only supported on Linux (systemd/BusyBox init), macOS (launchd) \
         and Windows (user background task)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_empty_key_text_names_keygen() {
        let text = empty_key_text(Path::new("C:/u/serve.toml"));
        assert!(
            text.contains("config keygen --config C:/u/serve.toml"),
            "{text}"
        );
        assert!(text.contains("does not write into a user's file"), "{text}");
    }

    // Compile-only guard: on Linux the runtime probe must exist and be bool.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_is_systemd_is_bool() {
        let _ = linux_is_systemd();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn system_wide_only_is_the_negation_of_systemd_on_linux() {
        assert_eq!(system_wide_only(), !linux_is_systemd());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn system_wide_only_is_false_off_linux() {
        assert!(!system_wide_only());
    }
}

#[allow(clippy::mixed_attributes_style)]
#[cfg(test)]
mod windows_tests {
    #![cfg(windows)]
    use super::*;
    use crate::config::ServeConfig;
    use crate::error::CliError;
    use crate::service::trust_windows::{test_env::EnvGuard, token_user};

    const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    fn write(dir: &Path, body: &str) -> std::path::PathBuf {
        let file = dir.join("serve.toml");
        std::fs::write(&file, body).unwrap();
        file
    }

    fn sid_text() -> String {
        token_user().unwrap().0
    }

    fn error_of(r: Result<ServeConfig>) -> CliError {
        match r.unwrap_err().downcast::<CliError>() {
            Ok(e) => e,
            Err(e) => panic!("expected CliError: {e:#}"),
        }
    }

    #[test]
    fn service_without_sid_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write(tmp.path(), "");
        let _env = EnvGuard::set(&[
            ("IROH_TUNNEL_TRUSTED_SID", None),
            ("IROH_TUNNEL_SCM", Some("1")),
        ]);
        match error_of(load_config::<ServeConfig>(&file)) {
            CliError::Permission(m) => assert!(m.contains("service install --system"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn empty_secret_key_under_service_is_config_error_and_file_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write(tmp.path(), "[node]\nsecret_key = \"\"\n");
        let before = std::fs::read(&file).unwrap();
        let sddl_before = std::process::Command::new("icacls")
            .arg(&file)
            .output()
            .unwrap()
            .stdout;
        let sid = sid_text();
        let _env = EnvGuard::set(&[
            ("IROH_TUNNEL_TRUSTED_SID", Some(&sid)),
            ("IROH_TUNNEL_STATE_DIR", None),
            ("IROH_TUNNEL_SCM", None),
        ]);
        match error_of(load_config::<ServeConfig>(&file)) {
            CliError::Config(m) => assert!(m.contains("config keygen"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(std::fs::read(&file).unwrap(), before);
        let sddl_after = std::process::Command::new("icacls")
            .arg(&file)
            .output()
            .unwrap()
            .stdout;
        assert_eq!(sddl_after, sddl_before);
    }

    #[test]
    fn invalid_toml_reports_failed_to_parse() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write(tmp.path(), "this is = = not toml");
        let sid = sid_text();
        let _env = EnvGuard::set(&[
            ("IROH_TUNNEL_TRUSTED_SID", Some(&sid)),
            ("IROH_TUNNEL_STATE_DIR", None),
            ("IROH_TUNNEL_SCM", None),
        ]);
        let err = load_config::<ServeConfig>(&file).unwrap_err();
        assert!(format!("{err:#}").contains("failed to parse"), "{err:#}");
    }

    #[test]
    fn writable_config_is_permission_error() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write(tmp.path(), &format!("[node]\nsecret_key = \"{KEY}\"\n"));
        let granted = std::process::Command::new("icacls")
            .args([file.to_str().unwrap(), "/grant", "*S-1-1-0:M"])
            .output()
            .is_ok_and(|o| o.status.success());
        assert!(granted);
        let sid = sid_text();
        let _env = EnvGuard::set(&[
            ("IROH_TUNNEL_TRUSTED_SID", Some(&sid)),
            ("IROH_TUNNEL_STATE_DIR", None),
            ("IROH_TUNNEL_SCM", None),
        ]);
        assert!(matches!(
            error_of(load_config::<ServeConfig>(&file)),
            CliError::Permission(_)
        ));
    }
}
