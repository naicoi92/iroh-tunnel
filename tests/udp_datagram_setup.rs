//! #152: real `iroh-tunnel` processes prove that a UDP service connection is
//! closed at setup when the peer cannot carry QUIC datagrams, on both sides,
//! without a redial storm and without hurting other services.
//!
//! The `test-utils` feature disables relay TLS verification for this test
//! binary and enables the `IROH_TUNNEL_TEST_DISABLE_DATAGRAMS` seam: use only
//! the isolated in-process relay, never real secrets or public relays.
//!
//! Cleanup stops roles with SIGTERM on Unix and asserts a successful exit.
//! On Windows `Child::kill` is used for cleanup only and graceful exit is not
//! asserted (documented limitation; graceful shutdown is covered by the
//! existing in-process tests).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use iroh::SecretKey;
use iroh_tunnel_host::config::encode_secret_key;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

const SEAM: &str = "IROH_TUNNEL_TEST_DISABLE_DATAGRAMS";
const MILESTONE_TIMEOUT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);
const SEND_EVERY: Duration = Duration::from_millis(200);
/// `STATUS_FLUSH_INTERVAL` (5 s) plus margin; the file is rewritten on change only.
const STATUS_WAIT: Duration = Duration::from_secs(7);

const SERVE_FAIL_ON_ACCESS: &str = "udp datagrams unsupported by serve peer";
const ACCESS_CLOSED_BY_SERVE: &str = "serve peer closed the udp connection: datagrams unsupported";
const ACCESS_FAIL_ON_SERVE: &str = "udp datagrams unsupported by access peer";
const SERVE_CLOSED_BY_ACCESS: &str = "access peer closed the udp connection: datagrams unsupported";
const BACKOFF_WARNING: &str = "dialing resumes in";
const CONNECTED: &str = "connected to serve peer";
const CONNECT_ATTEMPT: &str = "connect attempt";

/// A running role; `stop` ends it as an operator would.
struct Role {
    child: Child,
    log: PathBuf,
    state: PathBuf,
}

impl Role {
    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn stop(mut self) {
        #[cfg(unix)]
        // SAFETY: kill(2) on our own live child pid.
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        let _ = self.child.kill();
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                break Some(status);
            }
            if Instant::now() > deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        #[cfg(unix)]
        assert!(
            status.is_some_and(|s| s.success()),
            "role did not exit successfully on SIGTERM: {status:?}\n{}",
            self.log()
        );
        #[cfg(not(unix))]
        let _ = status;
    }
}

impl Drop for Role {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Service<'a> {
    name: &'a str,
    protocol: &'a str,
    /// Serve: target address. Access: listen address.
    addr: SocketAddr,
    /// Access only: the Serve node this service dials.
    node_id: Option<String>,
}

fn write_config(
    dir: &Path,
    file: &str,
    key: &SecretKey,
    relay: &str,
    services: &[Service<'_>],
) -> Result<PathBuf> {
    let mut text = format!(
        "[node]\nsecret_key = \"{}\"\nrelay_urls = [\"{relay}\"]\n",
        encode_secret_key(key)
    );
    for s in services {
        text.push_str(&format!(
            "\n[[services]]\nname = \"{}\"\nprotocol = \"{}\"\nhost = \"{}\"\nport = {}\n",
            s.name,
            s.protocol,
            s.addr.ip(),
            s.addr.port()
        ));
        if let Some(id) = &s.node_id {
            text.push_str(&format!("node_id = \"{id}\"\n"));
        }
    }
    let path = dir.join(file);
    std::fs::write(&path, text)?;
    Ok(path)
}

fn spawn(
    root: &Path,
    name: &str,
    role: &str,
    config: &Path,
    disable_datagrams: bool,
) -> Result<Role> {
    let home = root.join(format!("{name}-home"));
    let state = root.join(format!("{name}-state"));
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&state)?;
    let log = root.join(format!("{name}.log"));
    eprintln!("spawn {name}: {role} run --config {}", config.display());
    let mut command = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"));
    command
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("IROH_TUNNEL_TEST_CONFIG_DIR", &home)
        .env("IROH_TUNNEL_STATE_DIR", &state)
        .env("RUST_LOG", "iroh_tunnel=info")
        .env("NO_COLOR", "1")
        .env_remove(SEAM);
    if disable_datagrams {
        command.env(SEAM, "1");
    }
    let child = command
        .args([role, "run", "--config"])
        .arg(config)
        .stdout(std::fs::File::create(&log)?)
        .stderr(Stdio::null())
        .spawn()?;
    Ok(Role { child, log, state })
}

