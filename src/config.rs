//! TOML config schema, load/save, SecretKey management, and validation.
//!
//! Implements T-02.1 (structs), T-02.2 (load/save), T-02.3 (SecretKey),
//! T-02.4 (validation). Based on Page 05 v3 §2–§4.
//
// Methods here are consumed by the serve/access/config_cmd/service handlers
// (T-06/T-07/T-11/T-12); until then they're flagged dead code by the binary
// crate's single-crate layout.
#![allow(dead_code)]

use std::collections::HashSet;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use data_encoding::BASE64;
use iroh::SecretKey;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::engine::settings::{
    AccessServiceSettings, AccessSettings, NodeSettings, RelayToken, ServeServiceSettings,
    ServeSettings,
};

// Service names are lowercased dns-label-like identifiers (ALPN-safe).
fn name_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-z0-9-]+$").expect("valid static regex"))
}

/// Maximum service-name length: ALPN must stay ≤ 255 bytes, minus the
/// [`crate::proto`] prefix (see T-03). 63 keeps names dns-label friendly.
const MAX_NAME_LEN: usize = 63;

/// Valid node_id (PublicKey) string lengths.
///
/// iroh's [`PublicKey::Display`] emits lowercase hex (64 chars), and
/// [`PublicKey::from_str`] accepts either hex (64) or base32 (52). We accept
/// both so a node id copied from `serve`'s output (hex) round-trips into an
/// access config.
///
/// [`PublicKey::Display`]: iroh::PublicKey
/// [`PublicKey::from_str`]: iroh::PublicKey
const NODE_ID_LENS: [usize; 2] = [52, 64];

// ---------------------------------------------------------------------------
// T-02.1: structs
// ---------------------------------------------------------------------------

fn default_host() -> String {
    "127.0.0.1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ServeConfig {
    #[serde(default)]
    pub node: NodeConfig,
    #[serde(default)]
    pub services: Vec<ServeService>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AccessConfig {
    /// Node-level settings shared by all access services.
    ///
    /// `relay_urls` is the connectivity fallback for dialing peers (Page 05 v3
    /// §6). `secret_key` pins this access node's identity: if empty, a fresh
    /// key is generated on the first `run` and persisted to the config, so the
    /// access NodeId is stable across restarts (mirroring the serve role).
    #[serde(default)]
    pub node: NodeConfig,
    #[serde(default)]
    pub services: Vec<AccessService>,
}

#[derive(Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct NodeConfig {
    #[serde(default)]
    pub secret_key: String,
    #[serde(default)]
    pub relay_urls: Vec<String>,
    /// Auth token for relays that require one — the `relay_token` your
    /// self-hosted relay checks via `access.shared_token`
    /// (docs/self-hosted-relay.md). Sent as `Authorization: Bearer` on the
    /// relay connection; ignored by relays without access control.
    ///
    /// One token for every configured relay: mixing relays with different
    /// tokens is not supported. Only meaningful together with non-empty
    /// `relay_urls` (the n0 default fallback relays need no token).
    #[serde(default)]
    pub relay_token: Option<String>,
    /// Concurrent bidirectional-stream budget per QUIC connection (both
    /// roles, since 0.2.0). `None`/absent keeps noq's default (100).
    ///
    /// Headroom tuning, not a requirement: set it only after measuring the
    /// real concurrent-channel count of the workload. Worst-case buffer
    /// memory scales with `max_concurrent_streams × stream_receive_window`;
    /// when the budget is exhausted a new channel's `open_bi` is
    /// flow-control blocked until another stream closes.
    #[serde(default)]
    pub max_concurrent_streams: Option<u32>,
}

/// Secret values never reach `Debug` output (RS-10). An empty `secret_key` is
/// not a secret and prints as `""`, so an unset key stays visible.
impl std::fmt::Debug for NodeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const REDACTED: &str = "[redacted]";
        let secret_key = if self.secret_key.is_empty() {
            ""
        } else {
            REDACTED
        };
        f.debug_struct("NodeConfig")
            .field("secret_key", &secret_key)
            .field("relay_urls", &self.relay_urls)
            .field("relay_token", &self.relay_token.as_ref().map(|_| REDACTED))
            .field("max_concurrent_streams", &self.max_concurrent_streams)
            .finish()
    }
}

impl NodeConfig {
    /// Shared node-level validation. Both roles call this from their
    /// `RoleDoc::validate`.
    pub fn validate(&self) -> Result<()> {
        if let Some(max) = self.max_concurrent_streams {
            if max == 0 {
                anyhow::bail!("invalid max_concurrent_streams {max}: must be >= 1");
            }
        }
        if let Some(token) = &self.relay_token {
            if token.trim().is_empty() {
                anyhow::bail!("invalid relay_token: must not be empty or whitespace");
            }
            if token.chars().any(char::is_whitespace) {
                anyhow::bail!("invalid relay_token: must not contain whitespace");
            }
        }
        Ok(())
    }
}

