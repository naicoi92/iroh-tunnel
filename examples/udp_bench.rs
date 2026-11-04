//! Benchmark harness for #73: QUIC DATAGRAM versus a framed-stream reference.
//!
//! Drives synthetic flows between two in-process Iroh endpoints over a forced
//! relay path or a verified direct path, checks the exclusion criteria (E1, E2, E4 per run; E3 is sampled) and
//! prints one JSON object per run. It measures the transport only: there is no
//! tunnel, so cross-delivery (E2) is checked as header/body consistency, and
//! loss, delay and congestion must be injected by the operator outside this
//! process (see the evidence file). Not shipped.
//!
//! ```text
//! cargo run --release --example udp_bench --features test-utils --locked -- \
//!     --path relay --transport datagram --payload limit --flows 10 --count 500
//! ```

use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use clap::{Parser, ValueEnum};
use iroh::endpoint::presets::Minimal;
use iroh::endpoint::{Connection, Endpoint, RelayMode, SendDatagramError};
use iroh::test_utils::run_relay_server;
use iroh::tls::CaTlsConfig;

const ALPN: &[u8] = b"udp-bench/0";
/// Header: flow id (u32), sequence (u32), send time in ns since run start (u64).
const HEADER: usize = 16;
/// Largest `--payload`; QUIC datagrams and this harness never need more.
const MAX_PAYLOAD: usize = 65_535;
const DIRECT_WAIT: Duration = Duration::from_secs(10);
const RSS_SAMPLE_EVERY: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, ValueEnum)]
enum PathKind {
    /// IP transports are removed, so every packet crosses the relay.
    Relay,
    /// Waits for an IP path to be selected; fails if none is.
    Direct,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Transport {
    Datagram,
    /// Reference baseline only; one uni stream per flow, length-prefixed frames.
    Stream,
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, value_enum)]
    path: PathKind,
    #[arg(long, value_enum)]
    transport: Transport,
    /// Total datagram size: a byte count, or `limit-1`, `limit`, `limit+1`.
    #[arg(long, default_value = "512")]
    payload: String,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    flows: u32,
    /// Messages per flow.
    #[arg(long, default_value_t = 200)]
    count: u32,
    /// Messages per second per flow.
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..))]
    pps: u32,
    /// Milliseconds without arrivals, after the senders finish, that end a run.
    /// Raise it above the injected one-way delay, or late datagrams count as lost.
    #[arg(long, default_value_t = 2000, value_parser = clap::value_parser!(u64).range(1..=3_600_000))]
    idle_end_ms: u64,
    /// Free-text label for the run (for example the injected network condition).
    #[arg(long, default_value = "")]
    note: String,
}

#[derive(Default)]
struct Counters {
    /// Set once every sender task has finished, so receivers can stop on idle.
    sender_done: AtomicBool,
    sent: AtomicU64,
    refused_too_large: AtomicU64,
    refused_other: AtomicU64,
    received: AtomicU64,
    /// E4: delivered with a length that differs from what was sent.
    bad_length: AtomicU64,
    /// E1/E2: header and body disagree, or header names an unknown flow.
    corrupt: AtomicU64,
}

/// Deterministic fill so the receiver can verify the body without a copy.
fn fill_byte(flow: u32, seq: u32) -> u8 {
    (flow.wrapping_mul(31).wrapping_add(seq) & 0xff) as u8
}

fn encode(flow: u32, seq: u32, size: usize, sent_ns: u64) -> Vec<u8> {
    let mut buf = vec![fill_byte(flow, seq); size];
    buf[0..4].copy_from_slice(&flow.to_be_bytes());
    buf[4..8].copy_from_slice(&seq.to_be_bytes());
    buf[8..16].copy_from_slice(&sent_ns.to_be_bytes());
    buf
}

