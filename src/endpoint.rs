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
use iroh::endpoint::{Endpoint, RelayMode};
use iroh::RelayUrl;

use crate::config::{self, NodeConfig};

/// Build an [`Endpoint`] for the **serve** role.
///
/// The secret key is resolved from `node.secret_key`; if it was empty a fresh
/// one is generated and the caller is expected to persist it (the config layer
/// handles that via [`config::ServeConfig::resolve_and_save_key`]).
pub async fn create_serve_endpoint(node: &NodeConfig) -> Result<Endpoint> {
    // resolve_secret_key returns (key, needs_save); serve callers persist via
    // ServeConfig::resolve_and_save_key, so the boolean is ignored here.
    let (key, _needs_save) = config::resolve_secret_key(&node.secret_key)?;
    create_endpoint_with_key(key, &node.relay_urls).await
}

/// Build an [`Endpoint`] for the **access** role.
///
/// Uses an ephemeral [`SecretKey`] that is never persisted.
pub async fn create_access_endpoint(node: &NodeConfig) -> Result<Endpoint> {
    let key = iroh::SecretKey::generate();
    create_endpoint_with_key(key, &node.relay_urls).await
}

async fn create_endpoint_with_key(
    key: iroh::SecretKey,
    relay_urls: &[String],
) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(Minimal).secret_key(key);

    builder = builder.relay_mode(relay_mode_from_urls(relay_urls)?);

    builder
        .bind()
        .await
        .context("failed to bind iroh endpoint")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_relay_urls_yields_default_mode() {
        let mode = relay_mode_from_urls(&[]).unwrap();
        assert!(matches!(mode, RelayMode::Default));
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
                assert!(collected.iter().any(|u| u == "https://use1-1.relay.n0.iroh.link./"));
                assert!(collected.iter().any(|u| u == "https://euw-1.relay.n0.iroh.link./"));
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
        };
        let ep = create_serve_endpoint(&node).await.unwrap();
        assert_eq!(node_id_string(&ep), key.public().to_string());
    }
}
