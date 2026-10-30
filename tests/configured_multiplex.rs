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
        wait_log(&mut access, &format!("listen_addr={listen}")).await?;
        let started = Instant::now();
        let mut client = TcpStream::connect(listen).await?;
        client.write_all(b"pending").await?;
        // Observe completed timed-out dials as they occur, not the planned
        // delay in their log messages. Allow 500ms polling/scheduling slack.
        wait_log(&mut access, "connect attempt 1 timed out").await?;
        let first_attempt = Instant::now();
        wait_log(&mut access, "connect attempt 2 timed out").await?;
        let attempt_spacing = first_attempt.elapsed();
        anyhow::ensure!(
            attempt_spacing >= Duration::from_millis(5500),
            "timed-out attempts too close together: {attempt_spacing:?}"
        );
        let mut byte = [0; 1];
        let result = tokio::time::timeout(Duration::from_secs(40), client.read(&mut byte))
            .await
            .context("unavailable peer left client waiting beyond 40s")?;
        anyhow::ensure!(
            matches!(result, Ok(0) | Err(_)),
            "unexpected client data: {result:?}"
        );
        let elapsed = started.elapsed();
        anyhow::ensure!(
            (Duration::from_secs(28)..=Duration::from_secs(40)).contains(&elapsed),
            "client closed outside deadline tolerance: {elapsed:?}"
        );
        let log = wait_log(&mut access, "client connection deadline expired").await?;
        anyhow::ensure!(access.child.try_wait()?.is_none(), "access exited: {log}");
        eprintln!(
            "unavailable client closed after {elapsed:?}; timed-out attempt spacing {attempt_spacing:?}; retry log: {log}"
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
        wait_log(&mut access, &format!("listen_addr={listen}")).await?;
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

#[cfg(unix)]
fn terminate(process: &RoleProcess) -> Result<()> {
    let status = Command::new("kill")
        .args(["-TERM", &process.child.id().to_string()])
        .status()?;
    anyhow::ensure!(status.success(), "SIGTERM failed");
    Ok(())
}

#[cfg(unix)]
async fn reap(process: &mut RoleProcess, limit: Duration) -> Result<std::process::ExitStatus> {
    tokio::time::timeout(limit, async {
        loop {
            if let Some(status) = process.child.try_wait()? {
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("process exit timeout")?
}

#[cfg(unix)]
async fn shutdown_drain_scenario(role: &str) -> Result<()> {
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
    let mut serve = start("serve", &serve_home, &serve_path, 0)?;
    let mut access = start("access", &access_home, &access_path, 0)?;
    let mut new_peer = None;
    let outcome = tokio::time::timeout(Duration::from_secs(40), async {
        wait_log(&mut serve, "endpoint ready").await?;
        wait_log(&mut access, &format!("listen_addr={listen}")).await?;
        let mut completing = TcpStream::connect(listen).await?;
        completing.write_all(b"completing").await?;
        let (mut completing_target, _) = target.accept().await?;
        let mut marker = [0; 10];
        completing_target.read_exact(&mut marker).await?;
        anyhow::ensure!(&marker == b"completing", "wrong completing marker");
        completing_target.write_all(b"ready").await?;
        let mut reply = [0; 5];
        completing.read_exact(&mut reply).await?;
        anyhow::ensure!(&reply == b"ready", "wrong initial reply");
        let mut held = TcpStream::connect(listen).await?;
        held.write_all(b"held-open!").await?;
        let (mut held_target, _) = target.accept().await?;
        held_target.read_exact(&mut marker).await?;
        anyhow::ensure!(&marker == b"held-open!", "wrong held marker");
        held_target.write_all(b"ready").await?;
        held.read_exact(&mut reply).await?;
        anyhow::ensure!(&reply == b"ready", "wrong held reply");
        let log = wait_log(&mut serve, "stream=2").await?;
        anyhow::ensure!(log.matches("peer connected via").count() == 1, "not shared: {log}");
        let (stopping, home) = if role == "access" {
            (&mut access, &access_home)
        } else {
            (&mut serve, &serve_home)
        };
        let started = Instant::now();
        terminate(stopping)?;
        tokio::time::sleep(Duration::from_millis(250)).await;
        // Repeated stop neither resets grace nor interrupts established pipes.
        terminate(stopping)?;
        completing.shutdown().await?;
        let mut eof = [0; 1];
        anyhow::ensure!(completing_target.read(&mut eof).await? == 0, "missing request EOF");
        completing_target.write_all(b"final response inside grace").await
            .context("established target was closed before graceful completion")?;
        completing_target.shutdown().await?;
        let mut final_reply = Vec::new();
        completing.read_to_end(&mut final_reply).await?;
        anyhow::ensure!(final_reply == b"final response inside grace", "graceful response lost: {final_reply:?}");
        held.write_all(b"during grace").await?;
        let mut during = [0; 12];
        held_target.read_exact(&mut during).await?;
        anyhow::ensure!(&during == b"during grace", "held pipe did not survive stop");
        held_target.write_all(b"still alive").await?;
        let mut alive = [0; 11];
        held.read_exact(&mut alive).await?;
        anyhow::ensure!(&alive == b"still alive", "held response mismatch");
        if role == "access" {
            anyhow::ensure!(TcpStream::connect(listen).await.is_err(), "Access still accepts clients");
        } else {
            // Access retains its existing shared connection. A new stream must
            // not create a target socket while old streams are draining.
            let mut rejected = TcpStream::connect(listen).await?;
            rejected.write_all(b"must not reach target").await?;
            anyhow::ensure!(tokio::time::timeout(Duration::from_millis(100), target.accept()).await.is_err(), "Serve admitted a stream during drain");
            // Distinct Access identity also attempts a new peer handshake.
            let peer_home = temp.path().join("new-peer");
            std::fs::create_dir_all(&peer_home)?;
            let reservation = TcpListener::bind("127.0.0.1:0").await?;
            let peer_listen = reservation.local_addr()?;
            drop(reservation);
            let mut cfg = AccessConfig::load(&access_path)?;
            cfg.node.secret_key = encode_secret_key(&iroh::SecretKey::generate());
            cfg.services[0].port = peer_listen.port();
            let config = peer_home.join("access.toml");
            cfg.save(&config)?;
            new_peer = Some(start("access", &peer_home, &config, 0)?);
            wait_log(new_peer.as_mut().unwrap(), &format!("listen_addr={peer_listen}")).await?;
            let mut pending_peer = TcpStream::connect(peer_listen).await?;
            pending_peer.write_all(b"new peer must not reach target").await?;
        }
        anyhow::ensure!(home.join(format!("state/{role}-status.json")).exists(), "status removed before drain ended");
        tokio::select! {
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(started) + Duration::from_millis(1500)) => {},
            result = target.accept() => anyhow::bail!("new channel reached target during drain: {result:?}"),
        }
        terminate(stopping)?;
        let result = tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(4), held.read(&mut eof)) =>
                result.context("held channel survived drain bound")?,
            result = target.accept() => anyhow::bail!("new channel reached target during drain: {result:?}"),
        };
        anyhow::ensure!(matches!(result, Ok(0) | Err(_)), "held channel returned data: {result:?}");
        let elapsed = started.elapsed();
        anyhow::ensure!((Duration::from_secs(2)..Duration::from_secs(4)).contains(&elapsed), "held closure outside grace tolerance: {elapsed:?}");
        let result = tokio::time::timeout(Duration::from_secs(2), held_target.read(&mut eof)).await?;
        anyhow::ensure!(matches!(result, Ok(0) | Err(_)), "target remained open: {result:?}");
        let exit = reap(stopping, Duration::from_secs(3)).await?;
        anyhow::ensure!(exit.success(), "shutdown exit: {exit}");
        anyhow::ensure!(!home.join(format!("state/{role}-status.json")).exists(), "own status left behind");
        if role == "access" {
            let _rebound = TcpListener::bind(listen).await.context("listener not released")?;
        }
        eprintln!("{role}: admission stopped, exact graceful response, held channel closed at {elapsed:?}, successful exit and status cleanup");
        Ok::<_, anyhow::Error>(())
    }).await.context("shutdown scenario timeout");
    for process in [&mut access, &mut serve]
        .into_iter()
        .chain(new_peer.iter_mut())
    {
        if process.child.try_wait()?.is_none() {
            process.child.kill()?;
        }
        reap(process, Duration::from_secs(5)).await?;
        if !matches!(&outcome, Ok(Ok(()))) {
            eprintln!(
                "shutdown child log: {}",
                std::fs::read_to_string(&process.log)?
            );
        }
    }
    tokio::time::timeout(Duration::from_secs(5), server.shutdown()).await??;
    outcome??;
    Ok(())
}

// AC-S1 / TCP-02 / #86: ordered stop, bounded drain, then resource teardown.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn access_shutdown_drains_established_channels_before_forcing_remainder() -> Result<()> {
    shutdown_drain_scenario("access").await
}

// AC-S1 / TCP-02 / #86: applies to new streams on existing connections too.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_shutdown_drains_established_channels_before_forcing_remainder() -> Result<()> {
    shutdown_drain_scenario("serve").await
}

