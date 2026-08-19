//! Iroh [`Endpoint`] construction from a [`NodeConfig`].
//!
//! Implements T-05. Two roles:
//! - **serve**: persistent [`SecretKey`] resolved from config (and persisted
//!   back if freshly generated).
//! - **access**: ephemeral key, never persisted.
//!
//! Multi-relay: if `relay_urls` is non-empty, the first URL is the home relay
//! (advertised to peers) and the rest are failover candidates. An empty list
//! falls back to the n0 default relay map via [`RelayMode::Default`].
//!
//! Based on Page 05 v3 §6 (relay & discovery). Note: iroh 1.0 changed the
//! endpoint builder API vs. earlier drafts — see the API notes below.
//
// Consumed by the serve/access handlers (T-06/T-07); flagged dead code until
// then by the binary crate's single-crate layout.
#![allow(dead_code)]

use anyhow::{Context, Result};
use iroh::endpoint::presets::Minimal;
use iroh::endpoint::{Endpoint, QuicTransportConfig, RelayMode, VarInt};
use iroh::RelayUrl;

use std::time::Duration;

/// The n0 default relay URLs (use1-1, usw1-1, euc1-1, aps1-1).
///
/// Relays are optional in iroh-tunnel config: when a user does not configure
/// `relay_urls`, we still need *some* addressing information to dial a peer
/// (an `EndpointAddr` containing only a node id cannot be routed — see
/// `Endpoint::connect` docs in iroh 1.0). These n0-operated public relays are
/// the fallback, mirroring iroh's own [`RelayMode::Default`].
///
/// Why a dedicated helper instead of `RelayMode::Default` on the endpoint:
/// `RelayMode::Default` only lets *this* endpoint use the n0 relays for itself.
/// It does **not** help dial an arbitrary remote peer — to reach a peer through
/// a relay you must attach that relay's URL to the peer's `EndpointAddr`
/// explicitly (IROHTUN-44).
///
/// [`RelayMode::Default`]: iroh::endpoint::RelayMode::Default
pub fn n0_default_relay_urls() -> Vec<RelayUrl> {
    iroh::defaults::prod::default_relay_map().urls::<Vec<_>>()
}

use crate::config::{self, NodeConfig};

/// Build an [`Endpoint`] for the **serve** role.
///
/// The secret key is resolved from `node.secret_key`; if it was empty a fresh
/// one is generated and the caller is expected to persist it (the config layer
/// handles that via [`config::ServeConfig::resolve_and_save_key`]).
///
/// `alpns` are the service ALPNs this endpoint will accept incoming streams on.
/// In iroh 1.0 ALPNs are registered on the endpoint at build time (not filtered
/// per-`accept`), so the serve handler collects every service's ALPN up front
/// and passes the whole list here.
pub async fn create_serve_endpoint(node: &NodeConfig, alpns: &[Vec<u8>]) -> Result<Endpoint> {
    // resolve_secret_key returns (key, needs_save); serve callers persist via
    // ServeConfig::resolve_and_save_key, so the boolean is ignored here.
    let (key, _needs_save) = config::resolve_secret_key(&node.secret_key)?;
    create_endpoint_with_key(key, node, alpns).await
}

/// Build an [`Endpoint`] for the **access** role.
///
/// Resolves the secret key from `node.secret_key`; if it was empty a fresh one
/// is generated and the caller is expected to persist it (the config layer
/// handles that via [`config::AccessConfig::resolve_and_save_key`]), so the
/// access NodeId is stable across restarts. Access only dials out, so no ALPNs
/// are registered.
pub async fn create_access_endpoint(node: &NodeConfig) -> Result<Endpoint> {
    // resolve_secret_key returns (key, needs_save); access callers persist via
    // AccessConfig::resolve_and_save_key, so the boolean is ignored here.
    let (key, _needs_save) = config::resolve_secret_key(&node.secret_key)?;
    create_endpoint_with_key(key, node, &[]).await
}