/// Largest `idle_timeout` accepted, in seconds (one day).
const MAX_UDP_IDLE_TIMEOUT_SECS: u64 = 86_400;
/// Largest `max_flows` / `max_flows_per_peer` accepted. Each flow holds a
/// socket, so this is far above any descriptor limit; it only rejects absurd
/// values before they reach an allocator.
const MAX_UDP_FLOWS: usize = 1_000_000;
/// Largest `queue_depth` accepted. A bounded channel panics when asked for a
/// huge capacity, so the value is checked here, where a config error is
/// reported, instead of inside a connection task.
const MAX_UDP_QUEUE_DEPTH: usize = 65_536;

/// Optional UDP resource settings on a service table (UDP-06).
///
/// Every field is optional; absent fields use the defaults in
/// [`UdpSettings::resolve`]. All defaults are untested estimates, not
/// benchmarked values. Setting any of them on a non-UDP service is rejected.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UdpSettings {
    /// Seconds without traffic in either direction before a flow ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout: Option<u64>,
    /// Most flows the service holds at once; quarantined flows count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_flows: Option<usize>,
    /// Most flows one peer may hold (Serve only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_flows_per_peer: Option<usize>,
    /// Datagrams queued per flow before the newest is dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_depth: Option<usize>,
}

pub use crate::engine::settings::UdpLimits;

impl UdpSettings {
    /// The effective limits, with the UDP-06 defaults for absent fields. A
    /// per-peer cap never exceeds the service cap.
    pub fn resolve(&self) -> UdpLimits {
        UdpLimits::from_overrides(
            self.idle_timeout.map(std::time::Duration::from_secs),
            self.max_flows,
            self.max_flows_per_peer,
            self.queue_depth,
        )
    }

    fn is_unset(&self) -> bool {
        *self == Self::default()
    }

