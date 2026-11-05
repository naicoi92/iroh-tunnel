use anyhow::Result;
use iroh::endpoint::Endpoint;

use crate::endpoint::create_endpoint_with_key;
use crate::settings::AccessSettings;

/// Build an [`Endpoint`] for the **access** role.
///
/// Access only dials out, so no ALPNs are registered. The endpoint is
/// connected to the UNION of node + per-service relay URLs (issue #54) so
/// every service's dial has a live relay transport for its URLs (a relay only
/// carries traffic for peers connected to it). Per-service dials still attach
/// only that service's own URLs.
pub async fn bind_access(s: &AccessSettings) -> Result<Endpoint> {
    let mut relays = s.node.relay_urls.clone();
    for svc in &s.services {
        for url in &svc.relay_urls {
            if !relays.contains(url) {
                relays.push(url.clone());
            }
        }
    }
    create_endpoint_with_key(&s.node, &relays, &[]).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::node_id_string;
    use crate::settings::NodeSettings;

    #[tokio::test]
    async fn access_endpoint_with_same_key_has_same_node_id() {
        // A pinned access secret_key must yield a stable NodeId across endpoints
        // (the whole point of pinning access identity).
        let key = iroh::SecretKey::generate();
        let settings = AccessSettings {
            node: NodeSettings {
                secret_key: Some(key.clone()),
                ..Default::default()
            },
            services: Vec::new(),
        };
        let ep = bind_access(&settings).await.unwrap();
        assert_eq!(node_id_string(&ep), key.public().to_string());
    }
}