async fn create_endpoint_with_key(
    key: iroh::SecretKey,
    node: &NodeConfig,
    alpns: &[Vec<u8>],
) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(Minimal)
        .secret_key(key)
        .transport_config(transport_config(node.max_concurrent_streams));

    builder = builder.relay_mode(relay_mode_from_urls(&node.relay_urls)?);

    // Empty ALPN list is fine for access (outbound only); serve registers every
    // service ALPN so it can accept on all of them.
    if !alpns.is_empty() {
        builder = builder.alpns(alpns.to_vec());
    }

    builder.bind().await.context("failed to bind iroh endpoint")
}

/// Keep-alive interval for long-lived multiplexed connections.
///
/// iroh's default transport config already sends QUIC keep-alives every 5s
/// (iroh 1.0 `HEARTBEAT_INTERVAL`); we set it explicitly so long-lived
/// multiplexed connections do not silently depend on that default surviving
/// upstream changes.
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(5);

/// Transport config for stream multiplexing: pinned keep-alive plus an
/// optional concurrent-bidi-stream budget override.
///
/// `max_streams = None` (config absent) keeps noq's own default (100
/// concurrent bidirectional streams) — the budget is headroom tuning, not a
/// requirement, so it should only be raised after measuring the real
/// concurrent-channel count of the target workload. When the budget is
/// exhausted, further `open_bi` calls are flow-control blocked until another
/// stream closes. Worst-case buffer memory scales with
/// `max_concurrent_bidi_streams × stream_receive_window`, so raising it has a
/// memory cost.
///
/// Built from [`QuicTransportConfig::builder`] so every other iroh default
/// (path idle timeouts, multipath limits, …) is preserved unchanged.
fn transport_config(max_streams: Option<u32>) -> QuicTransportConfig {
    let mut builder = QuicTransportConfig::builder().keep_alive_interval(KEEP_ALIVE_INTERVAL);
    if let Some(max_streams) = max_streams {
        builder = builder.max_concurrent_bidi_streams(VarInt::from_u32(max_streams));
    }
    builder.build()
}

/// Translate the config `relay_urls` into a [`RelayMode`].
///
/// - Empty → [`RelayMode::Default`] (n0 public relays).
/// - Non-empty → [`RelayMode::custom`] with the first URL as home relay and
///   the rest as failover (relay servers are stateless, so any can serve a
///   peer; iroh advertises the home relay in the node's endpoint info).
fn relay_mode_from_urls(relay_urls: &[String]) -> Result<RelayMode> {
    if relay_urls.is_empty() {
        return Ok(RelayMode::Default);
    }
    let urls: Vec<RelayUrl> = relay_urls
        .iter()
        .map(|s| {
            s.parse::<RelayUrl>()
                .with_context(|| format!("invalid relay_url: {s}"))
        })
        .collect::<Result<_>>()?;
    Ok(RelayMode::custom(urls))
}

/// The node id (public key) of an [`Endpoint`], as its base32 string form.
pub fn node_id_string(ep: &Endpoint) -> String {
    ep.secret_key().public().to_string()
}

/// The home relay URL of an [`Endpoint`], if one has been assigned yet.
///
/// iroh assigns the home relay asynchronously after the endpoint connects to a
/// relay server, so this may return `None` right after bind. Serve prints it for
/// operator convenience (Page 06 v5 §1.1); access uses the config's
/// `relay_urls` to dial, so this is display-only.
pub fn home_relay(ep: &Endpoint) -> Option<RelayUrl> {
    ep.addr().relay_urls().next().cloned()
}

