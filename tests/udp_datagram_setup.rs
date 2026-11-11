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
const UDP_LISTENING: &str = "listening for local udp clients";
const TCP_LISTENING: &str = "listening for local clients";
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

/// How a role is started: quick mode for one service, `run --config` for several.
enum Launch<'a> {
    Config(&'a Path),
    Quick(Vec<String>),
}

fn quick_serve(name: &str, target: SocketAddr, relay: &str) -> Launch<'static> {
    Launch::Quick(
        [
            "serve",
            "-n",
            name,
            "-t",
            &target.to_string(),
            "-p",
            "udp",
            "--relay-url",
            relay,
        ]
        .map(String::from)
        .to_vec(),
    )
}

fn quick_access(
    name: &str,
    listen: SocketAddr,
    node: &str,
    protocol: &str,
    relay: &str,
) -> Launch<'static> {
    Launch::Quick(
        [
            "access",
            "-n",
            name,
            "-l",
            &listen.to_string(),
            "-N",
            node,
            "-p",
            protocol,
            "--relay-url",
            relay,
        ]
        .map(String::from)
        .to_vec(),
    )
}

fn spawn(
    root: &Path,
    name: &str,
    role: &str,
    launch: Launch<'_>,
    disable_datagrams: bool,
) -> Result<Role> {
    let home = root.join(format!("{name}-home"));
    let state = root.join(format!("{name}-state"));
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&state)?;
    let log = root.join(format!("{name}.log"));
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
    match &launch {
        Launch::Config(path) => command.args([role, "run", "--config"]).arg(path),
        Launch::Quick(args) => command.args(args),
    };
    eprintln!("spawn {name}: {command:?}");
    let child = command
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

/// Wait for the role's status file to carry a `node_id`; returns it.
async fn node_id(role: &mut Role, kind: &str) -> Result<String> {
    let file = format!("{kind}-status.json");
    wait_for("status node_id", role, |r| {
        let body = std::fs::read_to_string(r.state.join(&file)).ok()?;
        let json: serde_json::Value = serde_json::from_str(&body).ok()?;
        json["node_id"].as_str().map(String::from)
    })
    .await
}

/// A rendered log line: exact level token, exact `field=value` tokens and the
/// message text, all on the same line. Substrings of other fields (a peer id
/// containing "bad") can therefore never satisfy a matcher.
struct LineMatch<'a> {
    level: &'a str,
    message: &'a str,
    fields: Vec<(&'a str, &'a str)>,
}

impl LineMatch<'_> {
    fn matches(&self, line: &str) -> bool {
        line.contains(self.message)
            && line.split_whitespace().any(|t| t == self.level)
            && self.fields.iter().all(|(k, v)| {
                let want = format!("{k}={v}");
                line.split_whitespace().any(|t| t == want)
            })
    }

    fn position(&self, log: &str) -> Option<usize> {
        log.lines().position(|l| self.matches(l))
    }

    fn count(&self, log: &str) -> usize {
        log.lines().filter(|l| self.matches(l)).count()
    }
}

/// ERROR line with the exact service field and the full peer id.
fn error_match<'a>(
    message: &'a str,
    service_field: &'a str,
    service: &'a str,
    peer: &'a str,
) -> LineMatch<'a> {
    LineMatch {
        level: "ERROR",
        message,
        fields: vec![(service_field, service), ("peer", peer)],
    }
}

/// The single back-off WARN of an Access service.
fn backoff_match(service: &str) -> LineMatch<'_> {
    LineMatch {
        level: "WARN",
        message: BACKOFF_WARNING,
        fields: vec![("svc", service)],
    }
}

/// Fixture-owned background tasks: aborted on drop (so an assertion panic
/// cannot leak them) and aborted and joined by `shutdown` on the normal path.
#[derive(Default)]
struct Tasks(Vec<tokio::task::JoinHandle<()>>);

impl Tasks {
    fn spawn(&mut self, task: impl std::future::Future<Output = ()> + Send + 'static) {
        self.0.push(tokio::spawn(task));
    }

    async fn shutdown(mut self) {
        for handle in std::mem::take(&mut self.0) {
            handle.abort();
            let _ = handle.await;
        }
    }
}

impl Drop for Tasks {
    fn drop(&mut self) {
        for handle in &self.0 {
            handle.abort();
        }
    }
}

/// How many times a role is respawned on fresh ports when its listener could
/// not come up (another process took a released port).
const PORT_ATTEMPTS: usize = 5;

/// Reserve `count` distinct free loopback ports (all held until every one is
/// chosen), then release them for a role to rebind.
async fn reserve_addrs(count: usize, udp: bool) -> Result<Vec<SocketAddr>> {
    let mut held: Vec<Box<dyn std::any::Any>> = Vec::new();
    let mut addrs = Vec::new();
    for _ in 0..count {
        if udp {
            let socket = UdpSocket::bind("127.0.0.1:0").await?;
            addrs.push(socket.local_addr()?);
            held.push(Box::new(socket));
        } else {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            addrs.push(listener.local_addr()?);
            held.push(Box::new(listener));
        }
    }
    drop(held);
    Ok(addrs)
}

