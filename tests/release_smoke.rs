//! #97: smoke test of the PACKAGED release executables.
//!
//! Unlike every other integration test, this one never uses Cargo's own binaries
//! (they are built with `test-utils`, which turns relay TLS verification off).
//! It runs the production executables taken from the release archives and
//! forwards real TCP and UDP bytes through the public n0 relays, because a
//! production executable accepts only `https://` relays with a verified
//! certificate. It is `#[ignore]`d: it needs Internet access and the extracted
//! release executables.
//!
//! ```sh
//! RELEASE_SMOKE_BIN_DIR=/path/with/the/three/executables \
//! RELEASE_SMOKE_EVIDENCE=/path/to/evidence.json \
//!   cargo test --test release_smoke -- --ignored --nocapture
//! ```
//!
//! The `release-smoke` workflow runs it on every supported runner against the
//! `release-set` of one tag run and uploads the evidence file. Evidence names
//! the executable digests, the OS and architecture, the commands and the path
//! kinds that the status files report. It does NOT prove a forced relay path
//! (the production executables cannot be pointed at a private relay); that is
//! covered by `tests/udp_tunnel.rs` with a test build.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

const ATTEMPTS: usize = 3;
const REQUEST: &[u8] = b"release-smoke request \x00\xff";
const RESPONSE: &[u8] = b"release-smoke response \xff\x00";

struct Process(Child, PathBuf);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Which executables form one session: the all-in-one one (`iroh-tunnel serve`
/// / `iroh-tunnel access`) or the two standalone role executables.
#[derive(Clone, Copy, Debug)]
enum Form {
    AllInOne,
    Standalone,
}

#[derive(Clone, Copy, Debug)]
enum Proto {
    Tcp,
    Udp,
}

impl Proto {
    fn name(self) -> &'static str {
        match self {
            Proto::Tcp => "tcp",
            Proto::Udp => "udp",
        }
    }
}

struct Bins {
    dir: PathBuf,
}

impl Bins {
    fn path(&self, name: &str) -> PathBuf {
        let file = if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        };
        self.dir.join(file)
    }

    /// The executable and leading arguments that start `role` in `form`.
    fn command(&self, form: Form, role: &str) -> (PathBuf, Vec<String>) {
        match form {
            Form::AllInOne => (self.path("iroh-tunnel"), vec![role.to_string()]),
            Form::Standalone => (self.path(&format!("iroh-tunnel-{role}")), vec![]),
        }
    }
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn spawn(
    exe: &Path,
    args: &[String],
    home: &Path,
    log: &Path,
    commands: &mut Vec<String>,
) -> Result<Process> {
    commands.push(format!("{} {}", exe.display(), args.join(" ")));
    let out = std::fs::File::create(log)?;
    let child = Command::new(exe)
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home)
        .env("APPDATA", home)
        .env("LOCALAPPDATA", home)
        .env("IROH_TUNNEL_STATE_DIR", home)
        .env("RUST_LOG", "iroh_tunnel=info")
        .stdout(out.try_clone()?)
        .stderr(out)
        .spawn()
        .with_context(|| format!("spawn {}", exe.display()))?;
    Ok(Process(child, log.to_path_buf()))
}

/// The last lines of a role's log, so a failed start explains itself in the
/// evidence instead of only an exit code.
fn log_tail(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(12)..].join(" | ");
    if tail.is_empty() {
        "(empty)".into()
    } else {
        tail
    }
}

async fn wait_for<T>(
    what: &str,
    process: &mut Process,
    mut probe: impl FnMut() -> Option<T>,
) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(value) = probe() {
            return Ok(value);
        }
        if let Some(exit) = process.0.try_wait()? {
            anyhow::bail!(
                "{what}: process exited early: {exit}; log tail: {}",
                log_tail(&process.1)
            );
        }
        anyhow::ensure!(Instant::now() < deadline, "{what}: timed out");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// A free loopback port, reserved briefly. The window before the role binds it
/// is small; a clash fails the attempt and the attempt is retried.
async fn free_port(proto: Proto) -> Result<std::net::SocketAddr> {
    Ok(match proto {
        Proto::Tcp => TcpListener::bind("127.0.0.1:0").await?.local_addr()?,
        Proto::Udp => UdpSocket::bind("127.0.0.1:0").await?.local_addr()?,
    })
}

