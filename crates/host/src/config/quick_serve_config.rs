//! The in-memory serve config of a quick-mode run.

use anyhow::Result;

use super::quick_node::{quick_host, quick_node};
use super::{QuickServe, ServeConfig, ServeService};

/// Build the one-service serve config of a quick run from plain inputs.
///
/// Nothing is persisted; the schema itself is checked by
/// [`crate::runtime::run_quick_serve`].
///
/// # Errors
///
/// [`crate::error::CliError::Config`] when `req.key_file` cannot be read, is
/// empty or does not hold a base64 32-byte secret key.
pub fn quick_serve_config(req: &QuickServe) -> Result<ServeConfig> {
    Ok(ServeConfig {
        node: quick_node(req.relay_url.as_deref(), req.key_file.as_deref())?,
        services: vec![ServeService {
            name: req.name.clone(),
            protocol: req.protocol,
            host: quick_host(req.host.as_deref()),
            port: req.port,
            udp: Default::default(),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Protocol;
    use crate::error::CliError;

    fn request() -> QuickServe {
        QuickServe {
            name: "echo".into(),
            protocol: Protocol::Udp,
            host: None,
            port: 7000,
            relay_url: Some("https://relay.example".into()),
            key_file: None,
        }
    }

    #[test]
    fn builds_one_service_on_loopback_with_an_ephemeral_identity() {
        let cfg = quick_serve_config(&request()).unwrap();
        assert_eq!(cfg.node.secret_key, "");
        assert_eq!(
            cfg.node.relay_urls,
            vec!["https://relay.example".to_owned()]
        );
        assert_eq!(
            cfg.services,
            vec![ServeService {
                name: "echo".into(),
                protocol: Protocol::Udp,
                host: "127.0.0.1".into(),
                port: 7000,
                udp: Default::default(),
            }]
        );
    }

    #[test]
    fn an_unusable_key_file_is_a_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let req = QuickServe {
            key_file: Some(dir.path().join("missing.key")),
            ..request()
        };
        let err = quick_serve_config(&req).unwrap_err();
        assert!(
            matches!(err.downcast_ref::<CliError>(), Some(CliError::Config(m)) if m.starts_with("--key-file")),
            "{err:#}"
        );
    }
}
