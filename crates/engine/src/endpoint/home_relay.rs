use iroh::{Endpoint, RelayUrl};

/// The home relay URL of an [`Endpoint`], if one has been assigned yet.
///
/// iroh assigns the home relay asynchronously after the endpoint connects to a
/// relay server, so this may return `None` right after bind. Serve prints it for
/// operator convenience (Page 06 v5 §1.1); access uses the config's
/// `relay_urls` to dial, so this is display-only.
pub fn home_relay(ep: &Endpoint) -> Option<RelayUrl> {
    ep.addr().relay_urls().next().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn home_relay_is_none_for_a_relayless_endpoint() {
        use iroh::endpoint::presets::Minimal;
        let ep = Endpoint::builder(Minimal)
            .relay_mode(iroh::endpoint::RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        assert!(home_relay(&ep).is_none());
    }
}