/// Poll `check` every 100 ms until it yields a value, the role exits, or 30 s pass.
async fn wait_for<T>(
    what: &str,
    role: &mut Role,
    mut check: impl FnMut(&Role) -> Option<T>,
) -> Result<T> {
    let deadline = Instant::now() + MILESTONE_TIMEOUT;
    loop {
        if let Some(value) = check(role) {
            return Ok(value);
        }
        anyhow::ensure!(
            role.running(),
            "role exited while waiting for {what}\n{}",
            role.log()
        );
        anyhow::ensure!(
            Instant::now() < deadline,
            "timeout waiting for {what}\n{}",
            role.log()
        );
        tokio::time::sleep(POLL).await;
    }
}

async fn serve_ready(role: &mut Role, key: &SecretKey) -> Result<()> {
    let expected = key.public().to_string();
    wait_for("serve status node_id", role, |r| {
        let body = std::fs::read_to_string(r.state.join("serve-status.json")).ok()?;
        let json: serde_json::Value = serde_json::from_str(&body).ok()?;
        (json["node_id"] == expected.as_str()).then_some(())
    })
    .await
}

/// Wait for the Access UDP listener(s); returns the addresses in log order.
async fn udp_listeners_ready(role: &mut Role, count: usize) -> Result<()> {
    wait_for("access udp listener", role, |r| {
        let log = r.log();
        (log.lines()
            .filter(|l| l.contains("listening for local udp clients"))
            .count()
            >= count)
            .then_some(())
    })
    .await
}

fn error_line(log: &str, text: &str, service: &str, peer: &SecretKey) -> Option<usize> {
    let peer = peer.public().to_string();
    log.lines().position(|l| {
        l.contains(text) && l.contains("ERROR") && l.contains(service) && l.contains(&peer)
    })
}

/// A free local UDP/TCP-agnostic port: reserve, release, let the role rebind.
async fn free_udp_addr() -> Result<SocketAddr> {
    let probe = UdpSocket::bind("127.0.0.1:0").await?;
    Ok(probe.local_addr()?)
}

/// A UDP echo target that counts the datagrams it receives.
async fn udp_target() -> Result<(SocketAddr, Arc<AtomicUsize>)> {
    let socket = UdpSocket::bind("127.0.0.1:0").await?;
    let addr = socket.local_addr()?;
    let received = Arc::new(AtomicUsize::new(0));
    let counter = received.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65535];
        while let Ok((n, from)) = socket.recv_from(&mut buf).await {
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = socket.send_to(&buf[..n], from).await;
        }
    });
    Ok((addr, received))
}

/// A TCP echo target (one connection at a time is enough here).
async fn tcp_echo_target() -> Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 || stream.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    Ok(addr)
}

/// Keep sending to `to` for `span`.
async fn send_for(client: &UdpSocket, to: SocketAddr, span: Duration) -> Result<()> {
    let end = Instant::now() + span;
    while Instant::now() < end {
        client.send_to(b"ping", to).await?;
        tokio::time::sleep(SEND_EVERY).await;
    }
    Ok(())
}

fn count(log: &str, text: &str) -> usize {
    log.lines().filter(|l| l.contains(text)).count()
}

