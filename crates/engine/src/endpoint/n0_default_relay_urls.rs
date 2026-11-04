use iroh::RelayUrl;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
