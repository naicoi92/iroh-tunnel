//! Encoding of the node secret key for config storage.

use data_encoding::BASE64;
use iroh::SecretKey;

/// Encode a [`SecretKey`] to a base64 string for config storage.
pub fn encode_secret_key(key: &SecretKey) -> String {
    BASE64.encode(&key.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_key_decodes_to_the_same_key() {
        let key = SecretKey::generate();
        let (decoded, generated) =
            super::super::resolve_secret_key::resolve_secret_key(&encode_secret_key(&key)).unwrap();
        assert!(!generated);
        assert_eq!(decoded.public(), key.public());
    }
}