// AC-S2 / #86: pending dial/backoff and shared-dial lock wait cancel promptly.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn access_shutdown_cancels_pending_retry_without_waiting_for_grace() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let home = temp.path();
    let key = iroh::SecretKey::generate();
    let peer = iroh::SecretKey::generate();
    let (_map, relay, server) = iroh::test_utils::run_relay_server().await?;
    let outcome = tokio::time::timeout(Duration::from_secs(25), async {
        for (round, backoff) in [false, true].into_iter().enumerate() {
            let reservation = TcpListener::bind("127.0.0.1:0").await?;
            let listen = reservation.local_addr()?;
            drop(reservation);
            let config = home.join("access.toml");
            std::fs::write(&config, format!("[node]\nsecret_key = {:?}\nrelay_urls = [{:?}]\n\n[[services]]\nname = \"echo\"\nnode_id = {:?}\nprotocol = \"tcp\"\nport = {}\n", encode_secret_key(&key), relay.to_string(), peer.public().to_string(), listen.port()))?;
            let mut access = start("access", home, &config, round)?;
            let scenario = async {
                wait_log(&mut access, &format!("listen_addr={listen}")).await?;
                let mut first = TcpStream::connect(listen).await?;
                first.write_all(b"pending dial").await?;
                let mut second = TcpStream::connect(listen).await?;
                second.write_all(b"pending lock").await?;
                if backoff {
                    wait_log(&mut access, "connect attempt 1 timed out").await?;
                } else {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                let started = Instant::now();
                terminate(&access)?;
                // Immediate repeated signal while cancellation is in progress.
                terminate(&access)?;
                let mut byte_a = [0; 1];
                let mut byte_b = [0; 1];
                let (a, b) = tokio::join!(first.read(&mut byte_a), second.read(&mut byte_b));
                anyhow::ensure!(matches!(a, Ok(0) | Err(_)) && matches!(b, Ok(0) | Err(_)), "pending clients received data: {a:?}, {b:?}");
                anyhow::ensure!(started.elapsed() < Duration::from_secs(1), "pending clients consumed drain grace: {:?}", started.elapsed());
                // Iroh close waits for QUIC close retransmission on failed
                // connectivity (upstream documents about 3 s). Not pipe grace.
                let exit = reap(&mut access, Duration::from_secs(5)).await?;
                anyhow::ensure!(exit.success(), "cancellation exit: {exit}");
                anyhow::ensure!(started.elapsed() < Duration::from_secs(5), "pending shutdown exceeded close bound: {:?}", started.elapsed());
                let log = std::fs::read_to_string(&access.log)?;
                anyhow::ensure!(log.matches("client connection cancelled on shutdown").count() == 2, "pending clients not observed: {log}");
                anyhow::ensure!(!home.join("state/access-status.json").exists(), "status not cleaned");
                let _rebound = TcpListener::bind(listen).await?;
                eprintln!("backoff={backoff}: dial/lock clients cancelled, repeated stop exited at {:?}, listener/status released", started.elapsed());
                Ok::<_, anyhow::Error>(())
            }.await;
            if access.child.try_wait()?.is_none() {
                access.child.kill()?;
            }
            reap(&mut access, Duration::from_secs(5)).await?;
            if scenario.is_err() {
                eprintln!("cancellation child log: {}", std::fs::read_to_string(&access.log)?);
            }
            scenario?;
        }
        Ok::<_, anyhow::Error>(())
    }).await.context("pending shutdown scenario timeout");
    tokio::time::timeout(Duration::from_secs(5), server.shutdown()).await??;
    outcome??;
    Ok(())
}

