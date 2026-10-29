//! #83 / CFG-03: legacy multiplex has no configuration effect and disappears
//! on app-owned saves. Exercise public config boundaries and real commands.

use anyhow::Result;
use iroh_tunnel::config::{encode_secret_key, AccessConfig, ServeConfig};

#[test]
fn legacy_multiplex_values_are_ignored_and_removed_on_save() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("access.toml");
    let key = encode_secret_key(&iroh::SecretKey::generate());
    let peer = iroh::SecretKey::generate().public();
    let base = format!(
        r#"[node]
secret_key = "{key}"
relay_urls = ["https://relay.example"]
relay_token = "fixture-token"
max_concurrent_streams = 8

[[services]]
name = "echo"
node_id = "{peer}"
protocol = "tcp"
host = "127.0.0.1"
port = 5432
relay_urls = ["https://override.example"]
"#
    );
    std::fs::write(&path, &base)?;
    let expected = AccessConfig::load(&path)?;

    // CFG-03 says ignore the legacy field, not validate its old boolean type.
    for legacy in [
        "",
        "multiplex = true",
        "multiplex = false",
        "multiplex = \"auto\"",
        "multiplex = 0",
        "multiplex = 1.5",
        "multiplex = 2026-10-29",
        "multiplex = [false, true]",
        "multiplex = { enabled = false }",
        "[services.multiplex]\nenabled = false",
    ] {
        std::fs::write(&path, format!("{base}{legacy}\n"))?;
        let config = AccessConfig::load(&path)?;
        assert!(config == expected, "legacy field changed config: {legacy}");
        config.save(&path)?;
        let saved: toml::Value = toml::from_str(&std::fs::read_to_string(&path)?)?;
        assert!(
            saved["services"][0].get("multiplex").is_none(),
            "save retained legacy field: {legacy}"
        );
        assert!(
            AccessConfig::load(&path)? == expected,
            "save changed config"
        );
    }
    Ok(())
}

#[test]
fn config_commands_drop_legacy_multiplex_without_changing_other_settings() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("iroh-tunnel/access.toml");
    std::fs::create_dir_all(path.parent().unwrap())?;
    let key = encode_secret_key(&iroh::SecretKey::generate());
    let peer = iroh::SecretKey::generate().public().to_string();
    let source = format!(
        r#"# operator comment
[node]
secret_key = "{key}"
relay_urls = ["https://relay.example"]
max_concurrent_streams = 8

[[services]]
name = "echo" # keep comment
node_id = "{peer}"
protocol = "tcp"
port = 5432
multiplex = "ignored"

[[services]]
name = "remove-me"
node_id = "{peer}"
protocol = "tcp"
port = 5433
multiplex = false
"#
    );
    for args in [
        vec![
            "add",
            "--name",
            "added",
            "--protocol",
            "tcp",
            "--port",
            "5434",
            "--node-id",
            &peer,
        ],
        vec!["remove", "--name", "remove-me"],
        vec!["keygen"],
    ] {
        std::fs::write(&path, &source)?;
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
            .args(["access", "config"])
            .args(&args)
            .env("IROH_TUNNEL_TEST_CONFIG_DIR", temp.path())
            .output()?;
        assert!(
            output.status.success(),
            "{} failed: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("multiplex"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("multiplex"));
        let saved = std::fs::read_to_string(&path)?;
        let value: toml::Value = toml::from_str(&saved)?;
        for service in value["services"].as_array().unwrap() {
            assert!(
                service.get("multiplex").is_none(),
                "{} retained legacy field",
                args[0]
            );
        }
        assert!(saved.contains("# operator comment"));
        assert!(saved.contains("# keep comment"));
        let config = AccessConfig::load(&path)?;
        assert_eq!(config.node.relay_urls, ["https://relay.example"]);
        assert_eq!(config.node.max_concurrent_streams, Some(8));
        assert_eq!(config.services[0].name, "echo");
        assert_eq!(config.services[0].node_id, peer);
        assert_eq!(config.services[0].port, 5432);
        assert!(if args[0] == "keygen" {
            config.node.secret_key != key
        } else {
            config.node.secret_key == key
        });
    }
    Ok(())
}

#[test]
fn keygen_drops_multiplex_from_inline_services() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("access.toml");
    let peer = iroh::SecretKey::generate().public();
    std::fs::write(&path, format!(
        "# inline services\nservices = [{{ name = \"echo\", node_id = \"{peer}\", protocol = \"tcp\", port = 5432, multiplex = false }}]\n"
    ))?;
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
        .args(["access", "config", "keygen", "--config"])
        .arg(&path)
        .output()?;
    assert!(output.status.success());
    let saved = std::fs::read_to_string(&path)?;
    assert!(!saved.contains("multiplex"));
    assert!(saved.contains("# inline services"));
    assert_eq!(AccessConfig::load(&path)?.services[0].port, 5432);
    Ok(())
}

