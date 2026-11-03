//! Probe for #73: confirms the iroh 1.4.1 datagram API behaves as the
//! benchmark harness assumes, on a forced-relay path and a direct path.
//! Not shipped; see docs/design/issue-73-evidence.md.
//!
//! Run: `cargo run --example udp_bench_probe --features test-utils`

use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use iroh::endpoint::presets::Minimal;
use iroh::endpoint::{Connection, Endpoint, RelayMode};
use iroh::test_utils::run_relay_server;
use iroh::tls::CaTlsConfig;

const ALPN: &[u8] = b"udp-bench/0";

async fn build(relay_map: iroh::RelayMap, relay_only: bool, alpn: bool) -> Result<Endpoint> {
    let mut b = Endpoint::builder(Minimal)
        .relay_mode(RelayMode::Custom(relay_map))
        .ca_tls_config(CaTlsConfig::insecure_skip_verify());
    if relay_only {
        b = b.clear_ip_transports();
    }
    if alpn {
        b = b.alpns(vec![ALPN.to_vec()]);
    }
    b.bind().await.context("bind")
}

fn describe(conn: &Connection) -> String {
    let paths = conn.paths();
    let mut out = String::new();
    for p in paths.iter() {
        out.push_str(&format!(
            "[{} selected={}] ",
            if p.is_relay() { "relay" } else { "ip" },
            p.is_selected()
        ));
    }
    out
}

async fn run(relay_only: bool) -> Result<()> {
    let (relay_map, relay_url, _server) = run_relay_server().await?;
    let serve = build(relay_map.clone(), relay_only, true).await?;
    let access = build(relay_map, relay_only, false).await?;
    let addr = iroh::EndpointAddr::new(serve.id()).with_relay_url(relay_url);

    let accept = tokio::spawn({
        let serve = serve.clone();
        async move {
            let conn = serve.accept().await.context("no incoming")?.await?;
            let mut got = Vec::new();
            for _ in 0..4 {
                let d =
                    tokio::time::timeout(Duration::from_secs(5), conn.read_datagram()).await??;
                got.push(d.len());
            }
            anyhow::Ok((got, conn.max_datagram_size()))
        }
    });

    let conn = access.connect(addr, ALPN).await?;
    if !relay_only {
        // Direct is not instant: wait for holepunching to select an IP path.
        let waited = std::time::Instant::now();
        while !conn.paths().iter().any(|p| p.is_ip() && p.is_selected()) {
            anyhow::ensure!(
                waited.elapsed() < Duration::from_secs(10),
                "direct path not selected within 10 s"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        println!("direct path selected after {:?}", waited.elapsed());
    }
    let limit = conn.max_datagram_size();
    println!(
        "relay_only={relay_only} limit={limit:?} paths={}",
        describe(&conn)
    );
    let limit = limit.context("datagrams unavailable")?;
    for size in [64, 512, limit - 1, limit] {
        conn.send_datagram(Bytes::from(vec![7u8; size]))?;
    }
    // One past the limit must be refused, not truncated.
    let over = conn.send_datagram(Bytes::from(vec![7u8; limit + 1]));
    println!("oversize -> {over:?}");
    let (got, server_limit) = accept.await??;
    println!("server received sizes {got:?} server_limit={server_limit:?}");
    println!("paths after: {}", describe(&conn));
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    run(true).await?;
    run(false).await?;
    Ok(())
}
