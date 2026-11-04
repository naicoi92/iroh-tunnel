use iroh::RelayUrl;

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