/// Backoff and redial assertions shared by Steps 1 and 2.
fn assert_single_attempt(access_log: &str, failure_line: usize) {
    assert_eq!(count(access_log, CONNECTED), 1, "{access_log}");
    assert_eq!(count(access_log, BACKOFF_WARNING), 1, "{access_log}");
    let after: Vec<&str> = access_log.lines().skip(failure_line + 1).collect();
    assert!(
        after.iter().all(|l| !l.contains(CONNECT_ATTEMPT)),
        "redial after failure: {after:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_without_datagrams_fails_access_setup() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(90), async {
        let root = tempfile::tempdir()?;
        let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
        let relay = relay.to_string();
        let (target, received) = udp_target().await?;
        let (serve_key, access_key) = (SecretKey::generate(), SecretKey::generate());
        let listen = free_udp_addr().await?;

        let serve_cfg = write_config(
            root.path(),
            "serve.toml",
            &serve_key,
            &relay,
            &[Service {
                name: "dns",
                protocol: "udp",
                addr: target,
                node_id: None,
            }],
        )?;
        let mut serve = spawn(root.path(), "serve", "serve", &serve_cfg, true)?;
        serve_ready(&mut serve, &serve_key).await?;
        let access_cfg = write_config(
            root.path(),
            "access.toml",
            &access_key,
            &relay,
            &[Service {
                name: "dns",
                protocol: "udp",
                addr: listen,
                node_id: Some(serve_key.public().to_string()),
            }],
        )?;
        let mut access = spawn(root.path(), "access", "access", &access_cfg, false)?;
        udp_listeners_ready(&mut access, 1).await?;

        let client = UdpSocket::bind("127.0.0.1:0").await?;
        let (mut access_line, mut serve_line) = (None, None);
        let deadline = Instant::now() + MILESTONE_TIMEOUT;
        while access_line.is_none() || serve_line.is_none() {
            anyhow::ensure!(
                Instant::now() < deadline,
                "failure lines missing\nA:{}\nS:{}",
                access.log(),
                serve.log()
            );
            client.send_to(b"ping", listen).await?;
            tokio::time::sleep(SEND_EVERY).await;
            access_line = access_line
                .or_else(|| error_line(&access.log(), SERVE_FAIL_ON_ACCESS, "dns", &serve_key));
            serve_line = serve_line
                .or_else(|| error_line(&serve.log(), SERVE_CLOSED_BY_ACCESS, "dns", &access_key));
        }
        send_for(&client, listen, Duration::from_secs(5)).await?;

        assert_eq!(
            received.load(Ordering::SeqCst),
            0,
            "target must see nothing"
        );
        assert!(access.running() && serve.running(), "a role exited");
        let log = access.log();
        assert_single_attempt(
            &log,
            error_line(&log, SERVE_FAIL_ON_ACCESS, "dns", &serve_key).unwrap(),
        );
        access.stop();
        serve.stop();
        Ok(())
    })
    .await?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn access_without_datagrams_is_closed_by_serve() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(90), async {
        let root = tempfile::tempdir()?;
        let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
        let relay = relay.to_string();
        let (udp, received) = udp_target().await?;
        let tcp = tcp_echo_target().await?;
        let (serve_key, access_key, web_key) = (
            SecretKey::generate(),
            SecretKey::generate(),
            SecretKey::generate(),
        );
        let (dns_listen, web_listen) = (free_udp_addr().await?, free_tcp_addr().await?);

        let serve_cfg = write_config(
            root.path(),
            "serve.toml",
            &serve_key,
            &relay,
            &[
                Service {
                    name: "dns",
                    protocol: "udp",
                    addr: udp,
                    node_id: None,
                },
                Service {
                    name: "web",
                    protocol: "tcp",
                    addr: tcp,
                    node_id: None,
                },
            ],
        )?;
        let mut serve = spawn(root.path(), "serve", "serve", &serve_cfg, false)?;
        serve_ready(&mut serve, &serve_key).await?;
        let id = serve_key.public().to_string();
        let access_cfg = write_config(
            root.path(),
            "access.toml",
            &access_key,
            &relay,
            &[Service {
                name: "dns",
                protocol: "udp",
                addr: dns_listen,
                node_id: Some(id.clone()),
            }],
        )?;
        let mut access = spawn(root.path(), "access", "access", &access_cfg, true)?;
        udp_listeners_ready(&mut access, 1).await?;
        let web_cfg = write_config(
            root.path(),
            "web.toml",
            &web_key,
            &relay,
            &[Service {
                name: "web",
                protocol: "tcp",
                addr: web_listen,
                node_id: Some(id),
            }],
        )?;
        let mut web = spawn(root.path(), "web", "access", &web_cfg, false)?;
        wait_for("web listener", &mut web, |r| {
            r.log()
                .contains("listening for local clients")
                .then_some(())
        })
        .await?;

        let client = UdpSocket::bind("127.0.0.1:0").await?;
        let (mut access_line, mut serve_line) = (None, None);
        let deadline = Instant::now() + MILESTONE_TIMEOUT;
        while access_line.is_none() || serve_line.is_none() {
            anyhow::ensure!(
                Instant::now() < deadline,
                "failure lines missing\nA:{}\nS:{}",
                access.log(),
                serve.log()
            );
            client.send_to(b"ping", dns_listen).await?;
            tokio::time::sleep(SEND_EVERY).await;
            serve_line = serve_line
                .or_else(|| error_line(&serve.log(), ACCESS_FAIL_ON_SERVE, "dns", &access_key));
            access_line = access_line
                .or_else(|| error_line(&access.log(), ACCESS_CLOSED_BY_SERVE, "dns", &serve_key));
        }
        send_for(&client, dns_listen, Duration::from_secs(5)).await?;

        // TCP on the same Serve is unaffected.
        let mut stream =
            tokio::time::timeout(Duration::from_secs(30), TcpStream::connect(web_listen))
                .await
                .context("web connect timeout")??;
        stream.write_all(b"hello").await?;
        let mut reply = [0u8; 5];
        tokio::time::timeout(Duration::from_secs(30), stream.read_exact(&mut reply))
            .await
            .context("web round trip timeout")??;
        assert_eq!(&reply, b"hello");

        assert_eq!(
            received.load(Ordering::SeqCst),
            0,
            "target must see nothing"
        );
        assert!(
            access.running() && serve.running() && web.running(),
            "a role exited"
        );
        let log = access.log();
        assert_single_attempt(
            &log,
            error_line(&log, ACCESS_CLOSED_BY_SERVE, "dns", &serve_key).unwrap(),
        );
        web.stop();
        access.stop();
        serve.stop();
        Ok(())
    })
    .await?
}

