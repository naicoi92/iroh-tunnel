//! Platform selection helpers of the service backends (private).

use std::path::Path;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::PathBuf;

use anyhow::Result;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::effective_binary::EffectiveBinary;
use crate::Tier;

/// Load a role config for `run`.
///
/// A registration from an earlier version is refused first, before any read
/// (design D4). Under a system service ([`super::under_system_service`]) the
/// state directory and the config are verified against the admin-only owners
/// and the config is parsed only from the verified handle; a missing key is
/// then generated and persisted by the caller through the normal store. Any
/// other run (user tier, foreground elevated run) is a plain read.
///
/// # Errors
///
/// [`CliError::Config`] for a legacy registration or an invalid config,
/// [`CliError::Permission`] for an unsafe path under a system service.
///
/// [`CliError::Config`]: crate::error::CliError::Config
/// [`CliError::Permission`]: crate::error::CliError::Permission
pub(crate) fn load_config<C: crate::config::RoleDoc>(path: &Path) -> Result<C> {
    load_config_in::<C>(
        path,
        crate::Tier::current(),
        super::is_elevated(),
        &super::process_env,
    )
}

/// [`load_config`] with the tier, the elevation and the environment
/// injected.
fn load_config_in<C: crate::config::RoleDoc>(
    path: &Path,
    tier: crate::Tier,
    elevated: bool,
    env: super::EnvLookup<'_>,
) -> Result<C> {
    if is_legacy_registration(elevated, env) {
        return Err(legacy_refusal(C::ROLE)?.into());
    }
    if !super::system_service_marked(tier, env) {
        return crate::config::load_role_config::<C>(path);
    }
    let text = read_system_config(path, env)?;
    crate::config::parse_role_config::<C>(&text, path)
}

/// The verified config text of a system service.
#[cfg(unix)]
fn read_system_config(path: &Path, env: super::EnvLookup<'_>) -> Result<String> {
    super::trust::read_system_config(path, env)
}

/// The verified config text of a system service: the ownership walk, then
/// the confidentiality of the config directory and file, before parsing.
#[cfg(windows)]
fn read_system_config(path: &Path, env: super::EnvLookup<'_>) -> Result<String> {
    use super::trust_windows::{read_verified, verify_confidential_with, verify_dir};
    use crate::status::writer::ENV_STATE_DIR;

    let trusted = super::admin_trust(env)?;
    if let Some(state) = env(ENV_STATE_DIR).filter(|d| !d.is_empty()) {
        verify_dir(Path::new(&state), &trusted)?;
    }
    let text = read_verified(path, &trusted)?;
    if let Some(dir) = path.parent() {
        verify_confidential_with(dir, &trusted)?;
    }
    verify_confidential_with(path, &trusted)?;
    Ok(text)
}

/// No system service exists on other targets, so nothing is verified.
#[cfg(not(any(unix, windows)))]
fn read_system_config(path: &Path, _env: super::EnvLookup<'_>) -> Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config: {}", path.display()))
}

/// Pinned-trust variables an earlier version wrote into system registrations.
const LEGACY_TRUST_VARS: [&str; 2] = ["IROH_TUNNEL_TRUSTED_UID", "IROH_TUNNEL_TRUSTED_SID"];

/// Variables systemd (`INVOCATION_ID`) and launchd (`XPC_SERVICE_NAME`) set
/// for the processes they start.
const MANAGER_VARS: [&str; 2] = ["INVOCATION_ID", "XPC_SERVICE_NAME"];

/// Whether this process was started by a system registration of an earlier
/// version: one that pins a user's files (`IROH_TUNNEL_TRUSTED_UID`/`_SID`),
/// or an even older one where an elevated process runs under systemd or
/// launchd without the [`super::ENV_SERVICE`] marker. A foreground elevated
/// run sets no manager variable; an old BusyBox script cannot be told apart
/// from one and is treated as such.
fn is_legacy_registration(elevated: bool, env: super::EnvLookup<'_>) -> bool {
    if LEGACY_TRUST_VARS.iter().any(|name| env(name).is_some()) {
        return true;
    }
    let marked = env(super::ENV_SERVICE).is_some_and(|v| v == "1");
    elevated && !marked && MANAGER_VARS.iter().any(|name| env(name).is_some())
}