/// Returns `(flow, seq, sent_ns)` when the body matches the header.
fn decode(buf: &[u8], flows: u32) -> Option<(u32, u32, u64)> {
    if buf.len() < HEADER {
        return None;
    }
    let flow = u32::from_be_bytes(buf[0..4].try_into().ok()?);
    let seq = u32::from_be_bytes(buf[4..8].try_into().ok()?);
    let sent_ns = u64::from_be_bytes(buf[8..16].try_into().ok()?);
    let fill = fill_byte(flow, seq);
    (flow < flows && buf[HEADER..].iter().all(|b| *b == fill)).then_some((flow, seq, sent_ns))
}

async fn build(relay_map: iroh::RelayMap, relay_only: bool, serve: bool) -> Result<Endpoint> {
    let mut b = Endpoint::builder(Minimal)
        .relay_mode(RelayMode::Custom(relay_map))
        .ca_tls_config(CaTlsConfig::insecure_skip_verify());
    if relay_only {
        b = b.clear_ip_transports();
    }
    if serve {
        b = b.alpns(vec![ALPN.to_vec()]);
    }
    b.bind().await.context("bind endpoint")
}

fn selected_path(conn: &Connection) -> &'static str {
    let paths = conn.paths();
    if paths.iter().any(|p| p.is_selected() && p.is_ip()) {
        "ip"
    } else if paths.iter().any(|p| p.is_selected() && p.is_relay()) {
        "relay"
    } else {
        "none"
    }
}

