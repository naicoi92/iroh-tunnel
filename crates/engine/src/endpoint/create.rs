//! Private endpoint construction shared by `bind_serve` and `bind_access`.

use anyhow::{Context, Result};
use iroh::endpoint::presets::Minimal;
use iroh::endpoint::{Endpoint, QuicTransportConfig, RelayMode, VarInt};
#[cfg(feature = "test-utils")]
use iroh::tls::CaTlsConfig;
use iroh::{RelayMap, RelayUrl};

use std::time::Duration;

use crate::settings::{NodeSettings, RelayToken};

pub(crate) async fn create_endpoint_with_key(
    node: &NodeSettings,
    relay_urls: &[String],
    alpns: &[Vec<u8>],
) -> Result<Endpoint> {
    let key = node
        .secret_key
        .clone()
        .unwrap_or_else(iroh::SecretKey::generate);
    let mut builder = Endpoint::builder(Minimal)
        .secret_key(key)
        .transport_config(transport_config(node.max_concurrent_streams));

    builder = builder.relay_mode(relay_mode_from_urls(
        relay_urls,
        node.relay_token.as_ref().map(RelayToken::expose),
    )?);

    // Tests run the endpoint against an in-process relay with a self-signed
    // certificate (iroh test_utils); production keeps the platform verifier.
    #[cfg(feature = "test-utils")]
    {
        builder = builder.ca_tls_config(CaTlsConfig::insecure_skip_verify());
        if crate::testing::relay_only() {
            builder = builder.clear_ip_transports();
        }
    }

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
    #[cfg(feature = "test-utils")]
    if crate::testing::datagrams_disabled() {
        builder = builder.datagram_receive_buffer_size(None);
    }
    builder.build()
}

/// Translate the config `relay_urls` into a [`RelayMode`].
///
/// - Empty → [`RelayMode::Default`] (n0 public relays).
/// - Non-empty → [`RelayMode::custom`] with the first URL as home relay and
///   the rest as failover (relay servers are stateless, so any can serve a
///   peer; iroh advertises the home relay in the endpoint info).
///
/// `relay_token` (when Some) is applied to every relay in the map — see
/// [`NodeSettings::relay_token`] for the single-token semantics.
fn relay_mode_from_urls(relay_urls: &[String], relay_token: Option<&str>) -> Result<RelayMode> {
    if relay_urls.is_empty() {
        if relay_token.is_some() {
            tracing::warn!(
                "relay_token is set but relay_urls is empty — the token is unused \
                 while falling back to the n0 default relays"
            );
        }
        return Ok(RelayMode::Default);
    }
    let urls: Vec<RelayUrl> = relay_urls
        .iter()
        .map(|s| {
            s.parse::<RelayUrl>()
                .with_context(|| format!("invalid relay_url: {s}"))
        })
        .collect::<Result<_>>()?;
    let mut map = RelayMap::from_iter(urls);
    if let Some(token) = relay_token {
        // Applies the token to every relay in the map (both the home relay
        // and failovers): sent as `Authorization: Bearer` on the relay
        // WebSocket upgrade, ignored by relays without access control.
        map = map.with_auth_token(token);
    }
    Ok(RelayMode::Custom(map))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_relay_urls_yields_default_mode() {
        let mode = relay_mode_from_urls(&[], None).unwrap();
        assert!(matches!(mode, RelayMode::Default));
    }

    #[test]
    fn custom_relay_urls_parse_into_custom_mode() {
        let urls = vec![
            "https://use1-1.relay.n0.iroh.link.".to_string(),
            "https://euw-1.relay.n0.iroh.link.".to_string(),
        ];
        let mode = relay_mode_from_urls(&urls, None).unwrap();
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
    fn relay_token_is_applied_to_all_map_entries() {
        // with_auth_token must cover home + failover entries alike — the
        // relay transport for any of these URLs authenticates with it.
        let urls = vec![
            "https://relay-a.example".to_string(),
            "https://relay-b.example".to_string(),
        ];
        let mode = relay_mode_from_urls(&urls, Some("sekrit-token")).unwrap();
        match mode {
            RelayMode::Custom(map) => {
                for url in map.urls::<Vec<RelayUrl>>() {
                    let cfg = map.get(&url).expect("map entry present");
                    assert_eq!(
                        cfg.auth_token.as_deref(),
                        Some("sekrit-token"),
                        "token missing on {url}"
                    );
                }
            }
            other => panic!("expected Custom, got {other:?}"),
        }
    }

    #[test]
    fn invalid_relay_url_errors() {
        let urls = vec!["not a url".to_string()];
        let err = relay_mode_from_urls(&urls, None).unwrap_err();
        assert!(format!("{err:#}").contains("invalid relay_url"));
    }
}
