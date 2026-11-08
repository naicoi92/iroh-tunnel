//! The in-memory access config of a quick-mode run.

use anyhow::Result;

use super::quick_node::{quick_host, quick_node};
use super::{AccessConfig, AccessService, QuickAccess};
use crate::error::CliError;

/// Build the one-service access config of a quick run from plain inputs.
///
/// The peer node id is checked first, before the key file is read. Nothing
/// is persisted; the schema itself is checked by
/// [`crate::runtime::run_quick_access`].
///
/// # Errors
///
/// [`CliError::Config`] when `req.node_id` is not an endpoint id, or when
/// `req.key_file` cannot be read, is empty or does not hold a base64 32-byte
/// secret key.
pub fn quick_access_config(req: &QuickAccess) -> Result<AccessConfig> {
    req.node_id
        .parse::<iroh::EndpointId>()
        .map_err(|e| CliError::Config(format!("access quick mode: invalid --node-id: {e}")))?;
    Ok(AccessConfig {
        node: quick_node(req.relay_url.as_deref(), req.key_file.as_deref())?,
        services: vec![AccessService {
            name: req.name.clone(),
            node_id: req.node_id.clone(),
            protocol: req.protocol,
            host: quick_host(req.host.as_deref()),
            port: req.port,
            relay_urls: vec![],
            udp: Default::default(),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Protocol;

    fn peer() -> String {
        iroh::SecretKey::from_bytes(&[9; 32]).public().to_string()
    }

    fn request() -> QuickAccess {
        QuickAccess {
            name: "echo".into(),
            protocol: Protocol::Tcp,
            host: Some("::1".into()),
            port: 7001,
            node_id: peer(),
            relay_url: None,
            key_file: None,
        }
    }

    fn config_error(req: &QuickAccess) -> String {
        let err = quick_access_config(req).unwrap_err();
        match err.downcast_ref::<CliError>() {
            Some(CliError::Config(m)) => m.clone(),
            other => panic!("expected a config error, got {other:?}: {err:#}"),
        }
    }

    #[test]
    fn builds_one_service_for_the_peer() {
        let cfg = quick_access_config(&request()).unwrap();
        assert_eq!(cfg.node.secret_key, "");
        assert!(cfg.node.relay_urls.is_empty());
        assert_eq!(
            cfg.services,
            vec![AccessService {
                name: "echo".into(),
                node_id: peer(),
                protocol: Protocol::Tcp,
                host: "::1".into(),
                port: 7001,
                relay_urls: vec![],
                udp: Default::default(),
            }]
        );
    }

    #[test]
    fn an_invalid_node_id_is_a_config_error() {
        let req = QuickAccess {
            node_id: "invalid".into(),
            ..request()
        };
        let msg = config_error(&req);
        assert!(
            msg.starts_with("access quick mode: invalid --node-id: "),
            "{msg}"
        );
    }

    #[test]
    fn the_node_id_is_checked_before_the_key_file() {
        let dir = tempfile::tempdir().unwrap();
        let req = QuickAccess {
            node_id: "invalid".into(),
            key_file: Some(dir.path().join("missing.key")),
            ..request()
        };
        assert!(config_error(&req).contains("--node-id"));
    }

    #[test]
    fn an_unusable_key_file_is_a_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let req = QuickAccess {
            key_file: Some(dir.path().join("missing.key")),
            ..request()
        };
        assert!(config_error(&req).starts_with("--key-file"));
    }
}