/// The configuration error for a legacy registration of `role`, naming the
/// role's system-tier config path.
///
/// # Errors
///
/// [`CliError::Config`] when the system config path cannot be resolved.
///
/// [`CliError::Config`]: crate::error::CliError::Config
fn legacy_refusal(role: crate::Role) -> Result<crate::error::CliError> {
    let config = crate::paths::default_config_path_for(role.name(), crate::Tier::System)?;
    Ok(crate::error::CliError::Config(legacy_refusal_text(
        role.name(),
        &config,
        cfg!(windows),
    )))
}

/// The legacy-registration guidance for `role` and its system `config`; the
/// Windows wording replaces `sudo` with an elevated terminal.
fn legacy_refusal_text(role: &str, config: &Path, windows: bool) -> String {
    let config = config.display();
    if windows {
        format!(
            "this service was registered by an earlier version and points at a user's files; \
             create {config} (for example `iroh-tunnel {role} config keygen` from an elevated \
             terminal (Run as administrator)) and re-run `iroh-tunnel {role} service install` \
             from an elevated terminal (Run as administrator)"
        )
    } else {
        format!(
            "this service was registered by an earlier version and points at a user's files; \
             create {config} (for example `sudo iroh-tunnel {role} config keygen`) and re-run \
             `sudo iroh-tunnel {role} service install`"
        )
    }
}

/// Whether the service of `tier` runs as root: a system service always, and
/// any service on a host whose only registrations are system-wide (BusyBox
/// init).
pub(crate) fn root_run(tier: Tier, system_wide_only: bool) -> bool {
    tier == Tier::System || system_wide_only
}