/// Spawn a role on `count` reserved ports and wait until `ready` holds. When
/// the listener never appears (or the role exits) the child is reaped and the
/// role is respawned on fresh ports, up to `PORT_ATTEMPTS` times.
async fn spawn_listening(
    what: &str,
    count: usize,
    udp: bool,
    start: impl Fn(&[SocketAddr], usize) -> Result<Role>,
    ready: impl Fn(&Role, &[SocketAddr]) -> bool,
) -> Result<(Role, Vec<SocketAddr>)> {
    let mut last = None;
    for attempt in 0..PORT_ATTEMPTS {
        let addrs = reserve_addrs(count, udp).await?;
        let mut role = start(&addrs, attempt)?;
        match wait_for(what, &mut role, |r| ready(r, &addrs).then_some(())).await {
            Ok(()) => return Ok((role, addrs)),
            Err(err) => {
                eprintln!("{what}: attempt {attempt} failed on {addrs:?}: {err}");
                drop(role); // kills and reaps the child before the retry
                last = Some(err);
            }
        }
    }
    Err(last.expect("PORT_ATTEMPTS > 0")).context(format!("{what}: no usable port"))
}

/// A line of `role`'s log announcing a listener on every `addrs` entry.
fn listening(role: &Role, marker: &str, addrs: &[SocketAddr]) -> bool {
    let log = role.log();
    addrs.iter().all(|addr| {
        let want = format!("listen_addr={addr}");
        log.lines()
            .any(|l| l.contains(marker) && l.split_whitespace().any(|t| t == want))
    })
}

/// A UDP echo target that counts the datagrams it receives.
async fn udp_target(tasks: &mut Tasks) -> Result<(SocketAddr, Arc<AtomicUsize>)> {
    let socket = UdpSocket::bind("127.0.0.1:0").await?;
    let addr = socket.local_addr()?;
    let received = Arc::new(AtomicUsize::new(0));
    let counter = received.clone();
    tasks.spawn(async move {
        let mut buf = vec![0u8; 65535];
        while let Ok((n, from)) = socket.recv_from(&mut buf).await {
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = socket.send_to(&buf[..n], from).await;
        }
    });
    Ok((addr, received))
}

