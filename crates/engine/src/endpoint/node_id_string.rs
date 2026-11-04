use iroh::Endpoint;

/// The node id (public key) of an [`Endpoint`], as its base32 string form.
pub fn node_id_string(ep: &Endpoint) -> String {
    ep.secret_key().public().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::bind_access;
    use crate::settings::AccessSettings;

    #[tokio::test]
    async fn access_endpoint_binds_and_has_node_id() {
        // ephemeral key, default (n0) relays
        let ep = bind_access(&AccessSettings::default()).await.unwrap();
        let id = node_id_string(&ep);
        // iroh 1.0's PublicKey Display is lowercase hex (32 bytes => 64 chars).
        // (Parsing accepts both hex and base32, but Display emits hex.)
        assert_eq!(id.len(), 64, "node_id string: {id}");
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
