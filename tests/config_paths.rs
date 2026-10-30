//! Issue #84 end-to-end: platform config/state defaults, explicit overrides,
//! actionable path errors and safe macOS legacy migration (CFG-01, CFG-02;
//! acceptance cases CFG-PATH-01, CFG-MIGRATE-01, CFG-ERROR-01).
//!
//! Every test runs the real binary with an isolated `HOME`. XDG variables are
//! pointed elsewhere to prove they are ignored. Windows LocalAppData is covered
//! by `config_paths_windows.rs`; this file is Unix-only.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tunnel(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
        .args(args)
        .env_remove("IROH_TUNNEL_TEST_CONFIG_DIR")
        .env_remove("IROH_TUNNEL_STATE_DIR")
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .env("XDG_STATE_HOME", home.join("xdg-state"))
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .output()
        .expect("spawn iroh-tunnel")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn home() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    // Canonical so macOS /var → /private/var does not break path equality.
    let path = dir.path().canonicalize().unwrap();
    (dir, path)
}

fn new_config(home: &Path, role: &str) -> PathBuf {
    home.join(".config/iroh-tunnel")
        .join(format!("{role}.toml"))
}

#[test]
fn config_default_is_home_dot_config_ignoring_xdg() {
    let (_tmp, home) = home();
    for role in ["serve", "access"] {
        let out = tunnel(&home, &[role, "config", "path"]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert_eq!(
            stdout(&out).trim(),
            new_config(&home, role).display().to_string()
        );
    }

    let out = tunnel(&home, &["serve", "config", "keygen"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(new_config(&home, "serve").is_file());
    assert!(
        !home.join("xdg-config").exists(),
        "XDG_CONFIG_HOME must be ignored"
    );
}

#[test]
fn state_default_is_home_local_state_ignoring_xdg() {
    let (_tmp, home) = home();
    let out = tunnel(&home, &["serve", "status"]);
    assert!(!out.status.success());
    let expected = home.join(".local/state/iroh-tunnel/serve-status.json");
    assert!(
        stderr(&out).contains(&expected.display().to_string()),
        "status must look under ~/.local/state: {}",
        stderr(&out)
    );
}

#[test]
fn explicit_config_override_wins() {
    let (_tmp, home) = home();
    let custom = home.join("elsewhere/custom.toml");
    let out = tunnel(
        &home,
        &[
            "serve",
            "config",
            "keygen",
            "--config",
            custom.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(custom.is_file());
    assert!(!new_config(&home, "serve").exists());
}

#[test]
fn explicit_state_override_wins() {
    let (_tmp, home) = home();
    let state = home.join("custom-state");
    let out = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
        .args(["access", "status"])
        .env("HOME", &home)
        .env("IROH_TUNNEL_STATE_DIR", &state)
        .output()
        .unwrap();
    assert!(stderr(&out).contains(&state.join("access-status.json").display().to_string()));
}

#[test]
fn relative_home_is_rejected_with_guidance() {
    let (_tmp, home) = home();
    let out = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
        .args(["serve", "config", "path"])
        .current_dir(&home)
        .env_remove("IROH_TUNNEL_TEST_CONFIG_DIR")
        .env("HOME", "relative-home")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("HOME"), "{err}");
    assert!(
        err.contains("--config"),
        "guidance must name the override: {err}"
    );
    assert!(!home.join("relative-home").exists());
}

#[test]
fn unreadable_config_reports_path() {
    let (_tmp, home) = home();
    let path = new_config(&home, "serve");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "[node]\nsecret_key = \"\"\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&path).is_ok() {
        return; // running as root: permissions are not enforced
    }
    let out = tunnel(&home, &["serve", "config", "list"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains(&path.display().to_string()),
        "{}",
        stderr(&out)
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[node]\nsecret_key = \"\"\n"
    );
}

#[test]
fn invalid_config_reports_path_with_config_error_exit() {
    let (_tmp, home) = home();
    let path = new_config(&home, "serve");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "not = [valid toml").unwrap();
    let out = tunnel(&home, &["serve", "config", "list"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains(&path.display().to_string()),
        "{}",
        stderr(&out)
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "not = [valid toml");
}

#[cfg(target_os = "macos")]
mod macos_migration {
    use super::*;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    const LEGACY_KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
    const NEW_KEY: &str = "Hx4dHBsaGRgXFhUUExIREA8ODQwLCgkIBwYFBAMCAQA=";

    fn legacy_config(home: &Path, role: &str) -> PathBuf {
        home.join("Library/Application Support/iroh-tunnel")
            .join(format!("{role}.toml"))
    }

    fn body(key: &str, service: &str) -> String {
        format!(
            "# keep me\n[node]\nsecret_key = \"{key}\"\n\n[[services]]\nname = \"{service}\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 5432\n"
        )
    }

    fn write(path: &Path, content: &str, mode: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "serve.toml" && n != "access.toml")
            .collect()
    }

    #[test]
    fn new_config_wins_without_consulting_unreadable_legacy() {
        let (_tmp, home) = home();
        let legacy = legacy_config(&home, "serve");
        write(&legacy, &body(LEGACY_KEY, "legacy"), 0o000);
        write(&new_config(&home, "serve"), &body(NEW_KEY, "fresh"), 0o600);
        // Also lock the legacy directory: any lookup would fail loudly.
        let legacy_dir = legacy.parent().unwrap();
        std::fs::set_permissions(legacy_dir, std::fs::Permissions::from_mode(0o000)).unwrap();

        let out = tunnel(&home, &["serve", "config", "list"]);
        std::fs::set_permissions(legacy_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        assert!(stdout(&out).contains("fresh"));
        assert!(!stdout(&out).contains("legacy"));
        assert!(std::fs::read_to_string(new_config(&home, "serve"))
            .unwrap()
            .contains(NEW_KEY));
    }

    #[test]
    fn legacy_is_copied_preserving_source_identity_and_permissions() {
        let (_tmp, home) = home();
        for (role, source_mode) in [("serve", 0o644), ("access", 0o600)] {
            let legacy = legacy_config(&home, role);
            let content = body(LEGACY_KEY, &format!("{role}-svc"));
            let content = if role == "access" {
                content.replace(
                    "host = ",
                    "node_id = \"0000000000000000000000000000000000000000000000000000000000000000\"\nhost = ",
                )
            } else {
                content
            };
            write(&legacy, &content, source_mode);
            let before = std::fs::metadata(&legacy).unwrap().modified().unwrap();

            let out = tunnel(&home, &[role, "config", "show"]);
            assert!(out.status.success(), "{}", stderr(&out));
            assert_eq!(stdout(&out), content, "migrated bytes must be identical");

            let dest = new_config(&home, role);
            assert_eq!(std::fs::read_to_string(&dest).unwrap(), content);
            assert_eq!(
                mode(&dest),
                0o600,
                "secret must not be group/world readable"
            );
            assert_eq!(mode(dest.parent().unwrap()), 0o700);
            assert_eq!(
                std::fs::read_to_string(&legacy).unwrap(),
                content,
                "source kept"
            );
            assert_eq!(mode(&legacy), source_mode, "source permissions untouched");
            assert_eq!(
                std::fs::metadata(&legacy).unwrap().modified().unwrap(),
                before
            );
            assert!(stderr(&out).contains("migrated"), "{}", stderr(&out));
        }
        assert!(leftovers(&home.join(".config/iroh-tunnel")).is_empty());

        // Restart: the copy is used, the legacy file is never re-read.
        std::fs::write(legacy_config(&home, "serve"), "not = [toml").unwrap();
        let out = tunnel(&home, &["serve", "config", "list"]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert!(stdout(&out).contains("serve-svc"));
    }

    #[test]
    fn copy_failure_never_creates_a_substitute_identity() {
        let (_tmp, home) = home();
        let legacy = legacy_config(&home, "serve");
        let content = body(LEGACY_KEY, "legacy");
        write(&legacy, &content, 0o000);
        if std::fs::read(&legacy).is_ok() {
            return; // root ignores permissions
        }
        for args in [
            &["serve", "config", "keygen"][..],
            &[
                "serve",
                "config",
                "add",
                "--name",
                "web",
                "--protocol",
                "tcp",
                "--port",
                "8080",
            ][..],
            &["serve", "run"][..],
        ] {
            let out = tunnel(&home, args);
            assert!(!out.status.success(), "{args:?} must fail");
            let err = stderr(&out);
            assert!(err.contains(&legacy.display().to_string()), "{err}");
            assert!(err.contains("no new identity"), "{err}");
            assert!(
                !new_config(&home, "serve").exists(),
                "{args:?} created a substitute"
            );
        }
        std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read_to_string(&legacy).unwrap(), content);
    }

    #[test]
    fn concurrent_migrations_never_overwrite() {
        let (_tmp, home) = home();
        let legacy = legacy_config(&home, "serve");
        let content = body(LEGACY_KEY, "legacy");
        write(&legacy, &content, 0o600);

        let children: Vec<_> = (0..12)
            .map(|_| {
                Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
                    .args(["serve", "config", "show"])
                    .env_remove("IROH_TUNNEL_TEST_CONFIG_DIR")
                    .env("HOME", &home)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for child in children {
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{}", stderr(&out));
            assert_eq!(stdout(&out), content);
        }
        assert_eq!(
            std::fs::read_to_string(new_config(&home, "serve")).unwrap(),
            content
        );
        assert!(leftovers(&home.join(".config/iroh-tunnel")).is_empty());
    }

    #[test]
    fn explicit_config_skips_migration() {
        let (_tmp, home) = home();
        write(
            &legacy_config(&home, "serve"),
            &body(LEGACY_KEY, "legacy"),
            0o600,
        );
        let custom = home.join("custom.toml");
        write(&custom, &body(NEW_KEY, "custom"), 0o600);
        let out = tunnel(
            &home,
            &[
                "serve",
                "config",
                "keygen",
                "--config",
                custom.to_str().unwrap(),
            ],
        );
        assert!(out.status.success(), "{}", stderr(&out));
        assert!(!new_config(&home, "serve").exists());
    }
}
