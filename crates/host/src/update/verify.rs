//! Keyless Sigstore verification of the release checksums (private
//! infrastructure of `update`).

use anyhow::{bail, Context as _};

use super::types::SignatureVerifier;

/// OIDC issuer of GitHub Actions keyless certificates.
const CI_ISSUER: &str = "https://token.actions.githubusercontent.com";

/// Signer identity of release builds: this repository's release workflow on a
/// version tag. The tag itself is not pinned because `latest` learns the
/// version only from the (signed) checksums; the prefix still fixes the
/// repository, the workflow file and that the build ran for a tag.
const CI_IDENTITY_PREFIX: &str =
    "https://github.com/naicoi92/iroh-tunnel/.github/workflows/release.yml@refs/tags/v";

/// Keyless Sigstore verification of the `checksums*.txt` bundle: a certificate
/// issued by the Sigstore public-good CA (embedded trusted root, so no
/// network), logged in the transparency log, from the pinned issuer and an
/// identity starting with the pinned prefix.
pub struct SigstoreVerifier {
    issuer: String,
    identity_prefix: String,
}

impl SigstoreVerifier {
    /// Verifier for this project's release pipeline.
    #[must_use]
    pub fn release() -> Self {
        Self::new(CI_ISSUER, CI_IDENTITY_PREFIX)
    }

    /// Verifier pinned to `issuer` (exact) and identities starting with
    /// `identity_prefix`.
    #[must_use]
    pub fn new(issuer: &str, identity_prefix: &str) -> Self {
        Self {
            issuer: issuer.to_owned(),
            identity_prefix: identity_prefix.to_owned(),
        }
    }
}

impl SignatureVerifier for SigstoreVerifier {
    fn verify(&self, signed: &[u8], bundle: &[u8]) -> anyhow::Result<()> {
        use sigstore_verify::{VerificationPolicy, Verifier};

        let bundle = std::str::from_utf8(bundle).context("signature bundle is not UTF-8")?;
        let bundle = sigstore_verify::types::Bundle::from_json(bundle)
            .map_err(|e| anyhow::anyhow!("invalid signature bundle: {e}"))?;
        let root = sigstore_verify::trust_root::TrustedRoot::from_json(
            sigstore_verify::trust_root::SIGSTORE_PRODUCTION_TRUSTED_ROOT,
        )
        .map_err(|e| anyhow::anyhow!("cannot load the embedded Sigstore trusted root: {e}"))?;
        let verifier =
            Verifier::new(&root).map_err(|e| anyhow::anyhow!("cannot create verifier: {e}"))?;
        let policy = VerificationPolicy::any_identity().require_issuer(self.issuer.as_str());
        let result = verifier
            .verify(signed, &bundle, &policy)
            .map_err(|e| anyhow::anyhow!("signature does not verify: {e}"))?;
        let identity = result
            .identity()
            .map(|i| i.as_str().to_owned())
            .context("signature carries no signer identity")?;
        if !identity.starts_with(&self.identity_prefix) {
            bail!(
                "signed by {identity:?}, not by the release workflow of this project \
                 (expected {:?}...)",
                self.identity_prefix
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `cosign sign-blob` bundle (Sigstore public-good, issuer GitHub OAuth,
    // signer w.vollprecht@gmail.com), copied from sigstore-rust's test data.
    const BLOB: &[u8] = include_bytes!("../../../../tests/fixtures/sigstore/cosign-v3-blob.txt");

    const BUNDLE: &[u8] =
        include_bytes!("../../../../tests/fixtures/sigstore/cosign-v3-blob.sigstore.json");

    const ISSUER: &str = "https://github.com/login/oauth";

    #[test]
    fn genuine_bundle_for_pinned_issuer_and_identity_verifies() {
        let v = SigstoreVerifier::new(ISSUER, "w.vollprecht@gmail.com");
        v.verify(BLOB, BUNDLE).unwrap();
    }

    #[test]
    fn changed_payload_is_rejected() {
        let v = SigstoreVerifier::new(ISSUER, "w.vollprecht@gmail.com");
        let mut payload = BLOB.to_vec();
        payload.push(b'x');
        let err = v.verify(&payload, BUNDLE).unwrap_err();
        assert!(format!("{err:#}").contains("does not verify"), "{err:#}");
    }

    #[test]
    fn wrong_identity_is_rejected() {
        let v = SigstoreVerifier::new(ISSUER, "someone-else@example.com");
        let err = v.verify(BLOB, BUNDLE).unwrap_err();
        assert!(
            format!("{err:#}").contains("not by the release workflow"),
            "{err:#}"
        );
    }

    #[test]
    fn wrong_issuer_is_rejected() {
        let v = SigstoreVerifier::new(CI_ISSUER, "w.vollprecht@gmail.com");
        assert!(v.verify(BLOB, BUNDLE).is_err());
    }

    #[test]
    fn release_verifier_rejects_a_foreign_signer() {
        assert!(SigstoreVerifier::release().verify(BLOB, BUNDLE).is_err());
    }

    #[test]
    fn missing_or_invalid_bundle_is_rejected() {
        let v = SigstoreVerifier::release();
        assert!(v.verify(BLOB, b"").is_err());
        assert!(v.verify(BLOB, b"{}").is_err());
        assert!(v.verify(BLOB, b"not json").is_err());
        assert!(v.verify(BLOB, &[0xff, 0xfe]).is_err());
    }
}
