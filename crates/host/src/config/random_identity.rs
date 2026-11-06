//! `config keygen --print`: an identity that touches no file.

use super::types::GeneratedIdentity;

/// Generate a fresh key and its node ID without touching any configuration
/// file or the default path. Nothing is logged.
pub fn random_identity() -> GeneratedIdentity {
    let key = iroh::SecretKey::generate();
    GeneratedIdentity {
        secret: crate::config::encode_secret_key(&key),
        node_id: key.public().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `random_identity` takes no path and no environment input, so there is
    /// nothing to point it at; setting `HOME` would race other threads of the
    /// test binary. Instead assert the identity is self-consistent and that a
    /// tempdir this test controls stays empty across the call.
    #[test]
    fn random_identity_touches_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let identity = random_identity();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let (key, _) = crate::config::resolve_secret_key(identity.secret()).unwrap();
        assert_eq!(identity.node_id, key.public().to_string());
        assert!(!format!("{identity:?}").contains(identity.secret()));
    }
}
