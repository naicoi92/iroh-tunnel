//! Mapping from a config document to the engine settings of its role.

use anyhow::Result;
use iroh_tunnel_engine::settings::{
    AccessServiceSettings, AccessSettings, NodeSettings, RelayToken, ServeServiceSettings,
    ServeSettings,
};

use super::dto::{AccessConfig, NodeConfig, ServeConfig};
use super::resolve_secret_key::resolve_secret_key;

impl NodeConfig {
    /// Engine node settings: resolves `secret_key` (an empty key stays
    /// `None`, i.e. ephemeral) and wraps `relay_token` in a redacting type.
    fn node_settings(&self) -> Result<NodeSettings> {
        let secret_key = if self.secret_key.is_empty() {
            None
        } else {
            Some(resolve_secret_key(&self.secret_key)?.0)
        };
        Ok(NodeSettings {
            secret_key,
            relay_urls: self.relay_urls.clone(),
            relay_token: self.relay_token.clone().map(RelayToken::new),
            max_concurrent_streams: self.max_concurrent_streams,
        })
    }
}

/// Map a serve document to engine settings, resolving its secret key.
///
/// Fails like [`resolve_secret_key`] does, before anything binds.
pub fn serve_settings(cfg: &ServeConfig) -> Result<ServeSettings> {
    Ok(ServeSettings {
        node: cfg.node.node_settings()?,
        services: cfg
            .services
            .iter()
            .map(|svc| ServeServiceSettings {
                name: svc.name.clone(),
                protocol: svc.protocol,
                host: svc.host.clone(),
                port: svc.port,
                udp: svc.udp.resolve(),
            })
            .collect(),
    })
}

/// Map an access document to engine settings, resolving its secret key.
///
/// Fails like [`resolve_secret_key`] does, before anything binds.
pub fn access_settings(cfg: &AccessConfig) -> Result<AccessSettings> {
    Ok(AccessSettings {
        node: cfg.node.node_settings()?,
        services: cfg
            .services
            .iter()
            .map(|svc| AccessServiceSettings {
                name: svc.name.clone(),
                node_id: svc.node_id.clone(),
                protocol: svc.protocol,
                host: svc.host.clone(),
                port: svc.port,
                relay_urls: svc.relay_urls.clone(),
                udp: svc.udp.resolve(),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::encode_secret_key::encode_secret_key;
    use super::*;
    use iroh::SecretKey;
    use iroh_tunnel_engine::settings::Protocol;

    #[test]
    fn serve_settings_resolves_key_wraps_token_and_applies_udp_defaults() {
        let key = SecretKey::generate();
        let cfg: ServeConfig = toml::from_str(&format!(
            "[node]\nsecret_key = \"{}\"\nrelay_token = \"tok\"\n\n[[services]]\nname = \"dns\"\nprotocol = \"udp\"\nport = 53\nmax_flows = 8\n",
            encode_secret_key(&key)
        ))
        .unwrap();
        let settings = serve_settings(&cfg).unwrap();
        assert_eq!(settings.node.secret_key.unwrap().public(), key.public());
        assert_eq!(settings.node.relay_token.unwrap().expose(), "tok");
        assert_eq!(settings.services[0].protocol, Protocol::Udp);
        assert_eq!(settings.services[0].udp.max_flows, 8);
    }

    #[test]
    fn settings_mapping_rejects_an_invalid_secret_key_before_bind() {
        let mut cfg = AccessConfig::default();
        cfg.node.secret_key = "not!valid!base64!!".to_string();
        let err = access_settings(&cfg).unwrap_err();
        assert!(format!("{err:#}").contains("invalid secret_key"));
    }
}