// AC-S5 / #86: grace caps waiting; it is not an unconditional delay.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_roles_exit_without_waiting_for_drain_grace() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (_map, relay, server) = iroh::test_utils::run_relay_server().await?;
    let outcome = async {
        for role in ["serve", "access"] {
            let home = temp.path().join(role);
            std::fs::create_dir_all(&home)?;
            let reservation = TcpListener::bind("127.0.0.1:0").await?;
            let listen = reservation.local_addr()?;
            // Serve has no TCP admission listener: retain real target socket
            // so another parallel fixture cannot claim its port before rebind.
            let target = if role == "serve" { Some(reservation) } else { drop(reservation); None };
            let peer = iroh::SecretKey::generate().public();
            let reference = if role == "access" { format!("node_id = {:?}\n", peer.to_string()) } else { String::new() };
            let config = home.join(format!("{role}.toml"));
            std::fs::write(&config, format!("[node]\nsecret_key = {:?}\nrelay_urls = [{:?}]\n\n[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = {}\n{reference}", encode_secret_key(&iroh::SecretKey::generate()), relay.to_string(), listen.port()))?;
            let mut process = start(role, &home, &config, 0)?;
            let scenario = tokio::time::timeout(Duration::from_secs(35), async {
                let ready = if role == "access" { format!("listen_addr={listen}") } else { "accepting connections".to_owned() };
                wait_log(&mut process, &ready).await?;
                let started = Instant::now();
                terminate(&process)?;
                let exit = reap(&mut process, Duration::from_secs(2)).await?;
                anyhow::ensure!(exit.success(), "idle {role} exit: {exit}");
                anyhow::ensure!(!home.join(format!("state/{role}-status.json")).exists(), "idle status left behind");
                if target.is_none() {
                    let _rebound = TcpListener::bind(listen).await?;
                }
                eprintln!("idle {role} exited with resources released at {:?}", started.elapsed());
                Ok::<_, anyhow::Error>(())
            }).await.context("idle scenario timeout");
            if process.child.try_wait()?.is_none() { process.child.kill()?; }
            reap(&mut process, Duration::from_secs(5)).await?;
            if !matches!(&scenario, Ok(Ok(()))) {
                eprintln!("idle child log: {}", std::fs::read_to_string(&process.log)?);
            }
            scenario??;
        }
        Ok::<_, anyhow::Error>(())
    }.await;
    tokio::time::timeout(Duration::from_secs(5), server.shutdown()).await??;
    outcome
}