async fn wait_for_path(conn: &Connection, want: PathKind) -> Result<()> {
    let deadline = Instant::now() + DIRECT_WAIT;
    let want_name = match want {
        PathKind::Relay => "relay",
        PathKind::Direct => "ip",
    };
    while selected_path(conn) != want_name {
        if Instant::now() > deadline {
            bail!("path {want_name} not selected within {DIRECT_WAIT:?}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

fn resolve_payload(spec: &str, limit: usize) -> Result<usize> {
    let size = match spec {
        "limit-1" => limit.saturating_sub(1),
        "limit" => limit,
        "limit+1" => limit + 1,
        n => n
            .parse()
            .with_context(|| format!("invalid --payload {n:?}"))?,
    };
    if !(HEADER..=MAX_PAYLOAD).contains(&size) {
        bail!("payload {size} must be between {HEADER} and {MAX_PAYLOAD} bytes");
    }
    Ok(size)
}

/// Note: `ps` is forked every `RSS_SAMPLE_EVERY` during the measured window, which
/// perturbs CPU and latency a little; treat process figures as indicators only.
/// Runs [`sample_process`] on a blocking thread so `ps` never stalls a runtime worker
/// while latency is being measured.
async fn sample_process_blocking() -> Option<(u64, String)> {
    tokio::task::spawn_blocking(sample_process)
        .await
        .ok()
        .flatten()
}

/// Coarse process memory and CPU sample via `ps`; avoids `unsafe` and extra dependencies.
fn sample_process() -> Option<(u64, String)> {
    let out = Command::new("ps")
        .args(["-o", "rss=,time=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut it = text.split_whitespace();
    let rss_kb = it.next()?.parse().ok()?;
    Some((rss_kb, it.next()?.to_owned()))
}

#[derive(Default)]
struct Arrivals {
    /// Latency in ns of every arrival, for percentiles.
    latencies: Vec<u64>,
    /// Latency in ns per flow, in arrival order, for jitter.
    per_flow: HashMap<u32, Vec<u64>>,
    /// Highest sequence seen per flow, for reorder and duplicate counting.
    highest: HashMap<u32, u32>,
    seen: HashMap<(u32, u32), u32>,
    reordered: u64,
    duplicated: u64,
}

impl Arrivals {
    fn record(&mut self, flow: u32, seq: u32, latency_ns: u64) {
        self.latencies.push(latency_ns);
        self.per_flow.entry(flow).or_default().push(latency_ns);
        let count = self.seen.entry((flow, seq)).or_insert(0);
        *count += 1;
        if *count > 1 {
            // A duplicate is not also counted as reordered.
            self.duplicated += 1;
            return;
        }
        let high = self.highest.entry(flow).or_insert(seq);
        if seq < *high {
            self.reordered += 1;
        } else {
            *high = seq;
        }
    }

    /// Folds another flow's arrivals in; per-flow `(flow, seq)` keys never overlap.
    fn merge(&mut self, other: Arrivals) {
        self.latencies.extend(other.latencies);
        self.per_flow.extend(other.per_flow);
        self.seen.extend(other.seen);
        self.reordered += other.reordered;
        self.duplicated += other.duplicated;
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx]
}

/// Mean absolute difference between consecutive arrivals of the same flow,
/// averaged over all consecutive pairs of all flows.
fn jitter_ns(per_flow: &HashMap<u32, Vec<u64>>) -> u64 {
    let (total, pairs) = per_flow
        .values()
        .flat_map(|l| l.windows(2))
        .fold((0u64, 0u64), |(t, n), w| (t + w[0].abs_diff(w[1]), n + 1));
    total.checked_div(pairs).unwrap_or(0)
}

async fn receive_datagrams(
    conn: Connection,
    size: usize,
    flows: u32,
    start: Instant,
    idle_end: Duration,
    counters: Arc<Counters>,
) -> Arrivals {
    let mut arrivals = Arrivals::default();
    loop {
        match tokio::time::timeout(idle_end, conn.read_datagram()).await {
            Ok(Ok(data)) => {
                let now = start.elapsed().as_nanos() as u64;
                record_message(&data, size, flows, now, &counters, &mut arrivals);
            }
            // Connection closed.
            Ok(Err(_)) => break,
            // Idle: stop once every sender has finished.
            Err(_) if counters.sender_done.load(Ordering::Relaxed) => break,
            Err(_) => {}
        }
    }
    arrivals
}

fn record_message(
    data: &[u8],
    size: usize,
    flows: u32,
    now_ns: u64,
    counters: &Counters,
    arrivals: &mut Arrivals,
) {
    counters.received.fetch_add(1, Ordering::Relaxed);
    if data.len() != size {
        counters.bad_length.fetch_add(1, Ordering::Relaxed);
        return;
    }
    match decode(data, flows) {
        Some((flow, seq, sent_ns)) => arrivals.record(flow, seq, now_ns.saturating_sub(sent_ns)),
        None => {
            counters.corrupt.fetch_add(1, Ordering::Relaxed);
        }
    }
}

async fn receive_streams(
    conn: Connection,
    size: usize,
    flows: u32,
    start: Instant,
    idle_end: Duration,
    counters: Arc<Counters>,
) -> Arrivals {
    let mut readers = Vec::new();
    for _ in 0..flows {
        let Ok(Ok(mut rx)) =
            tokio::time::timeout(idle_end.saturating_mul(5), conn.accept_uni()).await
        else {
            break;
        };
        let counters = Arc::clone(&counters);
        readers.push(tokio::spawn(async move {
            let mut arrivals = Arrivals::default();
            let mut len_buf = [0u8; 4];
            while rx.read_exact(&mut len_buf).await.is_ok() {
                let len = u32::from_be_bytes(len_buf) as usize;
                // Guard the allocation: a frame can never exceed the run's size.
                if len > size {
                    counters.corrupt.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                let mut body = vec![0u8; len];
                if rx.read_exact(&mut body).await.is_err() {
                    break;
                }
                let now = start.elapsed().as_nanos() as u64;
                record_message(&body, size, flows, now, &counters, &mut arrivals);
            }
            arrivals
        }));
    }
    let mut merged = Arrivals::default();
    for reader in readers {
        if let Ok(arrivals) = reader.await {
            merged.merge(arrivals);
        }
    }
    merged
}

async fn send_datagrams(
    conn: Connection,
    args: &Args,
    size: usize,
    start: Instant,
    counters: Arc<Counters>,
) -> Result<()> {
    let tick = Duration::from_secs_f64(1.0 / f64::from(args.pps));
    let mut tasks = Vec::new();
    for flow in 0..args.flows {
        let conn = conn.clone();
        let counters = Arc::clone(&counters);
        let count = args.count;
        tasks.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(tick);
            for seq in 0..count {
                interval.tick().await;
                let msg = encode(flow, seq, size, start.elapsed().as_nanos() as u64);
                match conn.send_datagram(Bytes::from(msg)) {
                    Ok(()) => counters.sent.fetch_add(1, Ordering::Relaxed),
                    Err(SendDatagramError::TooLarge) => {
                        counters.refused_too_large.fetch_add(1, Ordering::Relaxed)
                    }
                    Err(_) => counters.refused_other.fetch_add(1, Ordering::Relaxed),
                };
            }
        }));
    }
    for task in tasks {
        task.await.context("sender task failed")?;
    }
    Ok(())
}

async fn send_streams(
    conn: Connection,
    args: &Args,
    size: usize,
    start: Instant,
    counters: Arc<Counters>,
) -> Result<()> {
    let tick = Duration::from_secs_f64(1.0 / f64::from(args.pps));
    let mut tasks = Vec::new();
    for flow in 0..args.flows {
        let conn = conn.clone();
        let counters = Arc::clone(&counters);
        let count = args.count;
        let idle_end = Duration::from_millis(args.idle_end_ms);
        tasks.push(tokio::spawn(async move {
            let Ok(mut tx) = conn.open_uni().await else {
                counters.refused_other.fetch_add(1, Ordering::Relaxed);
                return;
            };
            let mut interval = tokio::time::interval(tick);
            for seq in 0..count {
                interval.tick().await;
                let msg = encode(flow, seq, size, start.elapsed().as_nanos() as u64);
                // Bounded by MAX_PAYLOAD, so the conversion cannot fail.
                let len = u32::try_from(msg.len()).expect("payload bounded by MAX_PAYLOAD");
                let mut frame = Vec::with_capacity(4 + msg.len());
                frame.extend_from_slice(&len.to_be_bytes());
                frame.extend_from_slice(&msg);
                if tx.write_all(&frame).await.is_ok() {
                    counters.sent.fetch_add(1, Ordering::Relaxed);
                } else {
                    counters.refused_other.fetch_add(1, Ordering::Relaxed);
                }
            }
            let _ = tx.finish();
            // The peer may never read (heavy loss); do not hang the run on it.
            let _ = tokio::time::timeout(idle_end.saturating_mul(5), tx.stopped()).await;
        }));
    }
    for task in tasks {
        task.await.context("sender task failed")?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let relay_only = matches!(args.path, PathKind::Relay);
    let (relay_map, relay_url, _server) = run_relay_server().await?;
    let serve = build(relay_map.clone(), relay_only, true).await?;
    let access = build(relay_map, relay_only, false).await?;
    let addr = iroh::EndpointAddr::new(serve.id()).with_relay_url(relay_url);

    let accepting = tokio::spawn({
        let serve = serve.clone();
        async move {
            serve
                .accept()
                .await
                .context("no incoming")?
                .await
                .map_err(anyhow::Error::from)
        }
    });
    let conn = access.connect(addr, ALPN).await?;
    let server_conn = accepting.await??;
    wait_for_path(&conn, args.path).await?;

    let limit = conn.max_datagram_size();
    let size = match (args.transport, limit) {
        (Transport::Datagram, None) => bail!("datagrams unavailable on this connection"),
        (Transport::Datagram, Some(l)) => resolve_payload(&args.payload, l)?,
        (Transport::Stream, l) => resolve_payload(&args.payload, l.unwrap_or(1200))?,
    };
    let path = selected_path(&conn);

    let idle_end = Duration::from_millis(args.idle_end_ms);
    let counters = Arc::new(Counters::default());
    let start = Instant::now();
    let rss_before = sample_process_blocking().await;
    let peak_rss = Arc::new(AtomicU64::new(0));
    let sampler = tokio::spawn({
        let peak = Arc::clone(&peak_rss);
        async move {
            loop {
                if let Some((rss, _)) = sample_process_blocking().await {
                    peak.fetch_max(rss, Ordering::Relaxed);
                }
                tokio::time::sleep(RSS_SAMPLE_EVERY).await;
            }
        }
    });

    let receiver = match args.transport {
        Transport::Datagram => tokio::spawn(receive_datagrams(
            server_conn.clone(),
            size,
            args.flows,
            start,
            idle_end,
            Arc::clone(&counters),
        )),
        Transport::Stream => tokio::spawn(receive_streams(
            server_conn.clone(),
            size,
            args.flows,
            start,
            idle_end,
            Arc::clone(&counters),
        )),
    };
    match args.transport {
        Transport::Datagram => {
            send_datagrams(conn.clone(), &args, size, start, Arc::clone(&counters)).await?;
        }
        Transport::Stream => {
            send_streams(conn.clone(), &args, size, start, Arc::clone(&counters)).await?;
        }
    }
    counters.sender_done.store(true, Ordering::Relaxed);
    let send_elapsed = start.elapsed();
    let path_after = selected_path(&conn);
    let limit_after = conn.max_datagram_size();
    let arrivals = receiver.await?;
    sampler.abort();
    // Cancelled by the abort above; the join error is the expected outcome.
    let _ = sampler.await;
    let rss_after = sample_process_blocking().await;

    let mut sorted = arrivals.latencies.clone();
    sorted.sort_unstable();
    let sent = counters.sent.load(Ordering::Relaxed);
    let received = counters.received.load(Ordering::Relaxed);
    let attempted = u64::from(args.flows) * u64::from(args.count);
    let unique = arrivals.seen.len() as u64;
    let bytes = unique * size as u64;
    let secs = send_elapsed.as_secs_f64().max(f64::EPSILON);

    let report = serde_json::json!({
        "transport": format!("{:?}", args.transport),
        "path_requested": format!("{:?}", args.path),
        "path_selected_before": path,
        "path_selected_after": path_after,
        "datagram_limit_before": limit,
        "datagram_limit_after": limit_after,
        "payload": size,
        "flows": args.flows,
        "per_flow_messages": args.count,
        "pps_per_flow": args.pps,
        "note": args.note,
        "attempted": attempted,
        "sent": sent,
        "refused_too_large": counters.refused_too_large.load(Ordering::Relaxed),
        "refused_other": counters.refused_other.load(Ordering::Relaxed),
        "received": received,
        "unique_received": unique,
        "lost": sent.saturating_sub(unique),
        "reordered": arrivals.reordered,
        "duplicated": arrivals.duplicated,
        "latency_ms": {
            "p50": percentile(&sorted, 0.50) as f64 / 1e6,
            "p95": percentile(&sorted, 0.95) as f64 / 1e6,
            "p99": percentile(&sorted, 0.99) as f64 / 1e6,
            "max": sorted.last().copied().unwrap_or(0) as f64 / 1e6,
            "jitter_mean_abs_delta": jitter_ns(&arrivals.per_flow) as f64 / 1e6,
        },
        "throughput_bytes_per_s": bytes as f64 / secs,
        "process": {
            "rss_kb_before": rss_before.as_ref().map(|s| s.0),
            "rss_kb_peak": Some(peak_rss.load(Ordering::Relaxed)).filter(|v| *v > 0),
            "rss_kb_after": rss_after.as_ref().map(|s| s.0),
            "cpu_time_before": rss_before.map(|s| s.1),
            "cpu_time_after": rss_after.map(|s| s.1),
        },
        "exclusion": {
            "E1_E2_corrupt_or_inconsistent": counters.corrupt.load(Ordering::Relaxed),
            "E4_bad_length": counters.bad_length.load(Ordering::Relaxed),
        },
    });
    println!("{report}");
    conn.close(0u32.into(), b"done");
    Ok(())
}