#[test]
fn rejected_config_add_preserves_legacy_file_and_key() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("access.toml");
    let source = format!(
        "[node]\nsecret_key = {:?}\n\n[[services]]\nname = \"echo\"\nnode_id = {:?}\nprotocol = \"tcp\"\nport = 5432\nmultiplex = false\n",
        encode_secret_key(&iroh::SecretKey::generate()), iroh::SecretKey::generate().public().to_string()
    );
    std::fs::write(&path, &source)?;
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
        .args(["access", "config", "add", "--config"])
        .arg(&path)
        .args([
            "--name",
            "echo",
            "--protocol",
            "tcp",
            "--port",
            "5433",
            "--node-id",
            &iroh::SecretKey::generate().public().to_string(),
        ])
        .output()?;
    assert!(!output.status.success());
    assert!(
        std::fs::read_to_string(&path)? == source,
        "rejected mutation changed config"
    );
    Ok(())
}

#[test]
fn malformed_config_reports_parse_error_and_preserves_file() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("access.toml");
    let key = encode_secret_key(&iroh::SecretKey::generate());
    let source = format!("[node]\nsecret_key = \"{key}\"\n\n[[services]\n");
    std::fs::write(&path, &source)?;
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
        .args(["access", "config", "keygen", "--config"])
        .arg(&path)
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to parse config"));
    assert!(!stderr.contains(&key), "diagnostic exposed secret key");
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&key));
    assert!(
        std::fs::read_to_string(&path)? == source,
        "parse failure changed config"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn config_write_error_is_reported_and_readonly_file_preserved() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir()?;
    let path = temp.path().join("access.toml");
    let key = encode_secret_key(&iroh::SecretKey::generate());
    let source = format!("[node]\nsecret_key = \"{key}\"\n\n[[services]]\nname = \"echo\"\nnode_id = \"{}\"\nprotocol = \"tcp\"\nport = 5432\nmultiplex = false\n", iroh::SecretKey::generate().public());
    std::fs::write(&path, &source)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400))?;
    // Privileged processes can bypass mode bits; never claim coverage in that case.
    if std::fs::OpenOptions::new().write(true).open(&path).is_ok() {
        eprintln!("skipped: process bypasses read-only file permissions");
        return Ok(());
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
        .args(["access", "config", "keygen", "--config"])
        .arg(&path)
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to write config"));
    assert!(!stderr.contains(&key), "diagnostic exposed secret key");
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&key));
    assert!(
        std::fs::read_to_string(&path)? == source,
        "write failure changed config"
    );
    Ok(())
}

#[test]
fn generated_identity_save_drops_legacy_fields_for_both_roles() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("role.toml");
    let service = format!(
        "[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = 5432\nnode_id = {:?}\nmultiplex = false\n",
        iroh::SecretKey::generate().public().to_string()
    );
    for access in [true, false] {
        std::fs::write(&path, &service)?;
        let key = if access {
            AccessConfig::load(&path)?.resolve_and_save_key(&path)?
        } else {
            ServeConfig::load(&path)?.resolve_and_save_key(&path)?
        };
        assert!(!std::fs::read_to_string(&path)?.contains("multiplex"));
        let persisted_key = if access {
            AccessConfig::load(&path)?.node.secret_key
        } else {
            ServeConfig::load(&path)?.node.secret_key
        };
        assert!(persisted_key == encode_secret_key(&key));
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn config_edit_preserves_external_editor_output_including_multiplex() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    // edit 0.1 splits editor command on whitespace; fixed Unix temp root avoids
    // a workspace or TMPDIR with spaces changing the executable name.
    let temp = tempfile::tempdir_in("/tmp")?;
    let path = temp.path().join("iroh-tunnel/access.toml");
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, "# original\n")?;
    let editor_output = temp.path().join("editor-output.toml");
    let contents = "# editor owns this file\nservices = [{ name = \"echo\", protocol = \"tcp\", port = 5432, multiplex = false }]\n";
    std::fs::write(&editor_output, contents)?;
    let editor = temp.path().join("editor");
    std::fs::write(&editor, "#!/bin/sh\ncat \"$IROH_EDITOR_SOURCE\" > \"$1\"\n")?;
    std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o700))?;
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
        .args(["access", "config", "edit"])
        .env("IROH_TUNNEL_TEST_CONFIG_DIR", temp.path())
        .env("IROH_EDITOR_SOURCE", &editor_output)
        .env("VISUAL", &editor)
        .env("EDITOR", &editor)
        .output()?;
    assert!(
        output.status.success(),
        "editor failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_to_string(&path)?, contents);
    Ok(())
}
