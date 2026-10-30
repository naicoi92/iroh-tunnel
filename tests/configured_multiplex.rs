//! #83 / CFG-RUN-01 / CFG-MULTIPLEX-01: configured processes, real TCP
//! clients/target, isolated local relay. `test-utils` disables certificate
//! verification: never use this artifact with public relays or real secrets.

use anyhow::{Context, Result};
use iroh_tunnel::config::{encode_secret_key, AccessConfig};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct RoleProcess {
    child: Child,
    log: PathBuf,
}

impl Drop for RoleProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        self.child.wait().expect("reap configured role");
    }
}

fn command(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"));
    command
        .env("HOME", home)
        .env("IROH_TUNNEL_TEST_CONFIG_DIR", home)
        .env("IROH_TUNNEL_STATE_DIR", home.join("state"))
        .env("RUST_LOG", "iroh_tunnel=debug")
        .env("NO_COLOR", "1");
    command
}

fn start(role: &str, home: &Path, config: &Path, round: usize) -> Result<RoleProcess> {
    let log = home.join(format!("{role}-{round}.log"));
    eprintln!(
        "spawn {} {role} run --config {}; log={}",
        env!("CARGO_BIN_EXE_iroh-tunnel"),
        config.display(),
        log.display()
    );
    let child = command(home)
        .args([role, "run", "--config"])
        .arg(config)
        .stdout(std::fs::File::create(&log)?)
        .stderr(Stdio::null())
        .spawn()?;
    Ok(RoleProcess { child, log })
}

