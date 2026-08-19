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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct NodeConfig {
    #[serde(default)]
    pub secret_key: String,
    #[serde(default)]
    pub relay_urls: Vec<String>,
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

impl NodeConfig {
    /// Shared node-level validation. Both roles call this from their
    /// `RoleDoc::validate`.
    pub fn validate(&self) -> Result<()> {
        if let Some(max) = self.max_concurrent_streams {
            if max == 0 {
                anyhow::bail!("invalid max_concurrent_streams {max}: must be >= 1");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServeService {
    pub name: String,
    pub protocol: Protocol,
    #[serde(default = "default_host")]
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessService {
    pub name: String,
    pub node_id: String,
    pub protocol: Protocol,
    #[serde(default = "default_host")]
    pub host: String,
    pub port: u16,
    /// Stream multiplexing for this service (since 0.2.0).
    ///
    /// `true` (default): the service keeps ONE long-lived iroh connection to
    /// the serve peer and opens one bidirectional stream per local TCP
    /// connection. Handshakes are paid once.
    ///
    /// `false`: one iroh connection per local TCP connection — the pre-0.2.0
    /// behavior verbatim.
    ///
    /// ROLLOUT CONTRACT: multiplexing requires a 0.2.0+ serve peer — there
    /// is deliberately no protocol negotiation (the ALPN is unchanged).
    /// Upgrade serve nodes first; if you must run this access against an
    /// older serve, set `multiplex = false`. Only meaningful for TCP
    /// services (the UDP path is unchanged); accepted regardless so configs
    /// stay forward-compatible.
    #[serde(default = "default_multiplex")]
    pub multiplex: bool,
}

/// Default for [`AccessService::multiplex`].
fn default_multiplex() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
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
        let cfg: Self = toml::from_str(&content)
            .with_context(|| format!("failed to parse config: {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Serialize and write the config to disk.
    fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).context("failed to serialize config")?;
        std::fs::write(path, content)
            .with_context(|| format!("failed to write config: {}", path.display()))?;
        Ok(())
    }

    /// Resolve the node secret key; if it was just generated, persist it back
    /// into the config file. Returns the resolved key.
    ///
    /// The key value itself is never logged (NFR-05).
    fn resolve_and_save_key(&mut self, path: &Path) -> Result<SecretKey> {
        let (key, needs_save) = resolve_secret_key(&self.node_mut().secret_key)?;
        if needs_save {
            tracing::warn!("secret_key empty, generated new key, saving to config");
            self.node_mut().secret_key = encode_secret_key(&key);
            self.save(path)?;
        }
        Ok(key)
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
                multiplex: true,
            }],
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(format!("{err:#}").contains("invalid node_id"));
    }

    #[test]
    fn access_validation_accepts_hex_node_id() {
        // iroh 1.0's PublicKey::Display emits lowercase hex (64 chars). A node
        // id copied straight from `serve` output must validate.
        let hex_id = "a".repeat(64);
        let cfg = AccessConfig {
            services: vec![AccessService {
                name: "db".into(),
                node_id: hex_id,
                protocol: Protocol::Tcp,
                host: "127.0.0.1".into(),
                port: 5432,
                multiplex: true,
            }],
            ..Default::default()
        };
        cfg.validate().unwrap();
    }
    #[test]
    fn access_multiplex_defaults_to_true() {
        // TOML without the field parses with multiplex = true (pre-0.2.0
        // configs keep parsing; rollout contract: upgrade serve first).
        let toml = r#"
[[services]]
name = "db"
node_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
protocol = "tcp"
port = 5432
"#;
        let cfg: AccessConfig = toml::from_str(toml).unwrap();
        assert!(cfg.services[0].multiplex);
        cfg.validate().unwrap();
    }

    #[test]
    fn access_multiplex_parses_bool() {
        let id = "a".repeat(64);
        for (raw, want) in [("true", true), ("false", false)] {
            let toml = format!(
                r#"
[[services]]
name = "db"
node_id = "{id}"
protocol = "tcp"
port = 5432
multiplex = {raw}
"#
            );
            let cfg: AccessConfig = toml::from_str(&toml).unwrap();
            assert_eq!(cfg.services[0].multiplex, want, "multiplex = {raw}");
            cfg.validate().unwrap();
        }
    }

    #[test]
    fn access_multiplex_rejects_string() {
        // It's a bool, not a mode string — a stale "auto"/"off" value from
        // draft configs must fail loudly instead of being ignored.
        let id = "a".repeat(64);
        let toml = format!(
            r#"
[[services]]
name = "db"
node_id = "{id}"
protocol = "tcp"
port = 5432
multiplex = "auto"
"#
        );
        let err = toml::from_str::<AccessConfig>(&toml).unwrap_err();
        assert!(format!("{err}").contains("auto"));
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
}
