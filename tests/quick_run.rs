//! #81: real quick-run processes and TCP fixture. The `test-utils` feature
//! disables TLS verification for the whole test binary; use only isolated
//! local relay fixtures, never secrets or public relays, and do not distribute it.
//! Production builds must not enable that feature.
//!
//! Quick flags: -n/--name, -t/--target, -l/--listen, -N/--node-id,
//! -p/--protocol (TCP default), and --relay-url.

use anyhow::{Context, Result};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn status(path: &std::path::Path, child: &mut Process) -> Result<serde_json::Value> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(data) = std::fs::read(path) {
            if let Ok(json) = serde_json::from_slice(&data) {
                return Ok(json);
            }
        }
        if let Some(exit) = child.0.try_wait()? {
            anyhow::bail!("role exited: {exit}");
        }
        if Instant::now() > deadline {
            anyhow::bail!("status timeout: {}", path.display());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn role(home: &std::path::Path, state: &std::path::Path, args: &[&str]) -> Result<Process> {
    eprintln!(
        "spawn {} {args:?}; HOME={} IROH_TUNNEL_STATE_DIR={}",
        env!("CARGO_BIN_EXE_iroh-tunnel"),
        home.display(),
        state.display()
    );
    Ok(Process(
        Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
            .args(args)
            .env("HOME", home)
            .env("IROH_TUNNEL_STATE_DIR", state)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quick_tcp_forwards_bytes_to_fixed_target() -> Result<()> {
    tcp_roundtrip(false, false, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quick_short_flags_forward_bytes_to_fixed_target() -> Result<()> {
    tcp_roundtrip(true, false, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quick_ipv6_target_forwards_bytes() -> Result<()> {
    tcp_roundtrip(false, true, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quick_ipv6_listener_forwards_bytes() -> Result<()> {
    tcp_roundtrip(true, false, true).await
}

async fn tcp_roundtrip(short: bool, ipv6_target: bool, ipv6_listen: bool) -> Result<()> {
    if ipv6_target || ipv6_listen {
        match TcpListener::bind("[::1]:0").await {
            Ok(probe) => drop(probe),
            Err(error) => {
                eprintln!("SKIP IPv6 roundtrip: loopback unavailable: {error}");
                return Ok(());
            }
        }
    }
    let temp = tempfile::tempdir()?;
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let relay = relay.to_string();
    let target = TcpListener::bind(if ipv6_target {
        "[::1]:0"
    } else {
        "127.0.0.1:0"
    })
    .await?;
    let target_addr = target.local_addr()?.to_string();
    let fixture = tokio::spawn(async move {
        let (mut stream, _) = target.accept().await?;
        let mut request = [0u8; 7];
        stream.read_exact(&mut request).await?;
        anyhow::ensure!(&request == b"request", "wrong target request");
        stream.write_all(b"response").await?;
        Ok::<_, anyhow::Error>(())
    });
    let listener = TcpListener::bind(if ipv6_listen {
        "[::1]:0"
    } else {
        "127.0.0.1:0"
    })
    .await?;
    let listen = listener.local_addr()?;
    drop(listener);
    let serve_args = if short {
        vec![
            "serve",
            "-n",
            "echo",
            "-t",
            &target_addr,
            "-p",
            "tcp",
            "--relay-url",
            &relay,
        ]
    } else {
        vec![
            "serve",
            "--name",
            "echo",
            "--target",
            &target_addr,
            "--relay-url",
            &relay,
        ]
    };
    let mut serve = role(temp.path(), temp.path(), &serve_args)?;
    let serve_status = status(&temp.path().join("serve-status.json"), &mut serve).await?;
    let node = serve_status["node_id"].as_str().context("serve node id")?;
    let listen_string = listen.to_string();
    let access_args = if short {
        vec![
            "access",
            "-n",
            "echo",
            "-l",
            &listen_string,
            "-N",
            node,
            "-p",
            "tcp",
            "--relay-url",
            &relay,
        ]
    } else {
        vec![
            "access",
            "--name",
            "echo",
            "--listen",
            &listen_string,
            "--node-id",
            node,
            "--relay-url",
            &relay,
        ]
    };
    let mut access = role(temp.path(), temp.path(), &access_args)?;
    status(&temp.path().join("access-status.json"), &mut access).await?;
    let mut client = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(client) = TcpStream::connect(listen).await {
                break client;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await?;
    client.write_all(b"request").await?;
    let mut response = [0u8; 8];
    tokio::time::timeout(Duration::from_secs(30), client.read_exact(&mut response)).await??;
    assert_eq!(&response, b"response");
    fixture.await??;
    // Status describes observed paths, not a forced-relay assertion.
    tokio::time::sleep(Duration::from_secs(6)).await;
    for role_name in ["serve", "access"] {
        let snapshot =
            std::fs::read_to_string(temp.path().join(format!("{role_name}-status.json")))?;
        eprintln!("{role_name} status: {snapshot}");
    }
    eprintln!("roundtrip short={short} target={target_addr} listen={listen}: request -> response");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quick_roles_preserve_existing_config_and_keys_across_ephemeral_restarts() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let relay = relay.to_string();
    let preserved = existing_user_files(temp.path())?;
    let before = file_snapshot(temp.path())?;
    ephemeral_restarts(temp.path(), &relay).await?;
    assert_user_files_unchanged(&preserved)?;
    assert!(
        file_snapshot(temp.path())? == before,
        "quick restart changed user files"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quick_roles_do_not_create_config_or_keys_in_empty_home() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    ephemeral_restarts(temp.path(), &relay.to_string()).await?;
    assert_eq!(std::fs::read_dir(temp.path())?.count(), 0);
    Ok(())
}

async fn ephemeral_restarts(home: &std::path::Path, relay: &str) -> Result<()> {
    for role_name in ["serve", "access"] {
        let mut ids = Vec::new();
        for _ in 0..2 {
            let state = tempfile::tempdir()?;
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let listen = listener.local_addr()?.to_string();
            drop(listener);
            let args = if role_name == "serve" {
                vec![
                    "serve",
                    "-n",
                    "echo",
                    "-t",
                    "127.0.0.1:12345",
                    "--relay-url",
                    &relay,
                ]
            } else {
                vec![
                    "access",
                    "-n",
                    "echo",
                    "-l",
                    &listen,
                    "-N",
                    "0000000000000000000000000000000000000000000000000000000000000000",
                    "--relay-url",
                    &relay,
                ]
            };
            let mut process = role(home, state.path(), &args)?;
            let json = status(
                &state.path().join(format!("{role_name}-status.json")),
                &mut process,
            )
            .await?;
            ids.push(json["node_id"].as_str().context("node id")?.to_owned());
            drop(process);
        }
        eprintln!("{role_name} restart public identities: {ids:?}");
        assert_ne!(ids[0], ids[1], "{role_name} identity persisted");
    }
    Ok(())
}

// Synthetic fixtures only; never read operator keys. Both current and target
// default config locations are populated so quick mode cannot silently pick one.
fn existing_user_files(home: &std::path::Path) -> Result<Vec<(std::path::PathBuf, Vec<u8>)>> {
    let mut files = Vec::new();
    let key = iroh_tunnel::config::encode_secret_key(&iroh::SecretKey::from_bytes(&[42; 32]));
    for dir in [
        ".config/iroh-tunnel",
        "Library/Application Support/iroh-tunnel",
    ] {
        std::fs::create_dir_all(home.join(dir))?;
        for name in ["serve.toml", "access.toml", "identity.key"] {
            let path = home.join(dir).join(name);
            let bytes = if name.ends_with(".toml") {
                format!("# synthetic preservation fixture\n[node]\nsecret_key = {key:?}\nrelay_urls = [\"https://127.0.0.1:1\"]\n")
                    .into_bytes()
            } else {
                key.as_bytes().to_vec()
            };
            std::fs::write(&path, &bytes)?;
            files.push((path, bytes));
        }
    }
    Ok(files)
}

fn assert_user_files_unchanged(files: &[(std::path::PathBuf, Vec<u8>)]) -> Result<()> {
    for (path, bytes) in files {
        assert!(
            &std::fs::read(path)? == bytes,
            "modified {}",
            path.display()
        );
    }
    Ok(())
}

fn file_snapshot(
    root: &std::path::Path,
) -> Result<std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>> {
    fn collect(
        root: &std::path::Path,
        dir: &std::path::Path,
        files: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                collect(root, &path, files)?;
            } else {
                files.insert(path.strip_prefix(root)?.to_owned(), std::fs::read(path)?);
            }
        }
        Ok(())
    }
    let mut files = std::collections::BTreeMap::new();
    collect(root, root, &mut files)?;
    Ok(files)
}

fn rejected(args: &[&str], diagnostic: &str) -> Result<()> {
    let home = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    let files = existing_user_files(home.path())?;
    let before = file_snapshot(home.path())?;
    let config = files
        .iter()
        .find(|(path, _)| path.ends_with(format!("{}.toml", args[0])))
        .context("role fixture")?
        .0
        .to_str()
        .context("fixture path")?;
    let actual_args: Vec<_> = args
        .iter()
        .map(|arg| {
            if *arg == "existing.toml" {
                config
            } else {
                *arg
            }
        })
        .collect();
    let mut child = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
        .args(&actual_args)
        .env("HOME", home.path())
        .env("IROH_TUNNEL_STATE_DIR", state.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait()?.is_none() {
        if Instant::now() > deadline {
            child.kill()?;
            child.wait()?;
            anyhow::bail!("invalid/mixed command did not exit: {args:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{args:?}: {stderr}");
    let role_option = if args[0] == "serve" {
        "--target"
    } else {
        "--listen"
    };
    assert!(
        stderr.contains(args[0]) || stderr.contains(role_option),
        "missing role context for {args:?}: {stderr}"
    );
    assert!(stderr.contains(diagnostic), "{args:?}: {stderr}");
    assert_user_files_unchanged(&files)?;
    assert!(
        file_snapshot(home.path())? == before,
        "rejected command changed user files"
    );
    assert_eq!(std::fs::read_dir(state.path())?.count(), 0);
    eprintln!("rejected {args:?}, exit=2: {stderr}");
    Ok(())
}

#[test]
fn mixed_quick_flags_and_subcommands_are_rejected() -> Result<()> {
    for role_name in ["serve", "access"] {
        let mut flags = vec![
            ("--name", "echo"),
            ("-n", "echo"),
            ("--protocol", "tcp"),
            ("-p", "tcp"),
            ("--relay-url", "https://localhost:1"),
        ];
        if role_name == "serve" {
            flags.extend([("--target", "127.0.0.1:12345"), ("-t", "127.0.0.1:12345")]);
        } else {
            flags.extend([
                ("--listen", "127.0.0.1:12346"),
                ("-l", "127.0.0.1:12346"),
                ("--node-id", "invalid"),
                ("-N", "invalid"),
            ]);
        }
        for (flag, value) in flags {
            // Side-effect-free configured command succeeds before the fix.
            rejected(
                &[role_name, flag, value, "config", "path"],
                "cannot be used with",
            )?;
            rejected(
                &[role_name, flag, value, "run", "--config", "existing.toml"],
                "cannot be used with",
            )?;
        }
    }
    Ok(())
}

#[test]
fn invalid_quick_inputs_fail_without_config_or_key_writes() -> Result<()> {
    let peer = "0000000000000000000000000000000000000000000000000000000000000000";
    let reserved = std::net::TcpListener::bind("127.0.0.1:0")?;
    let listen = reserved.local_addr()?;
    let address = listen.to_string();
    drop(reserved);
    for role_name in ["serve", "access"] {
        let address_flag = if role_name == "serve" {
            "--target"
        } else {
            "--listen"
        };
        let mut valid = vec![role_name, "--name", "echo", address_flag, &address];
        if role_name == "access" {
            valid.extend(["--node-id", peer]);
        }
        for (value, diagnostic) in [
            ("not-an-address", address_flag),
            ("127.0.0.1:65536", address_flag),
            ("127.0.0.1:0", "port"),
        ] {
            let mut args = valid.clone();
            args[4] = value;
            rejected(&args, diagnostic)?;
        }
        let mut args = valid.clone();
        args[2] = "bad name";
        rejected(&args, "name")?;
        let mut args = valid.clone();
        args.extend(["--protocol", "udp"]);
        rejected(&args, "tcp")?;
        let mut args = valid.clone();
        args.extend(["--relay-url", "http://localhost:1"]);
        rejected(&args, "https://")?;
        let mut args = valid.clone();
        args.extend(["--unknown-option", "x"]);
        rejected(&args, "--unknown-option")?;
        rejected(&[role_name], address_flag)?;
        let mut args = valid.clone();
        args.drain(1..3);
        rejected(&args, "--name")?;
        if role_name == "access" {
            let mut args = valid.clone();
            args[6] = "invalid";
            rejected(&args, "node-id")?;
            rejected(&valid[..5], "--node-id")?;
        }
        assert!(
            std::net::TcpStream::connect(listen).is_err(),
            "invalid quick input left listener"
        );
    }
    Ok(())
}

#[test]
fn quick_config_errors_use_existing_exit_classification() -> Result<()> {
    rejected(&["serve", "-p", "udp"], "tcp")?;
    rejected(&["access", "-p", "udp"], "tcp")?;
    Ok(())
}

#[test]
fn configured_subcommands_allow_implicit_tcp_default() -> Result<()> {
    for role_name in ["serve", "access"] {
        let home = tempfile::tempdir()?;
        let output = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
            .args([role_name, "config", "path"])
            .env("HOME", home.path())
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(role_name));
    }
    Ok(())
}

#[test]
fn quick_operator_contract_explains_target_protection() -> Result<()> {
    let readme = include_str!("../README.md");
    assert!(readme.contains("NodeId is public identity, not target authorization"));
    assert!(readme.contains("Target services must enforce their own access protection"));
    let output = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
        .args(["access", "--help"])
        .output()?;
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("public identity, not authorization"));
    Ok(())
}
