//! Platform selection helpers of the service backends (private).

use std::path::Path;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::PathBuf;

use anyhow::Result;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::effective_binary::EffectiveBinary;
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
        crate::config::load_role_config::<C>(path)
    }
}

/// Windows twin of `trust::load_config`: under a system service the state
/// directory and the config are verified and the config is parsed only from
/// the verified handle. The service never writes into the user's file, so a
/// missing key is an error; with a non-empty key `resolve_and_persist_key` has
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
        return crate::config::load_role_config::<C>(path);
    };
    let trusted = TrustedSet::for_user_files(&sid);
    if let Some(state) = std::env::var_os(ENV_STATE_DIR).filter(|d| !d.is_empty()) {
        verify_dir(Path::new(&state), &trusted)?;
    }
    let text = read_verified(path, &trusted)?;
    let mut cfg = crate::config::parse_role_config::<C>(&text, path)?;
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

/// Whether the service of `scope` runs as root: a system service always, and
/// a user-scope request on a host whose only registrations are system-wide
/// (BusyBox init).
pub(crate) fn root_run(scope: ServiceScope, system_wide_only: bool) -> bool {
    scope == ServiceScope::System || system_wide_only
}

/// What the service manager of this host will run for the role's root-run
/// service, from the loaded definition.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn effective_binary(native: &mut super::native::Native, role: &str) -> EffectiveBinary {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            super::backend::systemd::effective_binary(native, role)
        } else {
            let _ = native;
            super::backend::busybox::effective_binary(role)
        }
    }
    #[cfg(target_os = "macos")]
    {
        super::backend::launchd::effective_binary(native, role)
    }
}

/// Files and directories whose text may name a service binary.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug, Clone, Default)]
pub(crate) struct RegistrationSources {
    /// Registration files (unit, init script, plist).
    pub files: Vec<PathBuf>,
    /// Drop-in directories whose entries are all read.
    pub drop_in_dirs: Vec<PathBuf>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl RegistrationSources {
    /// Every system registration location of `role` on any Unix backend.
    pub(crate) fn for_role(role: &str) -> Self {
        let suffix = super::registration::instance_suffix();
        let unit = format!("iroh-tunnel-{role}{suffix}.service");
        Self {
            files: vec![
                PathBuf::from("/etc/systemd/system").join(&unit),
                PathBuf::from(format!("/etc/init.d/S96iroh-tunnel-{role}")),
                PathBuf::from(format!(
                    "/Library/LaunchDaemons/dev.iroh-tunnel.{role}{suffix}.plist"
                )),
            ],
            drop_in_dirs: vec![
                PathBuf::from(format!("/etc/systemd/system/{unit}.d")),
                PathBuf::from(format!("/run/systemd/system/{unit}.d")),
            ],
        }
    }

    /// Whether any source text contains `needle`. A missing file or directory
    /// is no reference; any other read error counts as one.
    fn mentions(&self, needle: &str) -> bool {
        let file_mentions = |path: &Path| match std::fs::read(path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).contains(needle),
            Err(e) => e.kind() != std::io::ErrorKind::NotFound,
        };
        if self.files.iter().any(|f| file_mentions(f)) {
            return true;
        }
        self.drop_in_dirs
            .iter()
            .any(|dir| match std::fs::read_dir(dir) {
                Ok(entries) => entries
                    .map(|entry| entry.map(|e| e.path()))
                    .filter(|entry| {
                        entry
                            .as_ref()
                            .map_or(true, |p| p.extension().is_some_and(|x| x == "conf"))
                    })
                    .any(|entry| entry.map_or(true, |p| file_mentions(&p))),
                Err(e) => e.kind() != std::io::ErrorKind::NotFound,
            })
    }
}

/// Whether a managed binary copy (`needle` is its path) is still referenced by
/// a registration. True on any match and on any read or query failure.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn managed_copy_referenced(
    native: &mut super::native::Native,
    role: &str,
    sources: &RegistrationSources,
    needle: &str,
) -> bool {
    referenced_by(&effective_binary(native, role), sources, needle)
}

