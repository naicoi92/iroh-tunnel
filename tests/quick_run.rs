//! #81: real quick-run processes and TCP fixture. The `test-utils` feature
//! disables TLS verification for the whole test binary; use only isolated
//! local relay fixtures, never secrets or public relays, and do not distribute it.
//! Production builds must not enable that feature.
//!
//! Quick flags: -n/--name, -t/--target, -l/--listen, -N/--node-id,
//! -p/--protocol (TCP default), and --relay-url.

mod common;

use anyhow::{Context, Result};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

type ReapedChildren =
    std::sync::Arc<std::sync::Mutex<Vec<(u32, std::io::Result<std::process::ExitStatus>)>>>;

struct Process(Child, Option<ReapedChildren>);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let result = self.0.wait();
        if let Some(reaped) = &self.1 {
            reaped
                .lock()
                .expect("reap observer poisoned")
                .push((self.0.id(), result));
        }
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

fn isolated(command: &mut Command, home: &std::path::Path) {
    command
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("IROH_TUNNEL_TEST_CONFIG_DIR", home)
        .env("RUST_LOG", "iroh_tunnel=info");
}

fn role(home: &std::path::Path, state: &std::path::Path, args: &[&str]) -> Result<Process> {
    eprintln!(
        "spawn {} {args:?}; HOME={} IROH_TUNNEL_STATE_DIR={}",
        env!("CARGO_BIN_EXE_iroh-tunnel"),
        home.display(),
        state.display()
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"));
    isolated(&mut command, home);
    Ok(Process(
        command
            .args(args)
            .env("IROH_TUNNEL_STATE_DIR", state)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
        None,
    ))
}

// Status and the endpoint-ready message precede bind. Only the listener's
// address-bearing log confirms that this process owns the selected port.
async fn access_ready(
    home: &std::path::Path,
    state: &std::path::Path,
    short: bool,
    ipv6: bool,
    node: &str,
    relay: &str,
) -> Result<(Process, std::net::SocketAddr)> {
    access_ready_with_collision_probe(home, state, short, ipv6, node, relay, None).await
}

struct CollisionProbe {
    first_address: std::net::SocketAddr,
    reaped: ReapedChildren,
}

async fn access_ready_with_collision_probe(
    home: &std::path::Path,
    state: &std::path::Path,
    short: bool,
    ipv6: bool,
    node: &str,
    relay: &str,
    probe: Option<&mut CollisionProbe>,
) -> Result<(Process, std::net::SocketAddr)> {
    for attempt in 0..8 {
        let injected = probe.as_ref().filter(|_| attempt == 0);
        let listen = if let Some(probe) = injected {
            probe.first_address
        } else {
            let reservation =
                TcpListener::bind(if ipv6 { "[::1]:0" } else { "127.0.0.1:0" }).await?;
            let listen = reservation.local_addr()?;
            drop(reservation);
            listen
        };
        let address = listen.to_string();
        let log_path = state.join(format!("access-listener-{attempt}.log"));
        let log = std::fs::File::create(&log_path)?;
        let mut command = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"));
        isolated(&mut command, home);
        command
            .env("IROH_TUNNEL_STATE_DIR", state)
            .stdout(log)
            .stderr(Stdio::null());
        if short {
            command.args([
                "access",
                "-n",
                "echo",
                "-l",
                &address,
                "-N",
                node,
                "-p",
                "tcp",
                "--relay-url",
                relay,
            ]);
        } else {
            command.args([
                "access",
                "--name",
                "echo",
                "--listen",
                &address,
                "--node-id",
                node,
                "--relay-url",
                relay,
            ]);
        }
        let mut process = Process(
            command.spawn()?,
            probe.as_ref().map(|probe| probe.reaped.clone()),
        );
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let log = std::fs::read_to_string(&log_path)?;
            if log.lines().any(|line| {
                line.contains("listening for local clients")
                    && line.contains("listen_addr")
                    && line.contains(&format!("{}:{}", listen.ip(), listen.port()))
            }) {
                return Ok((process, listen));
            }
            if let Some(line) = log.lines().find(|line| line.contains("failed to bind")) {
                // OS error numbers are stable even when error text is localized.
                let in_use = if cfg!(windows) {
                    "os error 10048"
                } else if cfg!(target_os = "macos") {
                    "os error 48"
                } else {
                    "os error 98"
                };
                anyhow::ensure!(line.contains(in_use), "access bind failed: {line}");
                break;
            }
            if let Some(exit) = process.0.try_wait()? {
                anyhow::bail!("access exited before listener ready: {exit}; {log}");
            }
            anyhow::ensure!(Instant::now() < deadline, "access listener timeout: {log}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        drop(process);
    }
    anyhow::bail!("access listener address in use after 8 attempts")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn access_ready_retries_occupied_port_and_reaps_first_child() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let occupied = TcpListener::bind("127.0.0.1:0").await?;
    let mut probe = CollisionProbe {
        first_address: occupied.local_addr()?,
        reaped: Default::default(),
    };
    let (_access, listen) = access_ready_with_collision_probe(
        temp.path(),
        temp.path(),
        false,
        false,
        "0000000000000000000000000000000000000000000000000000000000000000",
        &relay.to_string(),
        Some(&mut probe),
    )
    .await?;
    assert_ne!(listen, probe.first_address);
    {
        let reaped = probe.reaped.lock().expect("reap observer poisoned");
        assert_eq!(reaped.len(), 1, "expected one failed child");
        assert!(
            reaped[0].1.is_ok(),
            "failed to wait for first child: {reaped:?}"
        );
        let first_log = std::fs::read_to_string(temp.path().join("access-listener-0.log"))?;
        assert!(first_log.contains("failed to bind"), "{first_log}");
        eprintln!(
            "occupied={} retry={listen} reaped={:?}; first log: {first_log}",
            probe.first_address, *reaped
        );
    }
    TcpStream::connect(listen).await?;
    Ok(())
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
async fn quick_long_and_short_flags_reach_same_fixed_target() -> Result<()> {
    let target = TcpListener::bind("127.0.0.1:0").await?;
    let target_addr = target.local_addr()?.to_string();
    let fixture = async {
        for (expected, reply) in [
            (b"long request".as_slice(), b"long response".as_slice()),
            (b"short request".as_slice(), b"short response".as_slice()),
        ] {
            let (mut stream, _) = target.accept().await?;
            let mut request = vec![0; expected.len()];
            stream.read_exact(&mut request).await?;
            anyhow::ensure!(request == expected, "wrong target request");
            stream.write_all(reply).await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    let sessions = async {
        tcp_session(
            false,
            false,
            &target_addr,
            b"long request",
            b"long response",
        )
        .await?;
        tcp_session(
            true,
            false,
            &target_addr,
            b"short request",
            b"short response",
        )
        .await?;
        Ok::<_, anyhow::Error>(())
    };
    tokio::time::timeout(Duration::from_secs(180), async {
        tokio::try_join!(fixture, sessions)?;
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    Ok(())
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
    let target = TcpListener::bind(if ipv6_target {
        "[::1]:0"
    } else {
        "127.0.0.1:0"
    })
    .await?;
    let target_addr = target.local_addr()?.to_string();
    let fixture = async {
        let (mut stream, _) = target.accept().await?;
        let mut request = [0u8; 7];
        stream.read_exact(&mut request).await?;
        anyhow::ensure!(&request == b"request", "wrong target request");
        stream.write_all(b"response").await?;
        Ok::<_, anyhow::Error>(())
    };
    tokio::time::timeout(Duration::from_secs(90), async {
        tokio::try_join!(
            fixture,
            tcp_session(short, ipv6_listen, &target_addr, b"request", b"response")
        )?;
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    Ok(())
}

async fn tcp_session(
    short: bool,
    ipv6_listen: bool,
    target_addr: &str,
    payload: &[u8],
    expected_response: &[u8],
) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let relay = relay.to_string();
    let serve_args = if short {
        vec![
            "serve",
            "-n",
            "echo",
            "-t",
            target_addr,
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
            target_addr,
            "--relay-url",
            &relay,
        ]
    };
    let mut serve = role(temp.path(), temp.path(), &serve_args)?;
    let serve_status = status(&temp.path().join("serve-status.json"), &mut serve).await?;
    let node = serve_status["node_id"].as_str().context("serve node id")?;
    let (_access, listen) =
        access_ready(temp.path(), temp.path(), short, ipv6_listen, node, &relay).await?;
    let mut client = TcpStream::connect(listen).await?;
    client.write_all(payload).await?;
    let mut response = vec![0u8; expected_response.len()];
    tokio::time::timeout(Duration::from_secs(30), client.read_exact(&mut response)).await??;
    assert_eq!(response, expected_response);
    // Status describes observed paths, not a forced-relay assertion.
    tokio::time::sleep(Duration::from_secs(6)).await;
    for role_name in ["serve", "access"] {
        let snapshot =
            std::fs::read_to_string(temp.path().join(format!("{role_name}-status.json")))?;
        eprintln!("{role_name} status: {snapshot}");
    }
    eprintln!(
        "roundtrip short={short} target={target_addr} listen={listen}: exact request and response"
    );
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
    // Only role-ownership runtime state may appear; no config, no keys.
    assert_eq!(file_snapshot(temp.path())?, Default::default());
    Ok(())
}

async fn ephemeral_restarts(home: &std::path::Path, relay: &str) -> Result<()> {
    for role_name in ["serve", "access"] {
        let mut ids = Vec::new();
        for _ in 0..2 {
            let state = tempfile::tempdir()?;
            let mut process = if role_name == "serve" {
                role(
                    home,
                    state.path(),
                    &[
                        "serve",
                        "-n",
                        "echo",
                        "-t",
                        "127.0.0.1:12345",
                        "--relay-url",
                        relay,
                    ],
                )?
            } else {
                access_ready(
                    home,
                    state.path(),
                    true,
                    false,
                    "0000000000000000000000000000000000000000000000000000000000000000",
                    relay,
                )
                .await?
                .0
            };
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

// Synthetic fixtures only; seed the actual test-feature default config path.
fn existing_user_files(home: &std::path::Path) -> Result<Vec<(std::path::PathBuf, Vec<u8>)>> {
    let mut files = Vec::new();
    let key = iroh_tunnel::config::encode_secret_key(&iroh::SecretKey::from_bytes(&[42; 32]));
    let dir = home.join("iroh-tunnel");
    std::fs::create_dir_all(&dir)?;
    for name in ["serve.toml", "access.toml", "identity.key"] {
        let path = dir.join(name);
        let bytes = if name.ends_with(".toml") {
            format!("# synthetic preservation fixture\n[node]\nsecret_key = {key:?}\nrelay_urls = [\"https://127.0.0.1:1\"]\n").into_bytes()
        } else {
            key.as_bytes().to_vec()
        };
        std::fs::write(&path, &bytes)?;
        files.push((path, bytes));
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
            } else if !common::is_ownership_artifact(&path) {
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
    let mut command = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"));
    isolated(&mut command, home.path());
    let mut child = command
        .args(&actual_args)
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
        let mut command = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"));
        isolated(&mut command, home.path());
        let output = command.args([role_name, "config", "path"]).output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            home.path()
                .join("iroh-tunnel")
                .join(format!("{role_name}.toml"))
                .to_str()
                .unwrap()
        );
    }
    Ok(())
}

#[test]
fn test_config_root_rejects_relative_paths() -> Result<()> {
    let home = tempfile::tempdir()?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"));
    isolated(&mut command, home.path());
    let output = command
        .env("IROH_TUNNEL_TEST_CONFIG_DIR", "relative")
        .args(["serve", "config", "path"])
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must be absolute"));
    Ok(())
}

#[test]
fn quick_operator_contract_explains_target_protection() -> Result<()> {
    let readme = include_str!("../README.md");
    assert!(readme.contains("NodeId is public identity, not target authorization"));
    assert!(readme.contains("Target services must enforce their own access protection"));
    let home = tempfile::tempdir()?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"));
    isolated(&mut command, home.path());
    let output = command.args(["access", "--help"]).output()?;
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("public identity, not authorization"));
    Ok(())
}

// TCP-02 / #86: the first target socket remains live while the second target
// dial is refused. Distinct markers and a rebound listener expose replay.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn target_dial_failure_closes_only_affected_channel_without_replay() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(90), async {
        let temp = tempfile::tempdir()?;
        let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
        let target = TcpListener::bind("127.0.0.1:0").await?;
        let target_addr = target.local_addr()?;
        let relay = relay.to_string();
        let serve_log = temp.path().join("serve.log");
        let mut command = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"));
        isolated(&mut command, temp.path());
        let mut serve = Process(
            command.env("NO_COLOR", "1").env("RUST_LOG", "iroh_tunnel=debug")
                .env("IROH_TUNNEL_STATE_DIR", temp.path())
                .args(["serve", "-n", "echo", "-t", &target_addr.to_string(), "--relay-url", &relay])
                .stdout(std::fs::File::create(&serve_log)?)
                .stderr(Stdio::null()).spawn()?,
            None,
        );
        let serve_status = status(&temp.path().join("serve-status.json"), &mut serve).await?;
        let node = serve_status["node_id"].as_str().context("serve node id")?;
        let (_access, listen) = access_ready(temp.path(), temp.path(), false, false, node, &relay).await?;
        let mut healthy = TcpStream::connect(listen).await?;
        healthy.write_all(b"healthy-first").await?;
        let (mut target_healthy, _) = target.accept().await?;
        let mut first = [0; 13];
        target_healthy.read_exact(&mut first).await?;
        assert_eq!(&first, b"healthy-first");
        target_healthy.write_all(b"healthy-reply").await?;
        let mut reply = [0; 13];
        healthy.read_exact(&mut reply).await?;
        assert_eq!(&reply, b"healthy-reply");

        // Existing accepted socket survives closing the listening socket.
        drop(target);
        let mut failed = TcpStream::connect(listen).await?;
        failed.write_all(b"failed-marker").await?;
        let mut byte = [0; 1];
        let closed = tokio::time::timeout(Duration::from_secs(15), failed.read(&mut byte)).await?;
        assert!(matches!(closed, Ok(0) | Err(_)), "failed channel remained readable: {closed:?}");
        let target = TcpListener::bind(target_addr).await?;
        healthy.write_all(b"healthy-after").await?;
        let mut after = [0; 13];
        target_healthy.read_exact(&mut after).await?;
        assert_eq!(&after, b"healthy-after");
        target_healthy.write_all(b"after-reply!!").await?;
        healthy.read_exact(&mut reply).await?;
        assert_eq!(&reply, b"after-reply!!");
        assert!(tokio::time::timeout(Duration::from_secs(2), target.accept()).await.is_err(), "unexpected target connection/replay");
        let mut new_client = TcpStream::connect(listen).await?;
        new_client.write_all(b"new-request").await?;
        let (mut new_target, _) = target.accept().await?;
        let mut new_request = [0; 11];
        new_target.read_exact(&mut new_request).await?;
        assert_eq!(&new_request, b"new-request");
        new_target.write_all(b"new-response").await?;
        let mut new_response = [0; 12];
        new_client.read_exact(&mut new_response).await?;
        assert_eq!(&new_response, b"new-response");
        assert!(tokio::time::timeout(Duration::from_secs(2), target.accept()).await.is_err(), "unexpected extra target connection/replay");
        let log = std::fs::read_to_string(&serve_log)?;
        assert_eq!(log.matches("peer connected via").count(), 1, "{log}");
        assert!(log.contains("stream=2") && log.contains("stream=3") && log.contains("failed to connect local service"), "{log}");
        eprintln!("target failure isolated; failed channel closed; healthy exact markers survived; no new target connection; serve log: {log}");
        Ok::<_, anyhow::Error>(())
    }).await.context("target failure isolation timeout")??;
    Ok(())
}

async fn accept_target_channel(
    client: &mut TcpStream,
    target: &TcpListener,
    marker: &[u8],
) -> Result<TcpStream> {
    // QUIC opens the stream lazily: send bytes before waiting for target accept.
    client.write_all(marker).await?;
    let (mut socket, _) = target.accept().await?;
    let mut received = vec![0; marker.len()];
    socket.read_exact(&mut received).await?;
    assert_eq!(received, marker, "target channel marker differs");
    Ok(socket)
}

async fn assert_tcp_exchange(
    client: &mut TcpStream,
    target: &mut TcpStream,
    request: &[u8],
    response: &[u8],
) -> Result<()> {
    client.write_all(request).await?;
    let mut received = vec![0; request.len()];
    target.read_exact(&mut received).await?;
    assert_eq!(received, request, "target request differs");
    target.write_all(response).await?;
    let mut received = vec![0; response.len()];
    client.read_exact(&mut received).await?;
    assert_eq!(received, response, "client response differs");
    Ok(())
}

// TCP-02 / #86: reset an established target socket while another channel
// remains live. Unlike dial failure, both channels first exchange exact bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn established_target_reset_closes_only_affected_channel_without_replay() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(90), async {
        let temp = tempfile::tempdir()?;
        let (_map, relay, server) = iroh::test_utils::run_relay_server().await?;
        let target = TcpListener::bind("127.0.0.1:0").await?;
        let target_addr = target.local_addr()?.to_string();
        let relay = relay.to_string();
        let serve_log = temp.path().join("serve-reset.log");
        let mut command = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"));
        isolated(&mut command, temp.path());
        let mut serve = Process(
            command
                .env("NO_COLOR", "1")
                .env("RUST_LOG", "iroh_tunnel=debug")
                .env("IROH_TUNNEL_STATE_DIR", temp.path())
                .args(["serve", "-n", "echo", "-t", &target_addr, "--relay-url", &relay])
                .stdout(std::fs::File::create(&serve_log)?)
                .stderr(Stdio::null())
                .spawn()?,
            None,
        );
        let serve_status = status(&temp.path().join("serve-status.json"), &mut serve).await?;
        let node = serve_status["node_id"].as_str().context("serve node id")?;
        let (mut access, listen) =
            access_ready(temp.path(), temp.path(), false, false, node, &relay).await?;
        let outcome = tokio::time::timeout(Duration::from_secs(45), async {
            let mut healthy = TcpStream::connect(listen).await?;
            let mut target_healthy =
                accept_target_channel(&mut healthy, &target, b"healthy-open").await?;
            assert_tcp_exchange(
                &mut healthy, &mut target_healthy, b"healthy-before-reset", b"healthy-before-reply",
            ).await?;
            let mut affected = TcpStream::connect(listen).await?;
            let mut target_affected =
                accept_target_channel(&mut affected, &target, b"affected-open").await?;
            assert_tcp_exchange(
                &mut affected, &mut target_affected, b"affected-before-reset", b"affected-before-reply",
            ).await?;
            // Read final marker before reset so buffered-data loss cannot hide replay.
            affected.write_all(b"affected-final-marker").await?;
            let mut final_marker = [0; 21];
            target_affected.read_exact(&mut final_marker).await?;
            assert_eq!(&final_marker, b"affected-final-marker");
            target_affected.set_zero_linger()?;
            assert_eq!(target_affected.linger()?, Some(Duration::ZERO));
            drop(target_affected);
            let mut byte = [0; 1];
            let closed = tokio::time::timeout(Duration::from_secs(15), affected.read(&mut byte))
                .await.context("affected client did not terminate after target reset")?;
            assert!(matches!(closed, Ok(0) | Err(_)), "unexpected affected data: {closed:?}");
            assert_tcp_exchange(
                &mut healthy, &mut target_healthy, b"healthy-after-reset", b"healthy-after-reply",
            ).await?;
            let mut new_client = TcpStream::connect(listen).await?;
            let mut new_target =
                accept_target_channel(&mut new_client, &target, b"new-open").await?;
            assert_tcp_exchange(
                &mut new_client, &mut new_target, b"new-after-reset", b"new-after-reply",
            ).await?;
            // A finite observation window cannot prove absence forever. It catches
            // immediate replay/reconnect and trailing bytes on every survivor.
            let mut healthy_target_byte = [0; 1];
            let mut healthy_client_byte = [0; 1];
            let mut new_target_byte = [0; 1];
            let mut new_client_byte = [0; 1];
            let quiet = tokio::time::timeout(Duration::from_secs(2), async {
                tokio::select! {
                    result = target.accept() => format!("unexpected target accept/replay: {result:?}"),
                    result = target_healthy.read(&mut healthy_target_byte) => format!("unsolicited healthy target data/closure: {result:?}"),
                    result = healthy.read(&mut healthy_client_byte) => format!("unsolicited healthy client data/closure: {result:?}"),
                    result = new_target.read(&mut new_target_byte) => format!("unsolicited new target data/closure: {result:?}"),
                    result = new_client.read(&mut new_client_byte) => format!("unsolicited new client data/closure: {result:?}"),
                }
            }).await;
            assert!(quiet.is_err(), "{}", quiet.unwrap());
            let log = std::fs::read_to_string(&serve_log)?;
            assert_eq!(log.matches("peer connected via").count(), 1, "{log}");
            eprintln!("established target reset: affected closed ({closed:?}); healthy/new exact exchanges; 2s no observable replay; one peer connection; serve log: {log}");
            Ok::<_, anyhow::Error>(())
        }).await.context("established target reset scenario timeout");

        // Fixture termination, not graceful application shutdown acceptance.
        for process in [&mut access, &mut serve] {
            process.0.kill().context("kill fixture role")?;
            tokio::time::timeout(Duration::from_secs(5), async {
                while process.0.try_wait()?.is_none() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Ok::<_, anyhow::Error>(())
            }).await.context("fixture role reap timeout")??;
            eprintln!("fixture role {} killed and reaped within 5s", process.0.id());
        }
        tokio::time::timeout(Duration::from_secs(5), server.shutdown())
            .await.context("local relay shutdown timeout")??;
        eprintln!("local relay shutdown within 5s");
        outcome??;
        Ok::<_, anyhow::Error>(())
    }).await.context("established target reset test timeout")??;
    Ok(())
}