/// What the service manager of this host will run for the role's root-run
/// service, from the loaded definition.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn effective_binary(native: &mut super::native::Native, role: &str) -> EffectiveBinary {
    match super::backend::Backend::select(Tier::System) {
        Ok(backend) => backend.effective_binary(native, role),
        Err(error) => EffectiveBinary::Unknown(format!("{error:#}")),
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
/// `Backend::select` picks its backend on the same condition.
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
mod load_tests {
    use super::*;
    use crate::config::ServeConfig;
    use crate::error::CliError;
    use crate::Tier;
    use std::collections::HashMap;
    use std::ffi::OsString;

    const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    /// An injected environment holding exactly `vars`.
    fn env_of(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
            .collect();
        move |name| map.get(name).cloned()
    }

    fn config_message(r: Result<ServeConfig>) -> String {
        match r.map(drop).unwrap_err().downcast::<CliError>() {
            Ok(CliError::Config(m)) => m,
            other => panic!("expected a config error, got {other:?}"),
        }
    }

    fn load(path: &Path, tier: Tier, elevated: bool, vars: &[(&str, &str)]) -> Result<ServeConfig> {
        load_config_in::<ServeConfig>(path, tier, elevated, &env_of(vars))
    }

    #[test]
    fn legacy_trusted_uid_is_refused_before_reading_config() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("alice/serve.toml");
        let vars = [("IROH_TUNNEL_TRUSTED_UID", "1000"), (ENV_SERVICE_KEY, "1")];
        // A missing file would be a read error; the refusal comes first.
        let msg = config_message(load(&missing, Tier::System, true, &vars));
        let expected = crate::paths::default_config_path_for("serve", Tier::System).unwrap();
        assert!(msg.contains(&expected.display().to_string()), "{msg}");
        assert!(msg.contains("earlier version"), "{msg}");
        assert!(!missing.exists());
        let unix = legacy_refusal_text("serve", Path::new("/etc/iroh-tunnel/serve.toml"), false);
        assert!(unix.contains("/etc/iroh-tunnel/serve.toml"), "{unix}");
        assert!(
            unix.contains("sudo iroh-tunnel serve service install"),
            "{unix}"
        );
    }

    #[test]
    fn legacy_trusted_sid_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("serve.toml");
        let vars = [("IROH_TUNNEL_TRUSTED_SID", "S-1-5-21-1-2-3-1001")];
        let msg = config_message(load(&missing, Tier::System, true, &vars));
        assert!(msg.contains("service install"), "{msg}");
        let config = Path::new(r"C:\ProgramData\iroh-tunnel\config\serve.toml");
        let windows = legacy_refusal_text("serve", config, true);
        assert!(
            windows.contains(r"C:\ProgramData\iroh-tunnel\config\serve.toml"),
            "{windows}"
        );
        assert!(
            windows.contains("`iroh-tunnel serve service install`"),
            "{windows}"
        );
        assert!(
            windows.contains("elevated terminal (Run as administrator)"),
            "{windows}"
        );
        assert!(!windows.contains("sudo"), "{windows}");
    }

    #[test]
    fn root_under_manager_without_service_marker_is_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("serve.toml");
        for manager in ["INVOCATION_ID", "XPC_SERVICE_NAME"] {
            let msg = config_message(load(&missing, Tier::System, true, &[(manager, "x")]));
            assert!(msg.contains("earlier version"), "{manager}: {msg}");
        }
        // With the marker it is a current registration, not a legacy one.
        assert!(!is_legacy_registration(
            true,
            &env_of(&[("INVOCATION_ID", "x"), (ENV_SERVICE_KEY, "1")])
        ));
        // A non-elevated process under a manager is a user service.
        assert!(!is_legacy_registration(
            false,
            &env_of(&[("INVOCATION_ID", "x")])
        ));
    }

    #[test]
    fn foreground_root_run_is_not_walked() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("serve.toml");
        // World-writable: a walk would refuse it; a foreground run reads it.
        std::fs::write(&file, format!("[node]\nsecret_key = \"{KEY}\"\n")).unwrap();
        open_up(&file);
        let cfg = load(&file, Tier::System, true, &[]).unwrap();
        assert_eq!(cfg.node.secret_key, KEY);
    }

    #[test]
    fn user_service_with_marker_is_not_walked() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("serve.toml");
        std::fs::write(&file, format!("[node]\nsecret_key = \"{KEY}\"\n")).unwrap();
        open_up(&file);
        let vars = [(ENV_SERVICE_KEY, "1"), ("INVOCATION_ID", "x")];
        let cfg = load(&file, Tier::User, false, &vars).unwrap();
        assert_eq!(cfg.node.secret_key, KEY);
    }

    #[test]
    fn system_service_marker_needs_the_system_tier_and_value_one() {
        use crate::service::{system_service_marked, ENV_SCM};
        assert!(system_service_marked(
            Tier::System,
            &env_of(&[(ENV_SERVICE_KEY, "1")])
        ));
        assert!(system_service_marked(
            Tier::System,
            &env_of(&[(ENV_SCM, "1")])
        ));
        assert!(!system_service_marked(
            Tier::System,
            &env_of(&[(ENV_SERVICE_KEY, "0")])
        ));
        assert!(!system_service_marked(
            Tier::User,
            &env_of(&[(ENV_SERVICE_KEY, "1")])
        ));
        assert!(!system_service_marked(Tier::System, &env_of(&[])));
    }

    const ENV_SERVICE_KEY: &str = crate::service::ENV_SERVICE;

    /// Make `path` writable by group and others (Unix); a walked load
    /// refuses it. Elsewhere a no-op.
    fn open_up(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666)).unwrap();
        }
        #[cfg(not(unix))]
        let _ = path;
    }

    #[cfg(unix)]
    mod unix_walk {
        use super::*;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        use std::path::PathBuf;

        fn me() -> String {
            // SAFETY: geteuid(2) has no preconditions and cannot fail.
            unsafe { libc::geteuid() }.to_string()
        }

        /// `<tmp>/etc/serve.toml` (0600) and `<tmp>/state`, both 0755 dirs.
        fn fixture(body: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().canonicalize().unwrap();
            let etc = root.join("etc");
            let state = root.join("state");
            for dir in [&etc, &state] {
                std::fs::create_dir(dir).unwrap();
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let file = etc.join("serve.toml");
            std::fs::write(&file, body).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
            (tmp, file, state)
        }

        fn permission_message(r: Result<ServeConfig>) -> String {
            match r.map(drop).unwrap_err().downcast::<CliError>() {
                Ok(CliError::Permission(m)) => m,
                other => panic!("expected a permission error, got {other:?}"),
            }
        }

        #[test]
        fn system_service_reads_admin_owned_config() {
            let (_tmp, file, state) = fixture(&format!("[node]\nsecret_key = \"{KEY}\"\n"));
            let me = me();
            let vars = [
                (ENV_SERVICE_KEY, "1"),
                ("IROH_TUNNEL_TEST_TRUSTED_OWNER", me.as_str()),
                ("IROH_TUNNEL_STATE_DIR", state.to_str().unwrap()),
            ];
            let cfg = load(&file, Tier::System, true, &vars).unwrap();
            assert_eq!(cfg.node.secret_key, KEY);
        }

        #[test]
        fn system_service_refuses_untrusted_owner() {
            if me() == "0" {
                // Root owns the fixture and root is always trusted.
                return;
            }
            let (_tmp, file, _state) = fixture(&format!("[node]\nsecret_key = \"{KEY}\"\n"));
            let msg =
                permission_message(load(&file, Tier::System, true, &[(ENV_SERVICE_KEY, "1")]));
            assert!(msg.contains("owner uid"), "{msg}");
        }

        #[test]
        fn system_service_refuses_a_loosened_config_naming_mode() {
            let (_tmp, file, _state) = fixture("");
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o666)).unwrap();
            let me = me();
            let vars = [
                (ENV_SERVICE_KEY, "1"),
                ("IROH_TUNNEL_TEST_TRUSTED_OWNER", me.as_str()),
            ];
            let msg = permission_message(load(&file, Tier::System, true, &vars));
            assert!(
                msg.contains(file.to_str().unwrap()) && msg.contains("0666"),
                "{msg}"
            );
        }

        #[test]
        fn symlinked_component_is_refused() {
            let (tmp, file, _state) = fixture("");
            let link = tmp.path().canonicalize().unwrap().join("link");
            std::os::unix::fs::symlink(file.parent().unwrap(), &link).unwrap();
            let me = me();
            let vars = [
                (ENV_SERVICE_KEY, "1"),
                ("IROH_TUNNEL_TEST_TRUSTED_OWNER", me.as_str()),
            ];
            let msg = permission_message(load(&link.join("serve.toml"), Tier::System, true, &vars));
            assert!(msg.contains("symlink"), "{msg}");
        }

        #[test]
        fn system_service_refuses_an_unsafe_state_dir() {
            let (_tmp, file, state) = fixture("");
            std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o777)).unwrap();
            let me = me();
            let vars = [
                (ENV_SERVICE_KEY, "1"),
                ("IROH_TUNNEL_TEST_TRUSTED_OWNER", me.as_str()),
                ("IROH_TUNNEL_STATE_DIR", state.to_str().unwrap()),
            ];
            let msg = permission_message(load(&file, Tier::System, true, &vars));
            assert!(msg.contains(state.to_str().unwrap()), "{msg}");
        }

        #[test]
        fn missing_key_is_persisted_under_system_service() {
            let (_tmp, file, _state) = fixture("[node]\nsecret_key = \"\"\n");
            let me = me();
            let vars = [
                (ENV_SERVICE_KEY, "1"),
                ("IROH_TUNNEL_TEST_TRUSTED_OWNER", me.as_str()),
            ];
            // `run` loads, then persists the generated key through the store.
            let mut cfg = load(&file, Tier::System, true, &vars).unwrap();
            crate::config::resolve_and_persist_key(&mut cfg, &file).unwrap();
            let text = std::fs::read_to_string(&file).unwrap();
            assert!(text.contains(&cfg.node.secret_key) && !cfg.node.secret_key.is_empty());
            let meta = std::fs::metadata(&file).unwrap();
            assert_eq!(meta.mode() & 0o777, 0o600);
            assert_eq!(meta.uid().to_string(), me);
        }

        #[test]
        fn malformed_test_owner_is_a_config_error() {
            let (_tmp, file, _state) = fixture("");
            let vars = [
                (ENV_SERVICE_KEY, "1"),
                ("IROH_TUNNEL_TEST_TRUSTED_OWNER", "alice"),
            ];
            let msg = config_message(load(&file, Tier::System, true, &vars));
            assert!(msg.contains("IROH_TUNNEL_TEST_TRUSTED_OWNER"), "{msg}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-only guard: on Linux the runtime probe must exist and be bool.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_is_systemd_is_bool() {
        let _ = linux_is_systemd();
    }

    #[test]
    fn root_run_covers_system_tier_and_system_wide_only_hosts() {
        assert!(root_run(Tier::System, false));
        assert!(!root_run(Tier::User, false));
        assert!(root_run(Tier::User, true));
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

#[cfg(test)]
#[cfg(windows)]
mod windows_tests {
    use super::*;
    use crate::config::ServeConfig;
    use crate::error::CliError;
    use crate::service::trust_windows::token_user;
    use crate::Tier;
    use std::ffi::OsString;

    const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    fn write(dir: &Path, body: &str) -> std::path::PathBuf {
        let file = dir.join("serve.toml");
        std::fs::write(&file, body).unwrap();
        file
    }

    fn icacls(args: &[&str]) -> Vec<u8> {
        let out = std::process::Command::new("icacls")
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "icacls {args:?}");
        out.stdout
    }

    /// A system service started by the SCM whose test build trusts the
    /// current user besides the admin principals.
    fn load(path: &Path) -> Result<ServeConfig> {
        let sid = OsString::from(token_user()?.0);
        let env = move |name: &str| match name {
            "IROH_TUNNEL_SCM" => Some(OsString::from("1")),
            "IROH_TUNNEL_TEST_TRUSTED_OWNER" => Some(sid.clone()),
            _ => None,
        };
        load_config_in::<ServeConfig>(path, Tier::System, true, &env)
    }

    fn error_of(r: Result<ServeConfig>) -> CliError {
        match r.map(drop).unwrap_err().downcast::<CliError>() {
            Ok(e) => e,
            Err(e) => panic!("expected CliError: {e:#}"),
        }
    }

    #[test]
    fn empty_secret_key_under_service_is_persisted() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write(tmp.path(), "[node]\nsecret_key = \"\"\n");
        let mut cfg = load(&file).unwrap();
        crate::config::resolve_and_persist_key(&mut cfg, &file).unwrap();
        assert!(!cfg.node.secret_key.is_empty());
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains(&cfg.node.secret_key), "key persisted");
    }

    #[test]
    fn invalid_toml_reports_failed_to_parse() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write(tmp.path(), "this is = = not toml");
        let err = load(&file).unwrap_err();
        assert!(format!("{err:#}").contains("failed to parse"), "{err:#}");
    }

    #[test]
    fn writable_config_is_permission_error() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write(tmp.path(), &format!("[node]\nsecret_key = \"{KEY}\"\n"));
        icacls(&[file.to_str().unwrap(), "/grant", "*S-1-1-0:M"]);
        assert!(matches!(error_of(load(&file)), CliError::Permission(_)));
    }

    #[test]
    fn service_start_refuses_readable_config_with_key() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write(tmp.path(), &format!("[node]\nsecret_key = \"{KEY}\"\n"));
        let path = file.to_str().unwrap();
        icacls(&[path, "/grant", "*S-1-5-32-545:R"]);
        let before = icacls(&[path]);
        match error_of(load(&file)) {
            CliError::Permission(m) => {
                assert!(m.contains("S-1-5-32-545"), "{m}");
                assert!(!m.contains(KEY), "{m}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(icacls(&[path]), before, "DACL unchanged");
    }

    #[test]
    fn service_start_refuses_a_readable_config_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write(tmp.path(), &format!("[node]\nsecret_key = \"{KEY}\"\n"));
        icacls(&[tmp.path().to_str().unwrap(), "/grant", "*S-1-5-32-545:R"]);
        assert!(matches!(error_of(load(&file)), CliError::Permission(m) if m.contains("Users")));
    }

    #[test]
    fn config_in_user_profile_is_refused_without_a_test_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write(tmp.path(), &format!("[node]\nsecret_key = \"{KEY}\"\n"));
        let env = |name: &str| (name == "IROH_TUNNEL_SCM").then(|| OsString::from("1"));
        let r = load_config_in::<ServeConfig>(&file, Tier::System, true, &env);
        match error_of(r) {
            CliError::Permission(m) => assert!(m.contains("untrusted SID"), "{m}"),
            other => panic!("{other:?}"),
        }
    }
}