/// [`managed_copy_referenced`] over an already-run loaded-definition query.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn referenced_by(loaded: &EffectiveBinary, sources: &RegistrationSources, needle: &str) -> bool {
    let loaded_hit = match loaded {
        EffectiveBinary::NotLoaded => false,
        EffectiveBinary::Binary(p) => p.to_string_lossy().contains(needle),
        EffectiveBinary::Unknown(_) => true,
    };
    loaded_hit || sources.mentions(needle)
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

    #[test]
    fn root_run_covers_system_scope_and_system_wide_only_hosts() {
        assert!(root_run(ServiceScope::System, false));
        assert!(!root_run(ServiceScope::User, false));
        assert!(root_run(ServiceScope::User, true));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    mod referenced {
        use super::super::*;
        #[cfg(target_os = "macos")]
        use crate::service::native::fake::{call, output, Events, FakeRunner};
        #[cfg(target_os = "macos")]
        use crate::service::native::{Native, OutputMode};

        const NEEDLE: &str = "/var/lib/iroh-tunnel/bin/serve/iroh-tunnel";

        fn sources(dir: &Path) -> RegistrationSources {
            RegistrationSources {
                files: vec![dir.join("unit")],
                drop_in_dirs: vec![dir.join("unit.d")],
            }
        }

        #[test]
        fn for_role_lists_every_registration_location() {
            let sources = RegistrationSources::for_role("serve");
            assert!(sources
                .files
                .contains(&PathBuf::from("/etc/init.d/S96iroh-tunnel-serve")));
            assert!(sources.drop_in_dirs.contains(&PathBuf::from(
                "/run/systemd/system/iroh-tunnel-serve.service.d"
            )));
        }

        #[test]
        fn managed_copy_referenced_by_file() {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::write(tmp.path().join("unit"), format!("ExecStart={NEEDLE} run")).unwrap();
            assert!(referenced_by(
                &EffectiveBinary::NotLoaded,
                &sources(tmp.path()),
                NEEDLE
            ));
        }

        #[test]
        fn managed_copy_referenced_by_drop_in() {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::create_dir(tmp.path().join("unit.d")).unwrap();
            std::fs::write(tmp.path().join("unit.d/override.conf"), NEEDLE).unwrap();
            assert!(referenced_by(
                &EffectiveBinary::NotLoaded,
                &sources(tmp.path()),
                NEEDLE
            ));
        }

        #[cfg(unix)]
        #[test]
        fn managed_copy_referenced_when_a_file_is_unreadable() {
            use std::os::unix::fs::PermissionsExt;
            let tmp = tempfile::tempdir().unwrap();
            let file = tmp.path().join("unit");
            std::fs::write(&file, "nothing").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
            // Root can read mode 000, so the case does not arise there.
            if std::fs::read(&file).is_err() {
                assert!(referenced_by(
                    &EffectiveBinary::NotLoaded,
                    &sources(tmp.path()),
                    NEEDLE
                ));
            }
        }

        #[test]
        fn managed_copy_not_referenced_when_nothing_names_it() {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::write(tmp.path().join("unit"), "ExecStart=/other").unwrap();
            assert!(!referenced_by(
                &EffectiveBinary::NotLoaded,
                &sources(tmp.path()),
                NEEDLE
            ));
            let other = EffectiveBinary::Binary(PathBuf::from("/other"));
            assert!(!referenced_by(&other, &sources(tmp.path()), NEEDLE));
        }

        #[test]
        fn managed_copy_referenced_by_conf_drop_in_only() {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("unit.d");
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("override.conf.bak"), NEEDLE).unwrap();
            assert!(!referenced_by(
                &EffectiveBinary::NotLoaded,
                &sources(tmp.path()),
                NEEDLE
            ));
            std::fs::write(dir.join("override.conf"), NEEDLE).unwrap();
            assert!(referenced_by(
                &EffectiveBinary::NotLoaded,
                &sources(tmp.path()),
                NEEDLE
            ));
        }

        #[test]
        fn managed_copy_referenced_when_loaded_path_surrounds_the_needle() {
            let tmp = tempfile::tempdir().unwrap();
            let loaded = EffectiveBinary::Binary(PathBuf::from(format!("/pre{NEEDLE}-post")));
            assert!(referenced_by(&loaded, &sources(tmp.path()), NEEDLE));
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn managed_copy_referenced_queries_the_host_backend() {
            use crate::service::native::fake::{output, Events, FakeRunner};
            use crate::service::native::{Native, OutputMode};
            let tmp = tempfile::tempdir().unwrap();
            let events = Events::default();
            // Answers the systemd query; the BusyBox backend reads no command.
            let mut fake = FakeRunner::new(
                &events,
                vec![output(0, "LoadState=not-found\nExecStart=\n", "")],
            );
            let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
            let mut native = Native::new(OutputMode::Inherit, &mut runner);
            assert!(!managed_copy_referenced(
                &mut native,
                "zz-unit-test-role",
                &sources(tmp.path()),
                NEEDLE
            ));
        }

        #[test]
        fn managed_copy_referenced_by_loaded_definition_or_unknown() {
            let tmp = tempfile::tempdir().unwrap();
            let loaded = EffectiveBinary::Binary(PathBuf::from(NEEDLE));
            assert!(referenced_by(&loaded, &sources(tmp.path()), NEEDLE));
            assert!(referenced_by(
                &EffectiveBinary::Unknown("x".into()),
                &sources(tmp.path()),
                NEEDLE
            ));
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn managed_copy_referenced_queries_the_host_backend() {
            let tmp = tempfile::tempdir().unwrap();
            let events = Events::default();
            let mut fake = FakeRunner::new(
                &events,
                vec![output(
                    113,
                    "",
                    "Could not find service \"x\" in domain for system",
                )],
            );
            let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
            let mut native = Native::new(OutputMode::Inherit, &mut runner);
            assert!(!managed_copy_referenced(
                &mut native,
                "serve",
                &sources(tmp.path()),
                NEEDLE
            ));
            assert_eq!(
                *events.borrow(),
                vec![call(
                    "launchctl",
                    &["print", "system/dev.iroh-tunnel.serve"],
                    OutputMode::Capture
                )]
            );
        }
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
