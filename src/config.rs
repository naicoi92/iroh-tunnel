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

/// Base32 node_id length (Iroh PublicKey encoded is 52 chars).
const NODE_ID_LEN: usize = 52;

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
    #[serde(default)]
    pub services: Vec<AccessService>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct NodeConfig {
    #[serde(default)]
    pub secret_key: String,
    #[serde(default)]
    pub relay_urls: Vec<String>,
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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
}

// ---------------------------------------------------------------------------
// T-02.2: load / save
// ---------------------------------------------------------------------------

impl ServeConfig {
    /// Load and validate a serve config from a TOML file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        let cfg: Self = toml::from_str(&content)
            .with_context(|| format!("failed to parse config: {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Serialize and write the config to disk.
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .context("failed to serialize config")?;
        std::fs::write(path, content)
            .with_context(|| format!("failed to write config: {}", path.display()))?;
        Ok(())
    }
}

impl AccessConfig {
    /// Load and validate an access config from a TOML file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        let cfg: Self = toml::from_str(&content)
            .with_context(|| format!("failed to parse config: {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Serialize and write the config to disk.
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .context("failed to serialize config")?;
        std::fs::write(path, content)
            .with_context(|| format!("failed to write config: {}", path.display()))?;
        Ok(())
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

impl ServeConfig {
    /// Resolve the node secret key; if it was just generated, persist it back
    /// into the config file. Returns the resolved key.
    ///
    /// The key value itself is never logged (NFR-05).
    pub fn resolve_and_save_key(&mut self, path: &Path) -> Result<SecretKey> {
        let (key, needs_save) = resolve_secret_key(&self.node.secret_key)?;
        if needs_save {
            tracing::warn!("secret_key empty, generated new key, saving to config");
            self.node.secret_key = encode_secret_key(&key);
            self.save(path)?;
        }
        Ok(key)
    }
}

// ---------------------------------------------------------------------------
// T-02.4: validation
// ---------------------------------------------------------------------------

fn validate_name(name: &str) -> Result<()> {
    if !name_regex().is_match(name) {
        anyhow::bail!(
            "invalid service name '{name}': must match ^[a-z0-9-]+$"
        );
    }
    if name.len() > MAX_NAME_LEN {
        anyhow::bail!(
            "invalid service name '{name}': max {MAX_NAME_LEN} bytes (ALPN limit)"
        );
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
        Ok(())
    }
}

impl AccessConfig {
    /// Validate services (names, ports, node_id format, duplicates).
    pub fn validate(&self) -> Result<()> {
        let mut seen: HashSet<&str> = HashSet::new();
        for svc in &self.services {
            validate_name(&svc.name)?;
            if !seen.insert(svc.name.as_str()) {
                anyhow::bail!("duplicate service name: '{}'", svc.name);
            }
            if svc.node_id.len() != NODE_ID_LEN {
                anyhow::bail!(
                    "invalid node_id '{}': must be {NODE_ID_LEN} chars (base32)",
                    svc.node_id
                );
            }
            validate_port(svc.port)?;
        }
        Ok(())
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
            }],
        };
        let err = cfg.validate().unwrap_err();
        assert!(format!("{err:#}").contains("invalid node_id"));
    }
}
