//! Shared fixtures of the update unit tests.

use std::path::PathBuf;

use super::types::{Owner, SignatureVerifier, UpdateContext};

pub(crate) struct AcceptAll;

impl SignatureVerifier for AcceptAll {
    fn verify(&self, _: &[u8], _: &[u8]) -> anyhow::Result<()> {
        Ok(())
    }
}

pub(crate) fn ctx_with<'a>(
    owner: Owner,
    install_path: PathBuf,
    verifier: &'a dyn SignatureVerifier,
) -> UpdateContext<'a> {
    UpdateContext {
        base_url: "http://127.0.0.1:9".to_owned(),
        exe_name: "iroh-tunnel-serve",
        target: "linux-amd64",
        current_version: "0.0.1",
        install_path,
        verifier,
        owner,
    }
}