/// A TCP echo target; per-connection tasks live in a `JoinSet` owned by the
/// listener task, so aborting that task aborts them too.
async fn tcp_echo_target(tasks: &mut Tasks) -> Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tasks.spawn(async move {
        let mut conns = tokio::task::JoinSet::new();
        while let Ok((mut stream, _)) = listener.accept().await {
            conns.spawn(async move {
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

/// No-redial assertions shared by Steps 1 and 2. "connected to serve peer" is
/// logged once per established connection, so exactly one proves a single dial;
/// "connect attempt" is logged only for failed attempts, so it is a secondary
/// check, not a dial marker.
fn assert_single_attempt(access_log: &str, failure_line: usize, service: &str) {
    assert_eq!(count(access_log, CONNECTED), 1, "{access_log}");
    assert_eq!(backoff_match(service).count(access_log), 1, "{access_log}");
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
        let mut tasks = Tasks::default();
        let (target, received) = udp_target(&mut tasks).await?;

        let mut serve = spawn(
            root.path(),
            "serve",
            "serve",
            quick_serve("dns", target, &relay),
            true,
        )?;
        let serve_id = node_id(&mut serve, "serve").await?;
        let (mut access, addrs) = spawn_listening(
            "access udp listener",
            1,
            true,
            |addrs, n| {
                spawn(
                    root.path(),
                    &format!("access-{n}"),
                    "access",
                    quick_access("dns", addrs[0], &serve_id, "udp", &relay),
                    false,
                )
            },
            |r, addrs| listening(r, UDP_LISTENING, addrs),
        )
        .await?;
        let listen = addrs[0];
        let access_id = node_id(&mut access, "access").await?;

        let access_fail = error_match(SERVE_FAIL_ON_ACCESS, "svc", "dns", &serve_id);
        let serve_closed = error_match(SERVE_CLOSED_BY_ACCESS, "service", "dns", &access_id);
        let client = UdpSocket::bind("127.0.0.1:0").await?;
        let deadline = Instant::now() + MILESTONE_TIMEOUT;
        while access_fail.position(&access.log()).is_none()
            || serve_closed.position(&serve.log()).is_none()
        {
            anyhow::ensure!(
                Instant::now() < deadline,
                "failure lines missing\nA:{}\nS:{}",
                access.log(),
                serve.log()
            );
            client.send_to(b"ping", listen).await?;
            tokio::time::sleep(SEND_EVERY).await;
        }
        send_for(&client, listen, Duration::from_secs(5)).await?;

        assert_eq!(
            received.load(Ordering::SeqCst),
            0,
            "target must see nothing"
        );
        assert!(access.running() && serve.running(), "a role exited");
        let log = access.log();
        assert_single_attempt(&log, access_fail.position(&log).unwrap(), "dns");
        access.stop();
        serve.stop();
        tasks.shutdown().await;
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
        let mut tasks = Tasks::default();
        let (udp, received) = udp_target(&mut tasks).await?;
        let tcp = tcp_echo_target(&mut tasks).await?;
        let serve_key = SecretKey::generate();

        // Two services: multi-service role, so `run --config`.
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
        let mut serve = spawn(
            root.path(),
            "serve",
            "serve",
            Launch::Config(&serve_cfg),
            false,
        )?;
        let serve_id = node_id(&mut serve, "serve").await?;
        let (mut access, addrs) = spawn_listening(
            "access udp listener",
            1,
            true,
            |addrs, n| {
                spawn(
                    root.path(),
                    &format!("access-{n}"),
                    "access",
                    quick_access("dns", addrs[0], &serve_id, "udp", &relay),
                    true,
                )
            },
            |r, addrs| listening(r, UDP_LISTENING, addrs),
        )
        .await?;
        let dns_listen = addrs[0];
        let access_id = node_id(&mut access, "access").await?;
        let (mut web, addrs) = spawn_listening(
            "web listener",
            1,
            false,
            |addrs, n| {
                spawn(
                    root.path(),
                    &format!("web-{n}"),
                    "access",
                    quick_access("web", addrs[0], &serve_id, "tcp", &relay),
                    false,
                )
            },
            |r, addrs| listening(r, TCP_LISTENING, addrs),
        )
        .await?;
        let web_listen = addrs[0];

        let serve_fail = error_match(ACCESS_FAIL_ON_SERVE, "service", "dns", &access_id);
        let access_closed = error_match(ACCESS_CLOSED_BY_SERVE, "svc", "dns", &serve_id);
        let client = UdpSocket::bind("127.0.0.1:0").await?;
        let deadline = Instant::now() + MILESTONE_TIMEOUT;
        while serve_fail.position(&serve.log()).is_none()
            || access_closed.position(&access.log()).is_none()
        {
            anyhow::ensure!(
                Instant::now() < deadline,
                "failure lines missing\nA:{}\nS:{}",
                access.log(),
                serve.log()
            );
            client.send_to(b"ping", dns_listen).await?;
            tokio::time::sleep(SEND_EVERY).await;
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
        assert_single_attempt(&log, access_closed.position(&log).unwrap(), "dns");
        web.stop();
        access.stop();
        serve.stop();
        tasks.shutdown().await;
        Ok(())
    })
    .await?
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
        let mut tasks = Tasks::default();
        let (bad_target, bad_received) = udp_target(&mut tasks).await?;
        let (good_target, _) = udp_target(&mut tasks).await?;
        let access_key = SecretKey::generate();

        let mut serve_a = spawn(
            root.path(),
            "serve-a",
            "serve",
            quick_serve("bad", bad_target, &relay),
            true,
        )?;
        let mut serve_b = spawn(
            root.path(),
            "serve-b",
            "serve",
            quick_serve("good", good_target, &relay),
            false,
        )?;
        let a_id = node_id(&mut serve_a, "serve").await?;
        let b_id = node_id(&mut serve_b, "serve").await?;
        let (mut access, addrs) = spawn_listening(
            "access udp listeners",
            2,
            true,
            |addrs, n| {
                let cfg = write_config(
                    root.path(),
                    &format!("access-{n}.toml"),
                    &access_key,
                    &relay,
                    &[
                        Service {
                            name: "bad",
                            protocol: "udp",
                            addr: addrs[0],
                            node_id: Some(a_id.clone()),
                        },
                        Service {
                            name: "good",
                            protocol: "udp",
                            addr: addrs[1],
                            node_id: Some(b_id.clone()),
                        },
                    ],
                )?;
                spawn(
                    root.path(),
                    &format!("access-{n}"),
                    "access",
                    Launch::Config(&cfg),
                    false,
                )
            },
            |r, addrs| listening(r, UDP_LISTENING, addrs),
        )
        .await?;
        let (bad_listen, good_listen) = (addrs[0], addrs[1]);

        let client = UdpSocket::bind("127.0.0.1:0").await?;
        let failure = error_match(SERVE_FAIL_ON_ACCESS, "svc", "bad", &a_id);
        let warning = backoff_match("bad");
        let deadline = Instant::now() + MILESTONE_TIMEOUT;
        loop {
            let log = access.log();
            if failure.position(&log).is_some() && warning.position(&log).is_some() {
                break;
            }
            anyhow::ensure!(Instant::now() < deadline, "setup failure missing\n{log}");
            client.send_to(b"ping", bad_listen).await?;
            tokio::time::sleep(SEND_EVERY).await;
        }

        // Keep hitting `bad` while `good` round-trips.
        let socket = UdpSocket::bind("127.0.0.1:0").await?;
        tasks.spawn(async move {
            loop {
                let _ = socket.send_to(b"ping", bad_listen).await;
                tokio::time::sleep(SEND_EVERY).await;
            }
        });
        let result = async {
            for i in 0..3 {
                round_trip(&client, good_listen, format!("good-{i}").as_bytes()).await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
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
        tasks.shutdown().await;
        Ok(())
    })
    .await?
}
