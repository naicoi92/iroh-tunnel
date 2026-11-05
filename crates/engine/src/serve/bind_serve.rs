use anyhow::Result;
use iroh::endpoint::Endpoint;

use crate::endpoint::create_endpoint_with_key;
use crate::proto;
use crate::settings::ServeSettings;

/// Build an [`Endpoint`] for the **serve** role.
///
/// The secret key is `s.node.secret_key` (the host resolves and persists it);
/// `None` binds an ephemeral key.
///
/// Every service's ALPN is registered up front: in iroh 1.0 ALPNs are
/// registered on the endpoint at build time (not filtered per-`accept`).
pub async fn bind_serve(s: &ServeSettings) -> Result<Endpoint> {
    let alpns: Vec<Vec<u8>> = s
        .services
        .iter()
        .map(|svc| proto::alpn_for(&svc.name))
        .collect();
    create_endpoint_with_key(&s.node, &s.node.relay_urls, &alpns).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::node_id_string;
    use crate::settings::NodeSettings;

    #[tokio::test]
    async fn serve_endpoint_with_same_key_has_same_node_id() {
        let key = iroh::SecretKey::generate();
        let settings = ServeSettings {
            node: NodeSettings {
                secret_key: Some(key.clone()),
                ..Default::default()
            },
            services: Vec::new(),
        };
        let ep = bind_serve(&settings).await.unwrap();
        assert_eq!(node_id_string(&ep), key.public().to_string());
    }
}
