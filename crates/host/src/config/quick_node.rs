//! Pieces shared by the quick-mode config builders.

use std::path::Path;

use anyhow::Result;

use super::read_key_file::read_key_file;
use super::NodeConfig;

/// Host a quick-mode address binds or targets when none is given: loopback.
const QUICK_DEFAULT_HOST: &str = "127.0.0.1";

/// The `[node]` table of a quick run: the key from `key_file` (empty means an
/// ephemeral identity) and the optional relay.
pub(super) fn quick_node(relay_url: Option<&str>, key_file: Option<&Path>) -> Result<NodeConfig> {
    Ok(NodeConfig {
        secret_key: read_key_file(key_file)?,
        relay_urls: relay_url.map(str::to_owned).into_iter().collect(),
        ..Default::default()
    })
}

/// Host of a quick-mode address; `None` means loopback.
pub(super) fn quick_host(host: Option<&str>) -> String {
    host.unwrap_or(QUICK_DEFAULT_HOST).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_host_means_loopback() {
        assert_eq!(quick_host(None), "127.0.0.1");
        assert_eq!(quick_host(Some("::1")), "::1");
    }

    #[test]
    fn the_node_carries_the_relay_and_no_key_without_a_file() {
        let node = quick_node(Some("https://relay.example"), None).unwrap();
        assert_eq!(node.secret_key, "");
        assert_eq!(node.relay_urls, vec!["https://relay.example".to_owned()]);
        assert!(quick_node(None, None).unwrap().relay_urls.is_empty());
    }
}