// AC-S6 / TCP-02: last pipe completion must not discard queued QUIC bytes.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn roles_deliver_last_channel_response_before_early_shutdown_exit() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (_map, relay, server) = iroh::test_utils::run_relay_server().await?;
    let outcome = tokio::time::timeout(Duration::from_secs(40), async {
        for role in ["access", "serve"] {
            let serve_home = temp.path().join(format!("{role}-serve"));
            let access_home = temp.path().join(format!("{role}-access"));
            std::fs::create_dir_all(&serve_home)?;
            std::fs::create_dir_all(&access_home)?;
            let serve_key = iroh::SecretKey::generate();
            let target = TcpListener::bind("127.0.0.1:0").await?;
            let reservation = TcpListener::bind("127.0.0.1:0").await?;
            let listen = reservation.local_addr()?;
            drop(reservation);
            let serve_path = serve_home.join("serve.toml");
            let access_path = access_home.join("access.toml");
            std::fs::write(&serve_path, format!("[node]\nsecret_key = {:?}\nrelay_urls = [{:?}]\n\n[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = {}\n", encode_secret_key(&serve_key), relay.to_string(), target.local_addr()?.port()))?;
            std::fs::write(&access_path, format!("[node]\nsecret_key = {:?}\nrelay_urls = [{:?}]\n\n[[services]]\nname = \"echo\"\nnode_id = {:?}\nprotocol = \"tcp\"\nport = {}\n", encode_secret_key(&iroh::SecretKey::generate()), relay.to_string(), serve_key.public().to_string(), listen.port()))?;
            let mut serve = start("serve", &serve_home, &serve_path, 0)?;
            let mut access = start("access", &access_home, &access_path, 0)?;
            let scenario = async {
                wait_log(&mut serve, "accepting connections").await?;
                wait_log(&mut access, &format!("listen_addr={listen}")).await?;
                let mut client = TcpStream::connect(listen).await?;
                client.write_all(b"last request").await?;
                client.shutdown().await?;
                let (mut socket, _) = target.accept().await?;
                let mut request = Vec::new();
                socket.read_to_end(&mut request).await?;
                anyhow::ensure!(request == b"last request", "last request mismatch");
                let stopping = if role == "access" { &mut access } else { &mut serve };
                let started = Instant::now();
                terminate(stopping)?;
                tokio::time::sleep(Duration::from_millis(250)).await;
                let response = vec![b'Z'; 256 * 1024];
                socket.write_all(&response).await?;
                socket.shutdown().await?;
                tokio::time::sleep(Duration::from_millis(100)).await;
                let mut received = Vec::new();
                client.read_to_end(&mut received).await?;
                anyhow::ensure!(received == response, "{role} lost last response: received {} bytes", received.len());
                let exit = reap(stopping, Duration::from_secs(3)).await?;
                anyhow::ensure!(exit.success(), "last-channel exit: {exit}");
                anyhow::ensure!(started.elapsed() < Duration::from_secs(3), "completed last channel consumed full grace");
                eprintln!("{role}: last channel delivered 256 KiB and EOF; exited at {:?}", started.elapsed());
                Ok::<_, anyhow::Error>(())
            }.await;
            for process in [&mut access, &mut serve] {
                if process.child.try_wait()?.is_none() { process.child.kill()?; }
                reap(process, Duration::from_secs(5)).await?;
                if scenario.is_err() { eprintln!("last-channel log: {}", std::fs::read_to_string(&process.log)?); }
            }
            scenario?;
        }
        Ok::<_, anyhow::Error>(())
    }).await.context("last-channel scenario timeout");
    tokio::time::timeout(Duration::from_secs(5), server.shutdown()).await??;
    outcome??;
    Ok(())
}