async fn free_tcp_addr() -> Result<SocketAddr> {
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    Ok(probe.local_addr()?)
}

/// One echo round trip through `listen`, retrying while the tunnel connects.
async fn round_trip(client: &UdpSocket, listen: SocketAddr, payload: &[u8]) -> Result<()> {
    let deadline = Instant::now() + MILESTONE_TIMEOUT;
    let mut buf = vec![0u8; 65535];
    while Instant::now() < deadline {
        client.send_to(payload, listen).await?;
        let wait = tokio::time::timeout(Duration::from_millis(500), async {
            loop {
                let (n, _) = client.recv_from(&mut buf).await?;
                if &buf[..n] == payload {
                    return Ok::<_, std::io::Error>(());
                }
            }
        })
        .await;
        if matches!(wait, Ok(Ok(()))) {
            return Ok(());
        }
    }
    anyhow::bail!("no echo for {payload:?} via {listen}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_failing_udp_service_does_not_stop_another() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(90), async {
        let root = tempfile::tempdir()?;
        let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
        let relay = relay.to_string();
        let (bad_target, bad_received) = udp_target().await?;
        let (good_target, _) = udp_target().await?;
        let (a_key, b_key, access_key) = (
            SecretKey::generate(),
            SecretKey::generate(),
            SecretKey::generate(),
        );
        let (bad_listen, good_listen) = (free_udp_addr().await?, free_udp_addr().await?);

        let a_cfg = write_config(
            root.path(),
            "a.toml",
            &a_key,
            &relay,
            &[Service {
                name: "bad",
                protocol: "udp",
                addr: bad_target,
                node_id: None,
            }],
        )?;
        let b_cfg = write_config(
            root.path(),
            "b.toml",
            &b_key,
            &relay,
            &[Service {
                name: "good",
                protocol: "udp",
                addr: good_target,
                node_id: None,
            }],
        )?;
        let mut serve_a = spawn(root.path(), "serve-a", "serve", &a_cfg, true)?;
        let mut serve_b = spawn(root.path(), "serve-b", "serve", &b_cfg, false)?;
        serve_ready(&mut serve_a, &a_key).await?;
        serve_ready(&mut serve_b, &b_key).await?;
        let access_cfg = write_config(
            root.path(),
            "access.toml",
            &access_key,
            &relay,
            &[
                Service {
                    name: "bad",
                    protocol: "udp",
                    addr: bad_listen,
                    node_id: Some(a_key.public().to_string()),
                },
                Service {
                    name: "good",
                    protocol: "udp",
                    addr: good_listen,
                    node_id: Some(b_key.public().to_string()),
                },
            ],
        )?;
        let mut access = spawn(root.path(), "access", "access", &access_cfg, false)?;
        udp_listeners_ready(&mut access, 2).await?;

        let client = UdpSocket::bind("127.0.0.1:0").await?;
        let warned = |log: &str| log.contains(BACKOFF_WARNING);
        let mut seen = None;
        let deadline = Instant::now() + MILESTONE_TIMEOUT;
        while seen.is_none() {
            anyhow::ensure!(
                Instant::now() < deadline,
                "setup failure missing\n{}",
                access.log()
            );
            client.send_to(b"ping", bad_listen).await?;
            tokio::time::sleep(SEND_EVERY).await;
            let log = access.log();
            seen = error_line(&log, SERVE_FAIL_ON_ACCESS, "bad", &a_key).filter(|_| warned(&log));
        }

        // Keep hitting `bad` while `good` round-trips.
        let noise = {
            let socket = UdpSocket::bind("127.0.0.1:0").await?;
            tokio::spawn(async move {
                loop {
                    let _ = socket.send_to(b"ping", bad_listen).await;
                    tokio::time::sleep(SEND_EVERY).await;
                }
            })
        };
        let result = async {
            for i in 0..3 {
                round_trip(&client, good_listen, format!("good-{i}").as_bytes()).await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        noise.abort();
        result?;

        tokio::time::sleep(STATUS_WAIT).await;
        let body = std::fs::read_to_string(access.state.join("access-status.json"))?;
        let status: serde_json::Value = serde_json::from_str(&body)?;
        let bad = status["services"]
            .as_array()
            .and_then(|s| s.iter().find(|s| s["name"] == "bad"))
            .with_context(|| format!("bad missing in {body}"))?;
        let drops = bad["udp"]["drops"]
            .as_object()
            .with_context(|| format!("no drops in {body}"))?;
        assert!(!drops.is_empty(), "{body}");
        for (reason, n) in drops {
            assert_eq!(n, 0, "drop counter {reason} of bad must stay 0: {body}");
        }
        assert_eq!(bad_received.load(Ordering::SeqCst), 0);
        assert!(access.running() && serve_a.running() && serve_b.running());
        access.stop();
        serve_a.stop();
        serve_b.stop();
        Ok(())
    })
    .await?
}