    /// Reject settings that cannot work. `per_peer_allowed` is false for the
    /// Access role, which has no peers to cap.
    fn validate(&self, service: &str, protocol: Protocol, per_peer_allowed: bool) -> Result<()> {
        if protocol != Protocol::Udp {
            if !self.is_unset() {
                anyhow::bail!(
                    "service '{service}': idle_timeout, max_flows, max_flows_per_peer and \
                     queue_depth apply only to protocol = \"udp\""
                );
            }
            return Ok(());
        }
        for (field, value, max) in [
            ("idle_timeout", self.idle_timeout, MAX_UDP_IDLE_TIMEOUT_SECS),
            (
                "max_flows",
                self.max_flows.map(|v| v as u64),
                MAX_UDP_FLOWS as u64,
            ),
            (
                "max_flows_per_peer",
                self.max_flows_per_peer.map(|v| v as u64),
                MAX_UDP_FLOWS as u64,
            ),
            (
                "queue_depth",
                self.queue_depth.map(|v| v as u64),
                MAX_UDP_QUEUE_DEPTH as u64,
            ),
        ] {
            match value {
                Some(0) => anyhow::bail!("service '{service}': invalid {field} 0: must be >= 1"),
                Some(v) if v > max => {
                    anyhow::bail!("service '{service}': invalid {field} {v}: must be <= {max}")
                }
                _ => {}
            }
        }
        if !per_peer_allowed {
            // Access has no peers to cap and no per-flow queue: it forwards
            // each datagram straight to the connection.
            for (field, set) in [
                ("max_flows_per_peer", self.max_flows_per_peer.is_some()),
                ("queue_depth", self.queue_depth.is_some()),
            ] {
                if set {
                    anyhow::bail!("service '{service}': {field} is not valid on an access service");
                }
            }
        }
        // Only an explicit contradiction is an error; a default per-peer cap
        // above an explicit `max_flows` is clamped by `resolve`.
        if let (Some(per_peer), Some(total)) = (self.max_flows_per_peer, self.max_flows) {
            if per_peer > total {
                anyhow::bail!(
                    "service '{service}': max_flows_per_peer {per_peer} exceeds max_flows {total}"
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServeService {
    pub name: String,
    #[serde(with = "protocol_serde")]
    pub protocol: Protocol,
    #[serde(default = "default_host")]
    pub host: String,
    pub port: u16,
    #[serde(flatten)]
    pub udp: UdpSettings,
}

/// Remote service mapping. Legacy `multiplex` is an ignored unknown field;
/// TCP always reuses a connection and serialization drops the legacy setting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessService {
    pub name: String,
    pub node_id: String,
    #[serde(with = "protocol_serde")]
    pub protocol: Protocol,
    #[serde(default = "default_host")]
    pub host: String,
    pub port: u16,
    /// Per-service relay override (access role): the relay URLs used to dial
    /// THIS service's serve peer, instead of `[node] relay_urls`.
    ///
    /// Empty/absent (default) → the node-level `relay_urls` apply. The
    /// endpoint stays connected to the union of node + service relays (with
    /// the node `relay_token`), while this service's dials attach only the
    /// URLs listed here.
    ///
    /// Serve-side note: the serve peer must itself be reachable through
    /// these relays (its own `relay_urls` should include them) — a relay
    /// only carries traffic for peers connected to it.
    #[serde(default)]
    pub relay_urls: Vec<String>,
    #[serde(flatten)]
    pub udp: UdpSettings,
}

pub use crate::engine::settings::Protocol;

/// Serde form of the engine [`Protocol`]: lowercase `"tcp"` / `"udp"`.
mod protocol_serde {
    use super::Protocol;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(p: &Protocol, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(p.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Protocol, D::Error> {
        match String::deserialize(d)?.as_str() {
            "tcp" => Ok(Protocol::Tcp),
            "udp" => Ok(Protocol::Udp),
            other => Err(serde::de::Error::unknown_variant(other, &["tcp", "udp"])),
        }
    }
}

// ---------------------------------------------------------------------------
// T-02.2 / T-02.3: shared load / save / resolve_and_save_key via RoleDoc
// ---------------------------------------------------------------------------

/// A role's TOML config document: schema + validation + the bits every role
/// shares (load, save, secret-key resolution).
///
/// [`ServeConfig`] and [`AccessConfig`] both `impl RoleDoc`. The default
/// methods below collapse what used to be three byte-identical method pairs
/// (`load`, `save`, `resolve_and_save_key`) into a single definition — only
/// [`RoleDoc::validate`] differs per role, so that stays the one required
/// method.
///
/// The trait is sealed to this crate (`pub(crate)` boundary) because it's an
/// internal abstraction over two known implementers, not a public extension
/// point.
pub(crate) trait RoleDoc: serde::Serialize + serde::de::DeserializeOwned {
    /// Validate the parsed document; called automatically at the end of
    /// [`RoleDoc::load`]. Each role implements its own checks.
    fn validate(&self) -> Result<()>;

    /// The node-level section that carries `secret_key` + `relay_urls`. Used
    /// by [`RoleDoc::resolve_and_save_key`] to read and persist the key.
    fn node_mut(&mut self) -> &mut NodeConfig;

    /// Load and validate a role config from a TOML file.
    fn load(path: &Path) -> Result<Self>
    where
        Self: Sized,
    {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        Self::from_toml(&content, path)
    }

    /// Parse and validate config text already read from `path` (used when the
    /// bytes come from a verified handle rather than a path lookup).
    fn from_toml(content: &str, path: &Path) -> Result<Self>
    where
        Self: Sized,
    {
        // `toml` errors quote the offending source line, which can hold a
        // secret; report the message and line only (RS-10).
        let cfg: Self = toml::from_str(content).map_err(|e| {
            anyhow::anyhow!(
                "failed to parse config: {}{}: {}",
                path.display(),
                crate::config_store::describe_location(content, e.span()),
                e.message()
            )
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Serialize and write the config to disk: under the writer lock,
    /// atomically and owner-only (CFG-04).
    fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).context("failed to serialize config")?;
        let path = crate::config_store::resolve_target(path);
        let _lock = crate::config_store::ConfigLock::acquire(&path)?;
        crate::config_store::write_atomic(&path, content.as_bytes())
    }

    /// Resolve the node secret key; if it was just generated, persist it back
    /// into the config file. Returns the resolved key.
    ///
    /// The key is stored by editing the file in place, owner-only, under the
    /// writer lock; if another process stored a key first, that key is
    /// adopted. The key value itself is never logged (NFR-05).
    fn resolve_and_save_key(&mut self, path: &Path) -> Result<SecretKey>
    where
        Self: Sized,
    {
        let (key, needs_save) = resolve_secret_key(&self.node_mut().secret_key)?;
        if !needs_save {
            return Ok(key);
        }
        tracing::warn!("secret_key empty, generated new key, saving to config");
        let stored =
            crate::config_store::persist_generated_key::<Self>(path, &encode_secret_key(&key))?;
        self.node_mut().secret_key = stored;
        Ok(resolve_secret_key(&self.node_mut().secret_key)?.0)
    }
}

impl ServeConfig {
    /// Load and validate a serve config from a TOML file.
    ///
    /// Thin delegate to the [`RoleDoc`] default — kept as an inherent method
    /// so existing call sites (`ServeConfig::load(path)`) read naturally.
    pub fn load(path: &Path) -> Result<Self> {
        RoleDoc::load(path)
    }

    /// Serialize and write the config to disk.
    pub fn save(&self, path: &Path) -> Result<()> {
        RoleDoc::save(self, path)
    }

    /// Resolve the node secret key; if it was just generated, persist it back
    /// into the config file. Returns the resolved key.
    pub fn resolve_and_save_key(&mut self, path: &Path) -> Result<SecretKey> {
        RoleDoc::resolve_and_save_key(self, path)
    }
}

impl AccessConfig {
    /// Load and validate an access config from a TOML file.
    ///
    /// Thin delegate to the [`RoleDoc`] default.
    pub fn load(path: &Path) -> Result<Self> {
        RoleDoc::load(path)
    }

    /// Serialize and write the config to disk.
    pub fn save(&self, path: &Path) -> Result<()> {
        RoleDoc::save(self, path)
    }

    /// Resolve the node secret key; if it was just generated, persist it back
    /// into the config file. Returns the resolved key.
    ///
    /// Pinned access identity: an empty `secret_key` is generated once and
    /// persisted, so the access NodeId is stable across restarts.
    pub fn resolve_and_save_key(&mut self, path: &Path) -> Result<SecretKey> {
        RoleDoc::resolve_and_save_key(self, path)
    }
}

// ---------------------------------------------------------------------------
// Engine settings mapping
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// T-02.3: SecretKey management
// ---------------------------------------------------------------------------

/// Resolve a secret key from its base64 config representation.
///
/// - Empty string: generate a fresh key, return `(key, true)` so the caller
///   knows it should persist the new key.
/// - Non-empty: base64-decode and parse into a [`SecretKey`], returning
///   `(key, false)`.
pub fn resolve_secret_key(s: &str) -> Result<(SecretKey, bool)> {
    if s.is_empty() {
        Ok((SecretKey::generate(), true))
    } else {
        let bytes = BASE64
            .decode(s.as_bytes())
            .context("invalid secret_key: not valid base64")?;
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid secret_key: expected 32 bytes"))?;
        Ok((SecretKey::from_bytes(&arr), false))
    }
}

/// Encode a [`SecretKey`] to a base64 string for config storage.
pub fn encode_secret_key(key: &SecretKey) -> String {
    BASE64.encode(&key.to_bytes())
}

// ---------------------------------------------------------------------------
// T-02.4: validation
// ---------------------------------------------------------------------------

fn validate_name(name: &str) -> Result<()> {
    if !name_regex().is_match(name) {
        anyhow::bail!("invalid service name '{name}': must match ^[a-z0-9-]+$");
    }
    if name.len() > MAX_NAME_LEN {
        anyhow::bail!("invalid service name '{name}': max {MAX_NAME_LEN} bytes (ALPN limit)");
    }
    Ok(())
}

fn validate_port(port: u16) -> Result<()> {
    if port == 0 {
        anyhow::bail!("invalid port: must be 1-65535");
    }
    Ok(())
}

impl ServeConfig {
    /// Validate node + services (names, ports, relay URLs, duplicates).
    pub fn validate(&self) -> Result<()> {
        let mut seen: HashSet<&str> = HashSet::new();
        for svc in &self.services {
            validate_name(&svc.name)?;
            if !seen.insert(svc.name.as_str()) {
                anyhow::bail!("duplicate service name: '{}'", svc.name);
            }
            validate_port(svc.port)?;
            svc.udp.validate(&svc.name, svc.protocol, true)?;
        }
        for url in &self.node.relay_urls {
            if !url.starts_with("https://") {
                anyhow::bail!("invalid relay_url '{url}': must be https://");
            }
        }
        self.node.validate()?;
        Ok(())
    }
}

impl RoleDoc for ServeConfig {
    fn validate(&self) -> Result<()> {
        // Delegate to the inherent method so callers using either
        // ServeConfig::validate or RoleDoc::validate get the same behavior.
        ServeConfig::validate(self)
    }

    fn node_mut(&mut self) -> &mut NodeConfig {
        &mut self.node
    }
}

impl AccessConfig {
    /// Validate node relay_urls + services (names, ports, node_id format,
    /// duplicates).
    pub fn validate(&self) -> Result<()> {
        for url in &self.node.relay_urls {
            if !url.starts_with("https://") {
                anyhow::bail!("invalid relay_url '{url}': must be https://");
            }
        }
        self.node.validate()?;
        let mut seen: HashSet<&str> = HashSet::new();
        for svc in &self.services {
            validate_name(&svc.name)?;
            if !seen.insert(svc.name.as_str()) {
                anyhow::bail!("duplicate service name: '{}'", svc.name);
            }
            if !NODE_ID_LENS.contains(&svc.node_id.len()) {
                anyhow::bail!(
                    "invalid node_id '{}': must be 52 (base32) or 64 (hex) chars",
                    svc.node_id
                );
            }
            validate_port(svc.port)?;
            svc.udp.validate(&svc.name, svc.protocol, false)?;
            for url in &svc.relay_urls {
                if !url.starts_with("https://") {
                    anyhow::bail!(
                        "invalid relay_url '{url}' on service '{}': must be https://",
                        svc.name
                    );
                }
            }
        }
        Ok(())
    }
}

impl RoleDoc for AccessConfig {
    fn validate(&self) -> Result<()> {
        AccessConfig::validate(self)
    }

    fn node_mut(&mut self) -> &mut NodeConfig {
        &mut self.node
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpfile(_name: &str, content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().expect("tmp");
        write!(f, "{content}").expect("write");
        f
    }

    #[test]
    fn protocol_serializes_lowercase() {
        // serde form: lowercase, round-trips through a containing struct.
        let svc = ServeService {
            name: "x".into(),
            protocol: Protocol::Tcp,
            host: "127.0.0.1".into(),
            port: 1,
            udp: Default::default(),
        };
        let toml = toml::to_string(&svc).unwrap();
        assert!(toml.contains("protocol = \"tcp\""));
        let parsed: ServeService = toml::from_str(&toml).unwrap();
        assert_eq!(parsed.protocol, Protocol::Tcp);

        let svc_udp = ServeService {
            protocol: Protocol::Udp,
            ..svc
        };
        let toml_udp = toml::to_string(&svc_udp).unwrap();
        assert!(toml_udp.contains("protocol = \"udp\""));
    }

    const UDP_SERVE: &str = "[[services]]\nname = \"dns\"\nprotocol = \"udp\"\nport = 53\n";

    #[test]
    fn udp_limits_default_to_the_documented_values_when_absent() {
        let f = tmpfile("serve.toml", UDP_SERVE);
        let cfg = ServeConfig::load(f.path()).unwrap();
        let limits = cfg.services[0].udp.resolve();
        assert_eq!(limits.idle_timeout, std::time::Duration::from_secs(60));
        assert_eq!(limits.max_flows, 256);
        assert_eq!(limits.max_flows_per_peer, 64);
        assert_eq!(limits.queue_depth, 64);
    }

    #[test]
    fn udp_limits_read_from_the_service_table() {
        let f = tmpfile(
            "serve.toml",
            &format!(
                "{UDP_SERVE}idle_timeout = 30\nmax_flows = 10\nmax_flows_per_peer = 4\nqueue_depth = 8\n"
            ),
        );
        let limits = ServeConfig::load(f.path()).unwrap().services[0]
            .udp
            .resolve();
        assert_eq!(limits.idle_timeout, std::time::Duration::from_secs(30));
        assert_eq!(limits.max_flows, 10);
        assert_eq!(limits.max_flows_per_peer, 4);
        assert_eq!(limits.queue_depth, 8);
    }

    #[test]
    fn udp_limits_of_zero_are_rejected_with_the_field_name() {
        for field in [
            "idle_timeout",
            "max_flows",
            "max_flows_per_peer",
            "queue_depth",
        ] {
            let f = tmpfile("serve.toml", &format!("{UDP_SERVE}{field} = 0\n"));
            let err = format!("{:#}", ServeConfig::load(f.path()).unwrap_err());
            assert!(err.contains(field), "{field}: {err}");
        }
    }

    #[test]
    fn a_per_peer_cap_above_the_service_cap_is_rejected() {
        let f = tmpfile(
            "serve.toml",
            &format!("{UDP_SERVE}max_flows = 4\nmax_flows_per_peer = 5\n"),
        );
        let err = format!("{:#}", ServeConfig::load(f.path()).unwrap_err());
        assert!(err.contains("max_flows_per_peer"), "{err}");
    }

    #[test]
    fn absurdly_large_udp_limits_are_rejected_not_passed_to_an_allocator() {
        for (field, value) in [
            ("queue_depth", "9000000000000000000"),
            ("queue_depth", "65537"),
            ("max_flows", "1000001"),
            ("idle_timeout", "86401"),
        ] {
            let f = tmpfile("serve.toml", &format!("{UDP_SERVE}{field} = {value}\n"));
            let err = format!("{:#}", ServeConfig::load(f.path()).unwrap_err());
            assert!(err.contains(field), "{field}={value}: {err}");
        }
        let f = tmpfile("serve.toml", &format!("{UDP_SERVE}queue_depth = 65536\n"));
        assert_eq!(
            ServeConfig::load(f.path()).unwrap().services[0]
                .udp
                .resolve()
                .queue_depth,
            65_536
        );
    }

    #[test]
    fn a_small_service_cap_alone_clamps_the_default_per_peer_cap() {
        let f = tmpfile("serve.toml", &format!("{UDP_SERVE}max_flows = 10\n"));
        let limits = ServeConfig::load(f.path()).unwrap().services[0]
            .udp
            .resolve();
        assert_eq!((limits.max_flows, limits.max_flows_per_peer), (10, 10));
    }

    #[test]
    fn udp_limits_on_a_tcp_service_are_rejected() {
        let f = tmpfile(
            "serve.toml",
            "[[services]]\nname = \"db\"\nprotocol = \"tcp\"\nport = 5432\nmax_flows = 5\n",
        );
        let err = format!("{:#}", ServeConfig::load(f.path()).unwrap_err());
        assert!(err.contains("udp"), "{err}");
    }

    #[test]
    fn access_takes_idle_timeout_and_cap_but_no_per_peer_cap_or_queue() {
        let node = "0".repeat(64);
        let base = format!(
            "[[services]]\nname = \"dns\"\nnode_id = \"{node}\"\nprotocol = \"udp\"\nport = 53\n"
        );
        let ok = tmpfile("access.toml", &format!("{base}max_flows = 7\n"));
        let limits = AccessConfig::load(ok.path()).unwrap().services[0]
            .udp
            .resolve();
        assert_eq!(limits.max_flows, 7);

        let queue = tmpfile("access.toml", &format!("{base}queue_depth = 3\n"));
        let err = format!("{:#}", AccessConfig::load(queue.path()).unwrap_err());
        assert!(err.contains("queue_depth"), "{err}");

        let bad = tmpfile("access.toml", &format!("{base}max_flows_per_peer = 2\n"));
        let err = format!("{:#}", AccessConfig::load(bad.path()).unwrap_err());
        assert!(err.contains("max_flows_per_peer"), "{err}");
    }

    #[test]
    fn a_config_without_udp_limits_serializes_without_them() {
        let f = tmpfile("serve.toml", UDP_SERVE);
        let cfg = ServeConfig::load(f.path()).unwrap();
        let out = toml::to_string(&cfg).unwrap();
        for field in ["idle_timeout", "max_flows", "queue_depth"] {
            assert!(!out.contains(field), "{field} leaked into {out}");
        }
    }

    #[test]
    fn unknown_fields_and_legacy_multiplex_still_load() {
        let serve = tmpfile(
            "serve.toml",
            "[node]\nfoo = 1\n\n[[services]]\nname = \"db\"\nprotocol = \"tcp\"\nport = 5432\n",
        );
        assert!(ServeConfig::load(serve.path()).is_ok());

        let node = "0".repeat(64);
        let access = tmpfile(
            "access.toml",
            &format!(
                "[[services]]\nname = \"db\"\nnode_id = \"{node}\"\nprotocol = \"tcp\"\nport = 5432\nmultiplex = true\n"
            ),
        );
        assert!(AccessConfig::load(access.path()).is_ok());
    }

    #[test]
    fn hostname_target_still_loads() {
        let f = tmpfile(
            "serve.toml",
            "[[services]]\nname = \"db\"\nprotocol = \"tcp\"\nhost = \"example.internal\"\nport = 5432\n",
        );
        let cfg = ServeConfig::load(f.path()).unwrap();
        assert_eq!(cfg.services[0].host, "example.internal");
    }

    #[test]
    fn access_node_id_length_checked_at_load_full_parse_at_runtime() {
        let access = |node_id: &str| {
            tmpfile(
                "access.toml",
                &format!(
                    "[[services]]\nname = \"db\"\nnode_id = \"{node_id}\"\nprotocol = \"tcp\"\nport = 5432\n"
                ),
            )
        };
        // Right length but not a parseable key: load only checks the length.
        let non_hex = "z".repeat(64);
        assert!(AccessConfig::load(access(&non_hex).path()).is_ok());

        let short = "0".repeat(10);
        let err = format!(
            "{:#}",
            AccessConfig::load(access(&short).path()).unwrap_err()
        );
        assert!(
            err.contains("must be 52 (base32) or 64 (hex) chars"),
            "{err}"
        );
    }

    #[test]
    fn serve_load_valid() {
        let f = tmpfile(
            "serve.toml",
            "[node]\nsecret_key = \"\"\n\n[[services]]\nname = \"postgres\"\nprotocol = \"tcp\"\nport = 5432\n",
        );
        let cfg = ServeConfig::load(f.path()).unwrap();
        assert_eq!(cfg.services.len(), 1);
        assert_eq!(cfg.services[0].name, "postgres");
        assert_eq!(cfg.services[0].host, "127.0.0.1"); // default
    }

    #[test]
    fn serve_load_missing_file_errors() {
        let err = ServeConfig::load(Path::new("/nonexistent/serve.toml")).unwrap_err();
        assert!(format!("{err:#}").contains("failed to read config"));
    }

    #[test]
    fn serve_load_bad_toml_errors() {
        let f = tmpfile("serve.toml", "this is = not = valid toml = =");
        let err = ServeConfig::load(f.path()).unwrap_err();
        assert!(format!("{err:#}").contains("failed to parse config"));
    }

    #[test]
    fn save_roundtrip_writes_file() {
        let cfg = ServeConfig {
            node: NodeConfig::default(),
            services: vec![ServeService {
                name: "web".into(),
                protocol: Protocol::Tcp,
                host: "127.0.0.1".into(),
                port: 8080,
                udp: Default::default(),
            }],
        };
        let f = tmpfile("serve.toml", "");
        cfg.save(f.path()).unwrap();
        let reloaded = ServeConfig::load(f.path()).unwrap();
        assert_eq!(reloaded, cfg);
    }

    #[test]
    fn access_resolve_and_save_key_generates_and_persists() {
        // An empty access secret_key is generated once and written back, so the
        // access NodeId is stable across restarts (mirrors serve behavior).
        let f = tmpfile("access.toml", "");
        let mut cfg = AccessConfig::default();
        assert!(cfg.node.secret_key.is_empty());

        let key = cfg.resolve_and_save_key(f.path()).unwrap();
        assert!(!cfg.node.secret_key.is_empty(), "key should be persisted");

        // The file on disk now carries the encoded key.
        let reloaded = AccessConfig::load(f.path()).unwrap();
        assert_eq!(reloaded.node.secret_key, cfg.node.secret_key);

        // Re-resolving off the persisted value is a no-op (needs_save == false)
        // and yields the same public key.
        let (key2, needs_save) = resolve_secret_key(&reloaded.node.secret_key).unwrap();
        assert!(!needs_save);
        assert_eq!(key.public(), key2.public());
    }

    #[test]
    fn access_resolve_and_save_key_no_save_when_present() {
        // A pre-existing secret_key is reused verbatim; the file is not rewritten.
        let (key, _) = resolve_secret_key("").unwrap();
        let enc = encode_secret_key(&key);
        let f = tmpfile("access.toml", "");
        let mut cfg = AccessConfig {
            node: NodeConfig {
                secret_key: enc.clone(),
                ..Default::default()
            },
            ..Default::default()
        };
        let resolved = cfg.resolve_and_save_key(f.path()).unwrap();
        assert_eq!(resolved.public(), key.public());
        assert_eq!(cfg.node.secret_key, enc, "value unchanged when already set");
    }

    #[test]
    fn resolve_empty_generates_key_and_needs_save() {
        let (key, needs_save) = resolve_secret_key("").unwrap();
        assert!(needs_save);
        // deterministic length once encoded
        let enc = encode_secret_key(&key);
        let (key2, needs_save2) = resolve_secret_key(&enc).unwrap();
        assert!(!needs_save2);
        assert_eq!(key.public(), key2.public());
    }

    #[test]
    fn resolve_invalid_base64_errors() {
        let err = resolve_secret_key("not!valid!base64!!").unwrap_err();
        assert!(format!("{err:#}").contains("invalid secret_key"));
    }

    #[test]
    fn resolve_wrong_length_errors() {
        // valid base64 but wrong byte length
        let short = BASE64.encode(b"only a few bytes");
        let err = resolve_secret_key(&short).unwrap_err();
        assert!(format!("{err:#}").contains("32 bytes"));
    }

    #[test]
    fn validation_rejects_uppercase_name() {
        let cfg = ServeConfig {
            services: vec![ServeService {
                name: "Postgres".into(),
                protocol: Protocol::Tcp,
                host: "127.0.0.1".into(),
                port: 5432,
                udp: Default::default(),
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(format!("{err:#}").contains("invalid service name"));
    }

    #[test]
    fn validation_rejects_duplicate_name() {
        let svc = ServeService {
            name: "db".into(),
            protocol: Protocol::Tcp,
            host: "127.0.0.1".into(),
            port: 5432,
            udp: Default::default(),
        };
        let cfg = ServeConfig {
            services: vec![svc.clone(), svc],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(format!("{err:#}").contains("duplicate service name"));
    }

    #[test]
    fn validation_rejects_zero_port() {
        let cfg = ServeConfig {
            services: vec![ServeService {
                name: "db".into(),
                protocol: Protocol::Tcp,
                host: "127.0.0.1".into(),
                port: 0,
                udp: Default::default(),
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(format!("{err:#}").contains("invalid port"));
    }

    #[test]
    fn validation_rejects_non_https_relay() {
        let cfg = ServeConfig {
            node: NodeConfig {
                relay_urls: vec!["http://insecure.relay".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(format!("{err:#}").contains("invalid relay_url"));
    }

    #[test]
    fn access_validation_rejects_short_node_id() {
        let cfg = AccessConfig {
            services: vec![AccessService {
                name: "db".into(),
                node_id: "tooshort".into(),
                protocol: Protocol::Tcp,
                host: "127.0.0.1".into(),
                port: 5432,
                relay_urls: vec![],
                udp: Default::default(),
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(format!("{err:#}").contains("invalid node_id"));
    }

    #[test]
    fn access_validation_rejects_non_https_service_relay() {
        // Per-service relay overrides must be https, with the service name
        // in the error so the offending entry is identifiable.
        let cfg = AccessConfig {
            services: vec![AccessService {
                name: "web".into(),
                node_id: "a".repeat(64),
                protocol: Protocol::Tcp,
                host: "127.0.0.1".into(),
                port: 8080,
                relay_urls: vec!["http://insecure.relay".into()],
                udp: Default::default(),
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("invalid relay_url"), "got: {msg}");
        assert!(msg.contains("'web'"), "service name missing: {msg}");
    }

    #[test]
    fn relay_token_validation_rejects_empty_and_whitespace() {
        for bad in [Some(""), Some("   "), Some("has space")] {
            let cfg = ServeConfig {
                node: NodeConfig {
                    relay_token: bad.map(str::to_string),
                    ..Default::default()
                },
                ..Default::default()
            };
            let err = cfg.validate().unwrap_err();
            assert!(format!("{err:#}").contains("relay_token"));
        }
    }

    #[test]
    fn relay_token_accepts_plain_secret() {
        let cfg = ServeConfig {
            node: NodeConfig {
                relay_token: Some("deadbeef1234".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.validate().unwrap();
    }

    #[test]
    fn access_validation_accepts_hex_node_id() {
        let hex_id = "a".repeat(64);
        let cfg = AccessConfig {
            services: vec![AccessService {
                name: "db".into(),
                node_id: hex_id,
                protocol: Protocol::Tcp,
                host: "127.0.0.1".into(),
                port: 5432,
                relay_urls: vec![],
                udp: Default::default(),
            }],
            ..Default::default()
        };
        cfg.validate().unwrap();
    }
    #[test]
    fn node_max_concurrent_streams_defaults_to_none() {
        // Absent field: None — the endpoint keeps noq's own default (100).
        let cfg: ServeConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.node.max_concurrent_streams, None);
        cfg.validate().unwrap();
    }

    #[test]
    fn node_max_concurrent_streams_parses() {
        let cfg: ServeConfig = toml::from_str(
            r#"
[node]
max_concurrent_streams = 512
"#,
        )
        .unwrap();
        assert_eq!(cfg.node.max_concurrent_streams, Some(512));
        cfg.validate().unwrap();
    }

    #[test]
    fn node_max_concurrent_streams_rejects_zero() {
        // 0 would forbid the peer from opening ANY bidi stream — nonsense
        // for a tunnel, so fail loudly at load time (both roles).
        for role in ["serve", "access"] {
            let toml = r#"
[node]
max_concurrent_streams = 0
"#;
            let err = match role {
                "serve" => toml::from_str::<ServeConfig>(toml)
                    .unwrap()
                    .validate()
                    .unwrap_err(),
                _ => toml::from_str::<AccessConfig>(toml)
                    .unwrap()
                    .validate()
                    .unwrap_err(),
            };
            assert!(
                format!("{err:#}").contains("max_concurrent_streams"),
                "{role}: {err:#}"
            );
        }
    }

    const SECRET_KEY_TEXT: &str = "c2VjcmV0LWtleS1mb3ItcmVkYWN0aW9uLXRlc3Q";
    const RELAY_TOKEN_TEXT: &str = "relay-token-for-redaction-test";

    #[test]
    fn config_debug_redacts_secret_key() {
        let f = tmpfile(
            "serve.toml",
            &format!(
                "[node]\nsecret_key = \"{SECRET_KEY_TEXT}\"\nrelay_urls = [\"https://relay.example\"]\nrelay_token = \"{RELAY_TOKEN_TEXT}\"\nmax_concurrent_streams = 7\n"
            ),
        );
        let cfg = ServeConfig::load(f.path()).unwrap();
        let shown = format!("{cfg:?}");
        assert!(!shown.contains(SECRET_KEY_TEXT), "{shown}");
        assert!(!shown.contains(RELAY_TOKEN_TEXT), "{shown}");
        assert!(shown.contains("[redacted]"), "{shown}");
        assert!(shown.contains("https://relay.example"), "{shown}");
        assert!(shown.contains("max_concurrent_streams: Some(7)"), "{shown}");
        let node = format!("{:?}", cfg.node);
        assert!(node.contains("relay_token: Some(\"[redacted]\")"), "{node}");
        assert!(node.contains("secret_key: \"[redacted]\""), "{node}");
        let empty = format!("{:?}", NodeConfig::default());
        assert!(empty.contains("secret_key: \"\""), "{empty}");
        assert!(empty.contains("relay_token: None"), "{empty}");
        let access: AccessConfig =
            toml::from_str(&format!("[node]\nsecret_key = \"{SECRET_KEY_TEXT}\"\n")).unwrap();
        assert!(!format!("{access:?}").contains(SECRET_KEY_TEXT));
    }

    #[test]
    fn error_chain_never_contains_secret_line() {
        let f = tmpfile(
            "serve.toml",
            &format!("[node]\nsecret_key = \"{SECRET_KEY_TEXT}\" oops\n"),
        );
        let e = ServeConfig::load(f.path()).unwrap_err();
        for text in [format!("{e:#}"), format!("{e:?}")] {
            assert!(!text.contains(SECRET_KEY_TEXT), "{text}");
        }
    }

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
