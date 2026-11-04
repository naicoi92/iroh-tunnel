//! Decoding of the node secret key stored in a config.

use anyhow::{Context, Result};
use data_encoding::BASE64;
use iroh::SecretKey;

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

#[cfg(test)]
mod tests {
    use super::super::encode_secret_key::encode_secret_key;
    use super::*;

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
}