async fn wait_log(process: &mut RoleProcess, expected: &str) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let log = std::fs::read_to_string(&process.log)?;
        if log.contains(expected) {
            return Ok(log);
        }
        anyhow::ensure!(
            !log.contains("failed to bind"),
            "listener bind failed: {log}"
        );
        anyhow::ensure!(process.child.try_wait()?.is_none(), "role exited: {log}");
        anyhow::ensure!(Instant::now() < deadline, "role readiness timeout: {log}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn exchange(listen: std::net::SocketAddr, target: &TcpListener) -> Result<()> {
    // Keep both clients and target sockets open: successful sequential channels
    // alone would not establish independent overlapping streams.
    let clients = async {
        let mut first = TcpStream::connect(listen).await?;
        first.write_all(b"first request").await?;
        let mut second = TcpStream::connect(listen).await?;
        second.write_all(b"other request").await?;
        let mut reply = [0; 14];
        first.read_exact(&mut reply).await?;
        anyhow::ensure!(&reply == b"first response", "first response mismatch");
        second.read_exact(&mut reply).await?;
        anyhow::ensure!(&reply == b"other response", "second response mismatch");
        // First channel remains usable after second has been serviced.
        first.write_all(b"still open").await?;
        let mut reply = [0; 10];
        first.read_exact(&mut reply).await?;
        anyhow::ensure!(&reply == b"still open", "first channel lost independence");
        Ok::<_, anyhow::Error>(())
    };
    let fixture = async {
        let (mut first, _) = target.accept().await?;
        let (mut second, _) = target.accept().await?;
        let mut first_request = [0; 13];
        let mut second_request = [0; 13];
        tokio::try_join!(
            first.read_exact(&mut first_request),
            second.read_exact(&mut second_request)
        )?;
        // Independent QUIC handshakes may finish out of accept order.
        if &first_request == b"other request" {
            std::mem::swap(&mut first, &mut second);
            std::mem::swap(&mut first_request, &mut second_request);
        }
        anyhow::ensure!(
            &first_request == b"first request",
            "first target request mismatch"
        );
        anyhow::ensure!(
            &second_request == b"other request",
            "second target request mismatch"
        );
        second.write_all(b"other response").await?;
        first.write_all(b"first response").await?;
        let mut request = [0; 10];
        first.read_exact(&mut request).await?;
        anyhow::ensure!(&request == b"still open", "target follow-up mismatch");
        first.write_all(&request).await?;
        Ok::<_, anyhow::Error>(())
    };
    tokio::time::timeout(Duration::from_secs(30), async {
        tokio::try_join!(clients, fixture)?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("configured TCP exchange timeout")??;
    Ok(())
}

async fn configured_variant(legacy: &str) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let serve_home = temp.path().join("serve");
    let access_home = temp.path().join("access");
    std::fs::create_dir_all(&serve_home)?;
    std::fs::create_dir_all(access_home.join("iroh-tunnel"))?;
    let serve_path = serve_home.join("serve.toml");
    let access_path = access_home.join("iroh-tunnel/access.toml");
    let serve_key = iroh::SecretKey::generate();
    let access_key = iroh::SecretKey::generate();
    let (_map, relay, _relay_server) = iroh::test_utils::run_relay_server().await?;
    let target = TcpListener::bind("127.0.0.1:0").await?;
    std::fs::write(&serve_path, format!(
        "[node]\nsecret_key = {:?}\nrelay_urls = [{:?}]\n\n[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = {}\n",
        encode_secret_key(&serve_key), relay.to_string(), target.local_addr()?.port()
    ))?;
    let mut access_source = String::new();
    for round in 0..2 {
        let mut serve = start("serve", &serve_home, &serve_path, round)?;
        let log = wait_log(&mut serve, "endpoint ready").await?;
        assert!(log.contains(&serve_key.public().to_string()));
        // Retry only address-in-use races; do not connect a readiness probe,
        // which would itself create an extra tunnel stream/connection.
        let (mut access, listen) = {
            let mut ready = None;
            for attempt in 0..8 {
                let reservation = TcpListener::bind("127.0.0.1:0").await?;
                let listen = reservation.local_addr()?;
                drop(reservation);
                if round == 0 {
                    access_source = format!(
                        "[node]\nsecret_key = {:?}\nrelay_urls = [{:?}]\n\n[[services]]\nname = \"echo\"\nnode_id = {:?}\nprotocol = \"tcp\"\nport = {}\n{legacy}\n",
                        encode_secret_key(&access_key), relay.to_string(), serve_key.public().to_string(), listen.port()
                    );
                    std::fs::write(&access_path, &access_source)?;
                } else {
                    let mut cfg = AccessConfig::load(&access_path)?;
                    cfg.services[0].port = listen.port();
                    cfg.save(&access_path)?;
                }
                let mut access = start("access", &access_home, &access_path, round * 8 + attempt)?;
                match wait_log(
                    &mut access,
                    &format!("listen_addr=127.0.0.1:{}", listen.port()),
                )
                .await
                {
                    Ok(_) => {
                        ready = Some((access, listen));
                        break;
                    }
                    Err(error) => {
                        let log = std::fs::read_to_string(&access.log)?;
                        if !["os error 48", "os error 98", "os error 10048"]
                            .iter()
                            .any(|code| log.contains(code))
                        {
                            return Err(error);
                        }
                    }
                }
            }
            ready.context("configured listener collision retries exhausted")?
        };
        let log = wait_log(&mut access, "listening for local clients").await?;
        assert!(log.contains(&access_key.public().to_string()));
        exchange(listen, &target).await?;
        let log = wait_log(&mut serve, "accepted stream on connection").await?;
        assert_eq!(
            log.matches("peer connected via").count(),
            1,
            "legacy={legacy:?}, round={round}: expected one reused service connection; {log}"
        );
        assert!(
            log.contains("stream=2"),
            "second stream not observed: {log}"
        );
        let access_log = std::fs::read_to_string(&access.log)?;
        assert!(!access_log.contains("per-channel"));
        assert!(
            !access_log
                .lines()
                .any(|line| line.contains("WARN") && line.contains("multiplex")),
            "legacy warning: {access_log}"
        );
        eprintln!("variant={legacy:?} round={round} peer={} target={} listener={listen}: one connection, streams 1+2, exact bidirectional payloads; connection log: {}", serve_key.public(), target.local_addr()?, log.lines().find(|line| line.contains("peer connected via")).unwrap());
        drop(access);
        drop(serve);
        if round == 0 {
            assert!(
                std::fs::read_to_string(&access_path)? == access_source,
                "loading changed legacy file"
            );
            let output = command(&access_home)
                .args([
                    "access",
                    "config",
                    "add",
                    "--name",
                    "saved",
                    "--protocol",
                    "tcp",
                    "--port",
                    "5433",
                    "--node-id",
                    &serve_key.public().to_string(),
                ])
                .output()?;
            anyhow::ensure!(
                output.status.success(),
                "config save failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = command(&access_home)
                .args(["access", "config", "remove", "--name", "saved"])
                .output()?;
            anyhow::ensure!(output.status.success(), "config remove failed");
            let saved = std::fs::read_to_string(&access_path)?;
            assert!(!saved.contains("multiplex"));
            let cfg = AccessConfig::load(&access_path)?;
            assert!(cfg.node.secret_key == encode_secret_key(&access_key));
            assert_eq!(cfg.services.len(), 1);
            assert_eq!(cfg.services[0].name, "echo");
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configured_false_reuses_connection_and_forwards_after_save_and_restart() -> Result<()> {
    configured_variant("multiplex = false").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configured_true_reuses_connection_and_forwards_after_save_and_restart() -> Result<()> {
    configured_variant("multiplex = true").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configured_omitted_reuses_connection_and_forwards_after_save_and_restart() -> Result<()> {
    configured_variant("").await
}

// TCP-02 / #86: peer loss closes live channels without replay after recovery.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configured_peer_loss_closes_active_channels_without_replay() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let serve_home = temp.path().join("serve");
    let access_home = temp.path().join("access");
    std::fs::create_dir_all(&serve_home)?;
    std::fs::create_dir_all(&access_home)?;
    let serve_path = serve_home.join("serve.toml");
    let access_path = access_home.join("access.toml");
    let serve_key = iroh::SecretKey::generate();
    let access_key = iroh::SecretKey::generate();
    let (_map, relay, server) = iroh::test_utils::run_relay_server().await?;
    let target = TcpListener::bind("127.0.0.1:0").await?;
    let reservation = TcpListener::bind("127.0.0.1:0").await?;
    let listen = reservation.local_addr()?;
    drop(reservation);
    std::fs::write(&serve_path, format!(
        "[node]\nsecret_key = {:?}\nrelay_urls = [{:?}]\n\n[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = {}\n",
        encode_secret_key(&serve_key), relay.to_string(), target.local_addr()?.port()
    ))?;
    std::fs::write(&access_path, format!(
        "[node]\nsecret_key = {:?}\nrelay_urls = [{:?}]\n\n[[services]]\nname = \"echo\"\nnode_id = {:?}\nprotocol = \"tcp\"\nport = {}\n",
        encode_secret_key(&access_key), relay.to_string(), serve_key.public().to_string(), listen.port()
    ))?;
    let mut serve = start("serve", &serve_home, &serve_path, 0)?;
    let mut access = start("access", &access_home, &access_path, 0)?;
    let mut restarted = None;
    let outcome = tokio::time::timeout(Duration::from_secs(100), async {
        let log = wait_log(&mut serve, "endpoint ready").await?;
        assert!(log.contains(&serve_key.public().to_string()));
        wait_log(&mut access, &format!("listen_addr=127.0.0.1:{}", listen.port())).await?;
        let mut first = TcpStream::connect(listen).await?;
        first.write_all(b"old-first-marker").await?;
        let (mut first_target, _) = target.accept().await?;
        let mut marker = [0; 16];
        first_target.read_exact(&mut marker).await?;
        assert_eq!(&marker, b"old-first-marker");
        first_target.write_all(b"first-reply").await?;
        let mut reply = [0; 11];
        first.read_exact(&mut reply).await?;
        assert_eq!(&reply, b"first-reply");
        let mut second = TcpStream::connect(listen).await?;
        second.write_all(b"old-second-marker").await?;
        let (mut second_target, _) = target.accept().await?;
        let mut marker = [0; 17];
        second_target.read_exact(&mut marker).await?;
        assert_eq!(&marker, b"old-second-marker");
        second_target.write_all(b"second-reply").await?;
        let mut reply = [0; 12];
        second.read_exact(&mut reply).await?;
        assert_eq!(&reply, b"second-reply");
        let log = wait_log(&mut serve, "stream=2").await?;
        assert_eq!(log.matches("peer connected via").count(), 1, "{log}");
        anyhow::ensure!(access.child.try_wait()?.is_none(), "Access exited before peer loss");
        anyhow::ensure!(serve.child.try_wait()?.is_none(), "Serve exited before peer loss");
        serve.child.kill().context("kill active Serve")?;
        tokio::time::timeout(Duration::from_secs(5), async {
            while serve.child.try_wait()?.is_none() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok::<_, anyhow::Error>(())
        }).await.context("reap killed Serve timeout")??;
        // Access and target stay alive; only Serve loss can close these channels.
        for client in [&mut first, &mut second] {
            let mut byte = [0; 1];
            let result = tokio::time::timeout(Duration::from_secs(35), client.read(&mut byte))
                .await.context("client did not close after Serve loss")?;
            assert!(matches!(result, Ok(0) | Err(_)), "unexpected old client data: {result:?}");
        }
        for socket in [&mut first_target, &mut second_target] {
            let mut byte = [0; 1];
            let result = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut byte))
                .await.context("old target socket did not close after Serve loss")?;
            assert!(matches!(result, Ok(0) | Err(_)), "unexpected old target bytes: {result:?}");
        }
        anyhow::ensure!(access.child.try_wait()?.is_none(), "Access exited after peer loss");
        let mut next = start("serve", &serve_home, &serve_path, 1)?;
        let log = wait_log(&mut next, "endpoint ready").await?;
        assert!(log.contains(&serve_key.public().to_string()), "restarted Serve identity differs");
        restarted = Some(next);
        let mut client = TcpStream::connect(listen).await?;
        client.write_all(b"new-client-marker").await?;
        let (mut new_target, _) = target.accept().await?;
        let mut marker = [0; 17];
        new_target.read_exact(&mut marker).await?;
        assert_eq!(&marker, b"new-client-marker", "old bytes replayed to recovered target");
        new_target.write_all(b"new-reply").await?;
        let mut reply = [0; 9];
        client.read_exact(&mut reply).await?;
        assert_eq!(&reply, b"new-reply");
        let mut byte = [0; 1];
        let quiet = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                result = target.accept() => format!("unexpected target connection: {result:?}"),
                result = new_target.read(&mut byte) => format!("unexpected recovered target bytes/closure: {result:?}"),
            }
        }).await;
        assert!(quiet.is_err(), "{}", quiet.unwrap());
        eprintln!("two active channels closed after killed Serve; same identity recovered, exact new exchange, 2s no target replay");
        Ok::<_, anyhow::Error>(())
    }).await.context("peer loss scenario timeout");
    for process in [&mut access, &mut serve]
        .into_iter()
        .chain(restarted.iter_mut())
    {
        if process.child.try_wait()?.is_none() {
            process.child.kill().context("kill fixture role")?;
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while process.child.try_wait()?.is_none() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("fixture reap timeout")??;
    }
    tokio::time::timeout(Duration::from_secs(5), server.shutdown())
        .await
        .context("relay shutdown timeout")??;
    outcome??;
    Ok(())
}

// TCP-02 / #86: an unavailable peer cannot leave a new client waiting forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unavailable_peer_retries_then_closes_client_within_deadline() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("access");
    std::fs::create_dir_all(&home)?;
    let key = iroh::SecretKey::generate();
    let peer = iroh::SecretKey::generate();
    let (_map, relay, server) = iroh::test_utils::run_relay_server().await?;
    let reservation = TcpListener::bind("127.0.0.1:0").await?;
    let listen = reservation.local_addr()?;
    drop(reservation);
    let config = home.join("access.toml");
    std::fs::write(&config, format!(
        "[node]\nsecret_key = {:?}\nrelay_urls = [{:?}]\n\n[[services]]\nname = \"echo\"\nnode_id = {:?}\nprotocol = \"tcp\"\nport = {}\n",
        encode_secret_key(&key), relay.to_string(), peer.public().to_string(), listen.port()
    ))?;
    let mut access = start("access", &home, &config, 0)?;
    let outcome = async {
        wait_log(&mut access, "listening for local clients").await?;
        let started = Instant::now();
        let mut client = TcpStream::connect(listen).await?;
        client.write_all(b"pending").await?;
        let mut byte = [0; 1];
        let result = tokio::time::timeout(Duration::from_secs(40), client.read(&mut byte))
            .await
            .context("unavailable peer left client waiting beyond 40s")?;
        anyhow::ensure!(
            matches!(result, Ok(0) | Err(_)),
            "unexpected client data: {result:?}"
        );
        let log = std::fs::read_to_string(&access.log)?;
        anyhow::ensure!(
            log.contains("connect attempt 1")
                && log.contains("retrying in 1000ms")
                && log.contains("connect attempt 2")
                && log.contains("retrying in 2000ms"),
            "repeated exponential backoff not observed: {log}"
        );
        eprintln!(
            "unavailable client closed after {:?}; retry log: {log}",
            started.elapsed()
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if access.child.try_wait()?.is_none() {
        access.child.kill()?;
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while access.child.try_wait()?.is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("fixture reap timeout")??;
    tokio::time::timeout(Duration::from_secs(5), server.shutdown())
        .await
        .context("relay shutdown timeout")??;
    outcome
}

async fn retry_scenario(scenario: &str) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let serve_home = temp.path().join("serve");
    let access_home = temp.path().join("access");
    std::fs::create_dir_all(&serve_home)?;
    std::fs::create_dir_all(&access_home)?;
    let serve_key = iroh::SecretKey::generate();
    let access_key = iroh::SecretKey::generate();
    let (_map, relay, server) = iroh::test_utils::run_relay_server().await?;
    let target = TcpListener::bind("127.0.0.1:0").await?;
    let reservation = TcpListener::bind("127.0.0.1:0").await?;
    let listen = reservation.local_addr()?;
    drop(reservation);
    let serve_path = serve_home.join("serve.toml");
    let access_path = access_home.join("access.toml");
    std::fs::write(&serve_path, format!("[node]\nsecret_key = {:?}\nrelay_urls = [{:?}]\n\n[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = {}\n", encode_secret_key(&serve_key), relay.to_string(), target.local_addr()?.port()))?;
    std::fs::write(&access_path, format!("[node]\nsecret_key = {:?}\nrelay_urls = [{:?}]\n\n[[services]]\nname = \"echo\"\nnode_id = {:?}\nprotocol = \"tcp\"\nport = {}\n", encode_secret_key(&access_key), relay.to_string(), serve_key.public().to_string(), listen.port()))?;
    let mut access = start("access", &access_home, &access_path, 0)?;
    let mut serve = None;
    let outcome = tokio::time::timeout(Duration::from_secs(65), async {
        wait_log(&mut access, "listening for local clients").await?;
        if scenario == "healthy" {
            let mut process = start("serve", &serve_home, &serve_path, 0)?;
            wait_log(&mut process, "endpoint ready").await?;
            serve = Some(process);
        }
        let started = Instant::now();
        let mut first = TcpStream::connect(listen).await?;
        first.write_all(b"first").await?;
        match scenario {
            "concurrent" => {
                tokio::time::sleep(Duration::from_secs(10)).await;
                let second_started = Instant::now();
                let mut second = TcpStream::connect(listen).await?;
                second.write_all(b"second").await?;
                let mut byte = [0; 1];
                let result = tokio::time::timeout(Duration::from_secs(25), first.read(&mut byte))
                    .await
                    .context("first client did not close")?;
                anyhow::ensure!(
                    matches!(result, Ok(0) | Err(_)),
                    "first returned data: {result:?}"
                );
                let first_elapsed = started.elapsed();
                let result = tokio::time::timeout(Duration::from_secs(15), second.read(&mut byte))
                    .await
                    .context("second client did not close")?;
                anyhow::ensure!(
                    matches!(result, Ok(0) | Err(_)),
                    "second returned data: {result:?}"
                );
                let second_elapsed = second_started.elapsed();
                anyhow::ensure!(
                    first_elapsed < Duration::from_secs(35)
                        && second_elapsed < Duration::from_secs(35),
                    "budgets exceeded: {first_elapsed:?}, {second_elapsed:?}"
                );
                anyhow::ensure!(
                    second_elapsed >= Duration::from_secs(25),
                    "second expired prematurely: {second_elapsed:?}"
                );
                eprintln!("staggered client durations: {first_elapsed:?}, {second_elapsed:?}");
            }
            "recovery" | "healthy" => {
                if scenario == "recovery" {
                    wait_log(&mut access, "connect attempt 1").await?;
                    let mut process = start("serve", &serve_home, &serve_path, 0)?;
                    wait_log(&mut process, "endpoint ready").await?;
                    serve = Some(process);
                }
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(25), target.accept())
                        .await
                        .context("target accept timeout")??;
                let mut request = [0; 5];
                socket.read_exact(&mut request).await?;
                anyhow::ensure!(&request == b"first", "bad request");
                socket.write_all(b"reply").await?;
                let mut reply = [0; 5];
                first.read_exact(&mut reply).await?;
                anyhow::ensure!(&reply == b"reply", "bad reply");
                eprintln!("{scenario} first exchange after {:?}", started.elapsed());
                if scenario == "healthy" {
                    tokio::time::sleep(Duration::from_secs(32)).await;
                    first.write_all(b"later").await?;
                    tokio::time::timeout(Duration::from_secs(5), socket.read_exact(&mut request))
                        .await??;
                    anyhow::ensure!(&request == b"later", "bad later request");
                    socket.write_all(b"again").await?;
                    tokio::time::timeout(Duration::from_secs(5), first.read_exact(&mut reply))
                        .await??;
                    anyhow::ensure!(&reply == b"again", "bad later reply");
                }
            }
            #[cfg(unix)]
            "shutdown" => {
                wait_log(&mut access, "connect attempt 1").await?;
                let status = Command::new("kill")
                    .args(["-TERM", &access.child.id().to_string()])
                    .status()?;
                anyhow::ensure!(status.success(), "SIGTERM failed");
                let mut byte = [0; 1];
                let result = tokio::time::timeout(Duration::from_secs(10), first.read(&mut byte))
                    .await
                    .context("shutdown did not close pending client")?;
                anyhow::ensure!(
                    matches!(result, Ok(0) | Err(_)),
                    "shutdown returned data: {result:?}"
                );
                wait_log(&mut access, "client connection cancelled on shutdown").await?;
                eprintln!(
                    "pending client closed on shutdown after {:?}",
                    started.elapsed()
                );
            }
            _ => anyhow::bail!("unknown retry scenario"),
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("retry scenario timeout");
    for process in std::iter::once(&mut access).chain(serve.iter_mut()) {
        if process.child.try_wait()?.is_none() {
            process.child.kill()?;
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while process.child.try_wait()?.is_none() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("fixture reap timeout")??;
    }
    tokio::time::timeout(Duration::from_secs(5), server.shutdown())
        .await
        .context("relay shutdown timeout")??;
    outcome??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn staggered_unavailable_clients_have_independent_deadlines() -> Result<()> {
    retry_scenario("concurrent").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn peer_recovery_connects_waiting_client_before_deadline() -> Result<()> {
    retry_scenario("recovery").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn established_client_remains_usable_past_connect_deadline() -> Result<()> {
    retry_scenario("healthy").await
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_cancels_pending_connection() -> Result<()> {
    retry_scenario("shutdown").await
}
