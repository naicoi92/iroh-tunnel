//! Plain runtime settings handed to the engine by its host.
//!
//! No serde and no file knowledge: the host maps its own documents to these
//! values (including secret-key resolution) before binding.

use std::time::Duration;

use iroh::SecretKey;

use crate::engine::udp;

/// Tunnel protocol of a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Tcp,
    Udp,
}

impl Protocol {
    /// Lowercase protocol name (`"tcp"` / `"udp"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Tcp => "tcp",
            Protocol::Udp => "udp",
        }
    }
}

/// Relay auth token. `Debug` never prints the value (RS-10).
#[derive(Clone, PartialEq, Eq)]
pub struct RelayToken(String);

impl RelayToken {
    pub fn new(token: String) -> Self {
        Self(token)
    }

    /// The token text, for the relay connection only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for RelayToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RelayToken([redacted])")
    }
}

/// Node-level endpoint settings shared by both roles.
#[derive(Clone, Default)]
pub struct NodeSettings {
    /// `None` binds with a freshly generated ephemeral key.
    pub secret_key: Option<SecretKey>,
    pub relay_urls: Vec<String>,
    pub relay_token: Option<RelayToken>,
    pub max_concurrent_streams: Option<u32>,
}

impl std::fmt::Debug for NodeSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const REDACTED: &str = "[redacted]";
        f.debug_struct("NodeSettings")
            .field("secret_key", &self.secret_key.as_ref().map(|_| REDACTED))
            .field("relay_urls", &self.relay_urls)
            .field("relay_token", &self.relay_token)
            .field("max_concurrent_streams", &self.max_concurrent_streams)
            .finish()
    }
}

/// UDP resource limits with every default applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpLimits {
    pub idle_timeout: Duration,
    pub max_flows: usize,
    pub max_flows_per_peer: usize,
    pub queue_depth: usize,
}

impl UdpLimits {
    /// The effective limits, with the UDP-06 defaults for absent overrides.
    /// A per-peer cap never exceeds the service cap.
    pub fn from_overrides(
        idle_timeout: Option<Duration>,
        max_flows: Option<usize>,
        max_flows_per_peer: Option<usize>,
        queue_depth: Option<usize>,
    ) -> UdpLimits {
        let max_flows = max_flows.unwrap_or(udp::DEFAULT_MAX_FLOWS);
        UdpLimits {
            idle_timeout: idle_timeout.unwrap_or(udp::IDLE_TIMEOUT),
            max_flows,
            max_flows_per_peer: max_flows_per_peer
                .unwrap_or(udp::DEFAULT_MAX_FLOWS_PER_PEER)
                .min(max_flows),
            queue_depth: queue_depth.unwrap_or(udp::DEFAULT_QUEUE_DEPTH),
        }
    }
}

impl Default for UdpLimits {
    fn default() -> Self {
        Self::from_overrides(None, None, None, None)
    }
}

/// One published serve service.
#[derive(Debug, Clone)]
pub struct ServeServiceSettings {
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub udp: UdpLimits,
}

/// Everything the serve role needs to bind and run.
#[derive(Debug, Clone, Default)]
pub struct ServeSettings {
    pub node: NodeSettings,
    pub services: Vec<ServeServiceSettings>,
}

/// One exposed access service.
#[derive(Debug, Clone)]
pub struct AccessServiceSettings {
    pub name: String,
    /// Raw serve-peer id; parsed at the existing runtime stage.
    pub node_id: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub relay_urls: Vec<String>,
    pub udp: UdpLimits,
}

/// Everything the access role needs to bind and run.
#[derive(Debug, Clone, Default)]
pub struct AccessSettings {
    pub node: NodeSettings,
    pub services: Vec<AccessServiceSettings>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_token_debug_is_redacted() {
        let token = RelayToken::new("sekrit".into());
        assert_eq!(format!("{token:?}"), "RelayToken([redacted])");
        assert_eq!(token.expose(), "sekrit");
        let node = NodeSettings {
            secret_key: Some(SecretKey::generate()),
            relay_token: Some(token),
            ..Default::default()
        };
        let shown = format!("{node:?}");
        assert!(!shown.contains("sekrit"), "{shown}");
        assert!(shown.contains("[redacted]"));
    }

    #[test]
    fn from_overrides_applies_defaults_and_clamps_per_peer() {
        let defaults = UdpLimits::from_overrides(None, None, None, None);
        assert_eq!(defaults.idle_timeout, udp::IDLE_TIMEOUT);
        assert_eq!(defaults.max_flows, udp::DEFAULT_MAX_FLOWS);
        assert_eq!(defaults.max_flows_per_peer, udp::DEFAULT_MAX_FLOWS_PER_PEER);
        assert_eq!(defaults.queue_depth, udp::DEFAULT_QUEUE_DEPTH);
        let clamped =
            UdpLimits::from_overrides(Some(Duration::from_secs(5)), Some(8), None, Some(2));
        assert_eq!(clamped.idle_timeout, Duration::from_secs(5));
        assert_eq!(clamped.max_flows_per_peer, 8);
        assert_eq!(clamped.queue_depth, 2);
    }

    #[test]
    fn protocol_as_str_is_lowercase() {
        assert_eq!(Protocol::Tcp.as_str(), "tcp");
        assert_eq!(Protocol::Udp.as_str(), "udp");
    }
}