/// One serve/access session; returns the transport kinds the Access status
/// file reported after the exchange.
async fn session(
    bins: &Bins,
    form: Form,
    proto: Proto,
    commands: &mut Vec<String>,
) -> Result<Vec<String>> {
    let temp = tempfile::tempdir()?;
    let home = temp.path();

    // The fixture the Serve role forwards to.
    let target = free_port(proto).await?;
    let fixture = match proto {
        Proto::Tcp => {
            let listener = TcpListener::bind(target).await?;
            tokio::spawn(async move {
                while let Ok((mut stream, _)) = listener.accept().await {
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; REQUEST.len()];
                        if stream.read_exact(&mut buf).await.is_ok() && buf == REQUEST {
                            let _ = stream.write_all(RESPONSE).await;
                        }
                    });
                }
            })
        }
        Proto::Udp => {
            let socket = UdpSocket::bind(target).await?;
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                loop {
                    match socket.recv_from(&mut buf).await {
                        Ok((n, from)) if &buf[..n] == REQUEST => {
                            let _ = socket.send_to(RESPONSE, from).await;
                        }
                        Ok(_) => {}
                        // Windows reports an earlier reply's ICMP error here;
                        // pause so a persistent error cannot spin.
                        Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                    }
                }
            })
        }
    };
    let _abort = Abort(fixture);

    let (serve_exe, mut serve_args) = bins.command(form, "serve");
    serve_args.extend([
        "--name".into(),
        "smoke".into(),
        "--target".into(),
        target.to_string(),
        "--protocol".into(),
        proto.name().into(),
    ]);
    let serve_dir = home.join("serve");
    std::fs::create_dir_all(&serve_dir)?;
    let mut serve = spawn(
        &serve_exe,
        &serve_args,
        &serve_dir,
        &serve_dir.join("serve.log"),
        commands,
    )?;
    let status_path = serve_dir.join("serve-status.json");
    let node = wait_for("serve status", &mut serve, || {
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&status_path).ok()?).ok()?;
        json["node_id"].as_str().map(str::to_owned)
    })
    .await?;

    let listen = free_port(proto).await?;
    let (access_exe, mut access_args) = bins.command(form, "access");
    access_args.extend([
        "--name".into(),
        "smoke".into(),
        "--listen".into(),
        listen.to_string(),
        "--node-id".into(),
        node,
        "--protocol".into(),
        proto.name().into(),
    ]);
    let access_dir = home.join("access");
    std::fs::create_dir_all(&access_dir)?;
    let access_log = access_dir.join("access.log");
    let mut access = spawn(
        &access_exe,
        &access_args,
        &access_dir,
        &access_log,
        commands,
    )?;
    let listen_text = listen.to_string();
    wait_for("access listener", &mut access, || {
        let log = std::fs::read_to_string(&access_log).ok()?;
        log.lines()
            .any(|l| l.contains("listening for local") && l.contains(&listen_text))
            .then_some(())
    })
    .await?;

    // Exchange real bytes through the tunnel. UDP gives no connect signal, so
    // the request is resent until the first reply.
    let deadline = Instant::now() + Duration::from_secs(60);
    match proto {
        Proto::Tcp => loop {
            let attempt = async {
                let mut client = TcpStream::connect(listen).await?;
                client.write_all(REQUEST).await?;
                let mut got = vec![0u8; RESPONSE.len()];
                tokio::time::timeout(Duration::from_secs(10), client.read_exact(&mut got))
                    .await??;
                anyhow::Ok(got)
            };
            if let Ok(got) = attempt.await {
                anyhow::ensure!(got == RESPONSE, "tcp response bytes differ");
                break;
            }
            anyhow::ensure!(Instant::now() < deadline, "no tcp response through tunnel");
            tokio::time::sleep(Duration::from_millis(500)).await;
        },
        Proto::Udp => {
            let client = UdpSocket::bind("127.0.0.1:0").await?;
            let mut buf = [0u8; 2048];
            loop {
                client.send_to(REQUEST, listen).await?;
                if let Ok(Ok((n, _))) =
                    tokio::time::timeout(Duration::from_millis(700), client.recv_from(&mut buf))
                        .await
                {
                    anyhow::ensure!(&buf[..n] == RESPONSE, "udp response bytes differ");
                    break;
                }
                anyhow::ensure!(Instant::now() < deadline, "no udp response through tunnel");
            }
        }
    }

    // The status file lists the transports of the live connection. Give the
    // path report a moment to catch up with the traffic.
    let status_path = access_dir.join("access-status.json");
    let mut kinds = Vec::new();
    for _ in 0..20 {
        if let Some(json) = std::fs::read(&status_path)
            .ok()
            .and_then(|d| serde_json::from_slice::<serde_json::Value>(&d).ok())
        {
            kinds = json["services"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|s| s["transports"].as_array().into_iter().flatten())
                .filter_map(|t| t["kind"].as_str().map(str::to_owned))
                .collect();
            if !kinds.is_empty() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    kinds.sort();
    kinds.dedup();
    Ok(kinds)
}

/// Aborts the fixture task when the session ends, however it ends.
struct Abort(tokio::task::JoinHandle<()>);
impl Drop for Abort {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Facts that bind the evidence to what was tested.
fn context(executables: &[serde_json::Value]) -> serde_json::Value {
    let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock");
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    // Elevation is checked by the platform's own tool; absent means unknown.
    let elevated = if cfg!(windows) {
        Command::new("net")
            .arg("session")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .ok()
    } else {
        Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
    };
    serde_json::json!({
        "timestamp_unix": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "user": user,
        "elevated": elevated,
        "source_sha": std::env::var("RELEASE_SMOKE_SOURCE_SHA").ok(),
        "tag": std::env::var("RELEASE_SMOKE_TAG").ok(),
        "cargo_lock_sha256": sha256_file(&lock).ok(),
        "archive_sha256": std::env::var("RELEASE_SMOKE_ARCHIVES")
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok()),
        "relay": "public n0 default relays",
        "executables": executables,
    })
}

fn write_evidence(evidence: &serde_json::Value) -> Result<()> {
    let text = serde_json::to_string_pretty(evidence)?;
    if let Some(path) = std::env::var_os("RELEASE_SMOKE_EVIDENCE") {
        std::fs::write(path, &text)?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs Internet (public n0 relays) and the extracted release executables"]
async fn packaged_binaries_forward_tcp_and_udp_through_public_relay() -> Result<()> {
    let dir = PathBuf::from(
        std::env::var_os("RELEASE_SMOKE_BIN_DIR").context("set RELEASE_SMOKE_BIN_DIR")?,
    );
    let bins = Bins { dir };

    let mut executables = Vec::new();
    for name in ["iroh-tunnel", "iroh-tunnel-serve", "iroh-tunnel-access"] {
        let path = bins.path(name);
        let version = Command::new(&path)
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("run {} --version", path.display()))?;
        anyhow::ensure!(version.status.success(), "{name} --version failed");
        let version = String::from_utf8_lossy(&version.stdout).trim().to_string();
        if let Some(expected) = std::env::var_os("RELEASE_SMOKE_VERSION") {
            let expected = expected.to_string_lossy();
            anyhow::ensure!(
                version.contains(expected.as_ref()),
                "{name}: version {version:?} does not contain {expected:?}"
            );
        }
        executables.push(serde_json::json!({
            "name": name,
            "sha256": sha256_file(&path)?,
            "version": version,
        }));
    }

    let mut evidence = context(&executables);
    // Written after every session so a crash or a job timeout still leaves a
    // record of what ran; `complete` flips only when all sessions finished.
    evidence["complete"] = false.into();
    evidence["results"] = serde_json::json!([]);
    write_evidence(&evidence)?;
    let mut results = Vec::new();
    let mut failed = Vec::new();
    for form in [Form::AllInOne, Form::Standalone] {
        for proto in [Proto::Tcp, Proto::Udp] {
            let mut commands = Vec::new();
            let mut outcome = Err(anyhow::anyhow!("not run"));
            let mut attempts = 0;
            for attempt in 1..=ATTEMPTS {
                attempts = attempt;
                commands.clear();
                outcome = session(&bins, form, proto, &mut commands).await;
                if outcome.is_ok() {
                    break;
                }
                eprintln!("{form:?} {proto:?} attempt {attempt}: {outcome:?}");
            }
            let (status, kinds, error) = match &outcome {
                Ok(kinds) => ("PASS", kinds.clone(), None),
                Err(error) => ("FAIL", vec![], Some(format!("{error:#}"))),
            };
            if status == "FAIL" {
                failed.push(format!("{form:?}/{}", proto.name()));
            }
            results.push(serde_json::json!({
                "case": "RELEASE-MATRIX-01",
                "form": format!("{form:?}"),
                "protocol": proto.name(),
                "status": status,
                "attempts": attempts,
                "reported_path_kinds": kinds,
                "error": error,
                "commands": commands,
            }));
            evidence["results"] = results.clone().into();
            write_evidence(&evidence)?;
        }
    }
    evidence["complete"] = true.into();
    write_evidence(&evidence)?;
    println!("{}", serde_json::to_string_pretty(&evidence)?);

    anyhow::ensure!(failed.is_empty(), "failed sessions: {failed:?}");
    Ok(())
}
