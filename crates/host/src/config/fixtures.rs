//! Shared fixtures of the config operations' unit tests.

use std::path::Path;

use super::types::{AddService, SetService};

pub(crate) const VALID_SERVE: &str =
    "[[services]]\nname = \"web\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 8080\n";

pub(crate) fn add_web(config: &Path) -> AddService {
    AddService {
        config: Some(config.to_path_buf()),
        name: "web".into(),
        protocol: "tcp".into(),
        host: "127.0.0.1".into(),
        port: 8080,
        node_id: None,
    }
}

pub(crate) fn set_web(config: &Path) -> SetService {
    SetService {
        config: Some(config.to_path_buf()),
        name: "web".into(),
        host: None,
        port: Some(8080),
        protocol: None,
        node_id: None,
    }
}

pub(crate) const KEYED: &str = "[node]\nsecret_key = \"\"\n";

pub(crate) fn write_keyed(path: &Path) -> String {
    let key = iroh::SecretKey::generate();
    let text = format!(
        "[node]\nsecret_key = \"{}\"\n",
        crate::config::encode_secret_key(&key)
    );
    std::fs::write(path, &text).unwrap();
    key.public().to_string()
}