/// Resolve the relay URLs to dial peers through, from the config form.
///
/// This is the access-side counterpart to [`relay_mode_from_urls`]: where the
/// latter decides how *this* endpoint uses relays for itself, this one
/// decides which relay URLs to attach to a *dialed peer's* `EndpointAddr`.
///
/// - Empty input → fall back to [`n0_default_relay_urls`] (mirrors
///   `RelayMode::Default`). The access role cannot dial a peer with an empty
///   relay list under the `Minimal` preset — see IROHTUN-44.
/// - Non-empty input → parse each entry as a [`RelayUrl`], surfacing the
///   invalid entry in the error.
///
/// Lifted out of `access::run` so the parsing logic has a single home
/// (previously it was duplicated with `relay_mode_from_urls` and inlined in
/// the access handler).
pub fn resolve_relay_urls(config_urls: &[String]) -> Result<Vec<RelayUrl>> {
    if config_urls.is_empty() {
        return Ok(n0_default_relay_urls());
    }
    config_urls
        .iter()
        .map(|s| {
            s.parse::<RelayUrl>()
                .with_context(|| format!("invalid relay_url: {s}"))
        })
        .collect()
}

/// Build the [`EndpointAddr`] to dial `node_id` through `relay_urls`.
///
/// Each URL in `relay_urls` is attached via [`EndpointAddr::with_relay_url`]
/// so iroh can try each in turn. With no relay URLs and no address-lookup
/// service (the `Minimal` preset registers none), iroh returns "No
/// addressing information available" — so callers must pass a non-empty
/// list (the access role does this by routing through [`resolve_relay_urls`]
/// which falls back to the n0 defaults).
///
/// Lifted out of `access::handle_local_connection` so `iroh::EndpointAddr`
/// construction stops leaking across the seam.
pub fn build_dial_addr(node_id: iroh::EndpointId, relay_urls: &[RelayUrl]) -> iroh::EndpointAddr {
    let mut addr = iroh::EndpointAddr::new(node_id);
    for url in relay_urls {
        addr = addr.with_relay_url(url.clone());
    }
    addr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_relay_urls_yields_default_mode() {
        let mode = relay_mode_from_urls(&[]).unwrap();
        assert!(matches!(mode, RelayMode::Default));
    }

    #[test]
    fn n0_default_relay_urls_returns_n0_hostnames() {
        // The fallback for empty config must return the n0-operated public
        // relays (IROHTUN-44). These hostnames are iroh's own defaults — if
        // this set ever changes upstream, the test will tell us.
        let urls = n0_default_relay_urls();
        assert!(!urls.is_empty(), "expected at least one default relay");
        let hostnames: Vec<String> = urls.iter().map(|u| u.to_string()).collect();
        assert!(
            hostnames
                .iter()
                .any(|h| h == "https://use1-1.relay.n0.iroh.link./"),
            "missing NA east relay: {hostnames:?}"
        );
    }

    #[test]
    fn custom_relay_urls_parse_into_custom_mode() {
        let urls = vec![
            "https://use1-1.relay.n0.iroh.link.".to_string(),
            "https://euw-1.relay.n0.iroh.link.".to_string(),
        ];
        let mode = relay_mode_from_urls(&urls).unwrap();
        match mode {
            RelayMode::Custom(map) => {
                // both URLs present in the map (urls() collects into Vec here)
                let collected: Vec<String> = map
                    .urls::<Vec<RelayUrl>>()
                    .into_iter()
                    .map(|u| u.to_string())
                    .collect();
                assert!(collected
                    .iter()
                    .any(|u| u == "https://use1-1.relay.n0.iroh.link./"));
                assert!(collected
                    .iter()
                    .any(|u| u == "https://euw-1.relay.n0.iroh.link./"));
            }
            other => panic!("expected Custom, got {other:?}"),
        }
    }

    #[test]
    fn invalid_relay_url_errors() {
        let urls = vec!["not a url".to_string()];
        let err = relay_mode_from_urls(&urls).unwrap_err();
        assert!(format!("{err:#}").contains("invalid relay_url"));
    }

    #[tokio::test]
    async fn access_endpoint_binds_and_has_node_id() {
        // ephemeral key, default (n0) relays
        let node = NodeConfig::default();
        let ep = create_access_endpoint(&node).await.unwrap();
        let id = node_id_string(&ep);
        // iroh 1.0's PublicKey Display is lowercase hex (32 bytes => 64 chars).
        // (Parsing accepts both hex and base32, but Display emits hex.)
        assert_eq!(id.len(), 64, "node_id string: {id}");
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn serve_endpoint_with_same_key_has_same_node_id() {
        let (key, _) = config::resolve_secret_key("").unwrap();
        let enc = config::encode_secret_key(&key);
        let node = NodeConfig {
            secret_key: enc,
            relay_urls: vec![],
            max_concurrent_streams: None,
        };
        let ep = create_serve_endpoint(&node, &[b"iroh-tunnel/db".to_vec()])
            .await
            .unwrap();
        assert_eq!(node_id_string(&ep), key.public().to_string());
    }

    #[tokio::test]
    async fn access_endpoint_with_same_key_has_same_node_id() {
        // A pinned access secret_key must yield a stable NodeId across endpoints
        // (the whole point of pinning access identity).
        let (key, _) = config::resolve_secret_key("").unwrap();
        let enc = config::encode_secret_key(&key);
        let node = NodeConfig {
            secret_key: enc,
            relay_urls: vec![],
            max_concurrent_streams: None,
        };
        let ep = create_access_endpoint(&node).await.unwrap();
        assert_eq!(node_id_string(&ep), key.public().to_string());
    }

    // ---- resolve_relay_urls + build_dial_addr tests ----

    #[test]
    fn resolve_relay_urls_falls_back_to_n0_defaults_when_empty() {
        // Empty config must return the n0-operated public relays (IROHTUN-44).
        let urls = resolve_relay_urls(&[]).unwrap();
        assert!(!urls.is_empty(), "expected fallback to n0 defaults");
        let hostnames: Vec<String> = urls.iter().map(|u| u.to_string()).collect();
        assert!(
            hostnames.iter().any(|h| h.contains("relay.n0.iroh.link")),
            "no n0 relay hostname in fallback: {hostnames:?}"
        );
    }

    #[test]
    fn resolve_relay_urls_parses_configured_urls() {
        let cfg = vec![
            "https://use1-1.relay.n0.iroh.link.".to_string(),
            "https://euw-1.relay.n0.iroh.link.".to_string(),
        ];
        let urls = resolve_relay_urls(&cfg).unwrap();
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].to_string(), "https://use1-1.relay.n0.iroh.link./");
    }

    #[test]
    fn resolve_relay_urls_errors_on_invalid_url() {
        let cfg = vec!["not a url".to_string()];
        let err = resolve_relay_urls(&cfg).unwrap_err();
        assert!(format!("{err:#}").contains("invalid relay_url"));
    }

    #[test]
    fn build_dial_addr_attaches_every_relay_url() {
        // A node_id we can parse from a known-public hex string. Use a freshly
        // generated key's public so we don't hard-code a magic value.
        let key = iroh::SecretKey::generate();
        let node_id: iroh::EndpointId = key.public();
        let urls: Vec<RelayUrl> = vec![
            "https://use1-1.relay.n0.iroh.link./".parse().unwrap(),
            "https://euw-1.relay.n0.iroh.link./".parse().unwrap(),
            "https://aps1-1.relay.n0.iroh.link./".parse().unwrap(),
        ];

        let addr = build_dial_addr(node_id, &urls);

        // Every URL must be attached so iroh can try each in turn.
        let attached: Vec<String> = addr.relay_urls().map(|u| u.to_string()).collect();
        for u in &urls {
            let s = u.to_string();
            assert!(
                attached.contains(&s),
                "missing relay URL {s} in addr: {attached:?}"
            );
        }
        assert_eq!(attached.len(), urls.len(), "relay URL count mismatch");
    }

    #[test]
    fn build_dial_addr_with_empty_urls_is_just_the_node_id() {
        // The caller is required to pass a non-empty list (resolve_relay_urls
        // guarantees this), but build_dial_addr itself should not panic on
        // empty — it should produce a minimal address carrying only the node id.
        let key = iroh::SecretKey::generate();
        let node_id: iroh::EndpointId = key.public();
        let addr = build_dial_addr(node_id, &[]);
        assert_eq!(addr.relay_urls().count(), 0);
    }
}
