//! #100: the signed release artifact set.
//!
//! Two layers:
//! - `release_set_*` run `packaging/verify-release-set.sh` against a synthetic
//!   but complete layout and prove it rejects every incomplete variant. They
//!   run everywhere a POSIX shell exists.
//! - `signed_release_*` verify the REAL artifacts produced by the release
//!   workflow with the verifier delivered by #95 ([`SigstoreVerifier`]). They
//!   need a directory of real artifacts, so they are `#[ignore]`d and run by
//!   the `release` / `promote` workflows with `RELEASE_DIST` and
//!   `RELEASE_TAG` set (`cargo test --test release_artifacts -- --ignored`).

use anyhow::{Context, Result};
use iroh_tunnel_host::update::{SignatureVerifier, SigstoreVerifier};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const ROLES: [&str; 3] = ["iroh-tunnel", "iroh-tunnel-serve", "iroh-tunnel-access"];
const VERSION: &str = "9.9.9";
const CI_ISSUER: &str = "https://token.actions.githubusercontent.com";

/// Platform suffixes of the role archives listed in `checksums.txt`.
const LINUX_WINDOWS_ARCHIVES: [&str; 7] = [
    "linux-amd64.tar.gz",
    "linux-arm64.tar.gz",
    "linux-amd64-musl.tar.gz",
    "linux-arm64-musl.tar.gz",
    "linux-riscv64gc-musl.tar.gz",
    "windows-amd64.zip",
    "windows-arm64.zip",
];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Names that `checksums.txt` must list for `version` (everything but darwin).
fn checksummed_names(version: &str) -> Vec<String> {
    let mut names = Vec::new();
    for role in ROLES {
        for suffix in &LINUX_WINDOWS_ARCHIVES {
            names.push(format!("{role}-{version}-{suffix}"));
        }
        for pkg in [
            "amd64.deb",
            "arm64.deb",
            "amd64.apk",
            "arm64.apk",
            "riscv64.apk",
        ] {
            names.push(format!("{role}_{version}_{pkg}"));
        }
    }
    names
}

fn darwin_names(version: &str) -> Vec<String> {
    ROLES
        .iter()
        .map(|r| format!("{r}-{version}-darwin-arm64.tar.gz"))
        .collect()
}

/// Write a complete, consistent release layout into `dir`.
fn complete_set(dir: &Path, version: &str) -> Result<()> {
    let mut plain = String::new();
    for name in checksummed_names(version) {
        let bytes = format!("payload of {name}").into_bytes();
        std::fs::write(dir.join(&name), &bytes)?;
        plain.push_str(&format!("{}  {name}\n", hex(&Sha256::digest(&bytes))));
    }
    let mut darwin = String::new();
    for name in darwin_names(version) {
        let bytes = format!("payload of {name}").into_bytes();
        std::fs::write(dir.join(&name), &bytes)?;
        darwin.push_str(&format!("{}  {name}\n", hex(&Sha256::digest(&bytes))));
    }
    std::fs::write(dir.join("checksums.txt"), plain)?;
    std::fs::write(dir.join("checksums-darwin.txt"), darwin)?;
    for name in ["checksums.txt", "checksums-darwin.txt"] {
        std::fs::write(dir.join(format!("{name}.sigstore.json")), "{}")?;
    }
    Ok(())
}

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packaging/verify-release-set.sh")
}

fn check_set(dir: &Path, version: &str) -> Result<std::process::Output> {
    std::process::Command::new("sh")
        .arg(script())
        .arg(dir)
        .arg(version)
        .output()
        .context("cannot run verify-release-set.sh")
}

#[cfg(unix)]
mod release_set {
    use super::*;

    #[test]
    fn release_set_complete_layout_passes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        complete_set(dir.path(), VERSION)?;
        let out = check_set(dir.path(), VERSION)?;
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(())
    }

    #[test]
    fn release_set_rejects_each_missing_artifact_and_names_it() -> Result<()> {
        let mut all = checksummed_names(VERSION);
        all.extend(darwin_names(VERSION));
        all.extend(
            ["checksums.txt", "checksums-darwin.txt"]
                .iter()
                .flat_map(|c| [(*c).to_owned(), format!("{c}.sigstore.json")]),
        );
        for missing in all {
            let dir = tempfile::tempdir()?;
            complete_set(dir.path(), VERSION)?;
            std::fs::remove_file(dir.path().join(&missing))?;
            let out = check_set(dir.path(), VERSION)?;
            assert!(!out.status.success(), "accepted a set without {missing}");
            assert!(
                String::from_utf8_lossy(&out.stderr).contains(&missing),
                "error does not name {missing}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }

    #[test]
    fn release_set_rejects_artifact_whose_checksum_differs() -> Result<()> {
        let dir = tempfile::tempdir()?;
        complete_set(dir.path(), VERSION)?;
        let name = format!("iroh-tunnel-serve-{VERSION}-windows-arm64.zip");
        std::fs::write(dir.path().join(&name), "tampered")?;
        let out = check_set(dir.path(), VERSION)?;
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains(&name));
        Ok(())
    }

    #[test]
    fn release_set_rejects_artifact_not_listed_in_checksums() -> Result<()> {
        let dir = tempfile::tempdir()?;
        complete_set(dir.path(), VERSION)?;
        let name = format!("iroh-tunnel-access-{VERSION}-darwin-arm64.tar.gz");
        std::fs::write(dir.path().join("checksums-darwin.txt"), "")?;
        let out = check_set(dir.path(), VERSION)?;
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains(&name));
        Ok(())
    }

    #[test]
    fn release_set_rejects_unexpected_extra_file() -> Result<()> {
        let dir = tempfile::tempdir()?;
        complete_set(dir.path(), VERSION)?;
        std::fs::write(dir.path().join("stray.bin"), "x")?;
        let out = check_set(dir.path(), VERSION)?;
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("stray.bin"));
        Ok(())
    }
}

// ---- real artifacts (CI only) ----------------------------------------------

/// One checksums file with its payload and verification bundle.
struct Signed {
    name: String,
    payload: Vec<u8>,
    bundle: Vec<u8>,
}

struct Real {
    dir: PathBuf,
    tag: String,
}

impl Real {
    fn from_env() -> Result<Self> {
        let dir = std::env::var("RELEASE_DIST").context("RELEASE_DIST is not set")?;
        let tag = std::env::var("RELEASE_TAG").context("RELEASE_TAG is not set")?;
        Ok(Self {
            dir: PathBuf::from(dir),
            tag,
        })
    }

    /// Signer identity of the release workflow run for this tag. The verifier
    /// matches it as a prefix, so it is also checked for equality below.
    fn identity(&self) -> String {
        format!(
            "https://github.com/naicoi92/iroh-tunnel/.github/workflows/release.yml@refs/tags/{}",
            self.tag
        )
    }

    fn pairs(&self) -> Result<Vec<Signed>> {
        ["checksums.txt", "checksums-darwin.txt"]
            .iter()
            .map(|n| {
                let signed = std::fs::read(self.dir.join(n)).with_context(|| (*n).to_owned())?;
                let bundle = std::fs::read(self.dir.join(format!("{n}.sigstore.json")))
                    .with_context(|| format!("{n}.sigstore.json"))?;
                Ok(Signed {
                    name: (*n).to_owned(),
                    payload: signed,
                    bundle,
                })
            })
            .collect()
    }
}

/// Print `sha256  name` for every asset: the immutable identifiers recorded as
/// evidence by the workflow summary.
fn print_identifiers(dir: &Path) -> Result<()> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    names.sort();
    for name in names {
        let digest = hex(&Sha256::digest(std::fs::read(dir.join(&name))?));
        println!("sha256:{digest}  {name}");
    }
    Ok(())
}

#[test]
#[ignore = "needs the real signed release artifacts (RELEASE_DIST, RELEASE_TAG)"]
fn signed_release_bundles_verify_for_pinned_issuer_and_release_identity() -> Result<()> {
    let real = Real::from_env()?;
    let verifier = SigstoreVerifier::new(CI_ISSUER, &real.identity());
    for Signed {
        name,
        payload: signed,
        bundle,
    } in real.pairs()?
    {
        verifier
            .verify(&signed, &bundle)
            .with_context(|| format!("{name} does not verify"))?;
        // The shipped default verifier (used by `update`) accepts it too.
        SigstoreVerifier::release()
            .verify(&signed, &bundle)
            .with_context(|| format!("{name} rejected by the default verifier"))?;
    }
    print_identifiers(&real.dir)
}

#[test]
#[ignore = "needs the real signed release artifacts (RELEASE_DIST, RELEASE_TAG)"]
fn signed_release_rejects_wrong_identity_and_wrong_issuer() -> Result<()> {
    let real = Real::from_env()?;
    for Signed {
        name,
        payload: signed,
        bundle,
    } in real.pairs()?
    {
        let wrong_identity = SigstoreVerifier::new(
            CI_ISSUER,
            "https://github.com/someone-else/iroh-tunnel/.github/workflows/release.yml@refs/tags/v",
        );
        assert!(
            wrong_identity.verify(&signed, &bundle).is_err(),
            "{name} accepted for a wrong identity"
        );
        let other_workflow = SigstoreVerifier::new(
            CI_ISSUER,
            "https://github.com/naicoi92/iroh-tunnel/.github/workflows/ci.yml@",
        );
        assert!(
            other_workflow.verify(&signed, &bundle).is_err(),
            "{name} accepted for a different workflow"
        );
        let wrong_issuer =
            SigstoreVerifier::new("https://github.com/login/oauth", &real.identity());
        assert!(
            wrong_issuer.verify(&signed, &bundle).is_err(),
            "{name} accepted for a wrong issuer"
        );
    }
    Ok(())
}

#[test]
#[ignore = "needs the real signed release artifacts (RELEASE_DIST, RELEASE_TAG)"]
fn signed_release_rejects_changed_payload_and_swapped_bundle() -> Result<()> {
    let real = Real::from_env()?;
    let verifier = SigstoreVerifier::new(CI_ISSUER, &real.identity());
    let pairs = real.pairs()?;
    for Signed {
        name,
        payload: signed,
        bundle,
    } in &pairs
    {
        let mut changed = signed.clone();
        changed.push(b'\n');
        assert!(
            verifier.verify(&changed, bundle).is_err(),
            "{name}: changed payload accepted"
        );
    }
    // A genuine bundle for the other checksums file must not vouch for this one.
    let first_signed = &pairs[0].payload;
    let other_bundle = &pairs[1].bundle;
    assert!(
        verifier.verify(first_signed, other_bundle).is_err(),
        "bundle of checksums-darwin.txt accepted for checksums.txt"
    );
    Ok(())
}

#[test]
#[ignore = "needs the real signed release artifacts (RELEASE_DIST, RELEASE_TAG)"]
fn signed_release_rejects_missing_or_invalid_verification_material() -> Result<()> {
    let real = Real::from_env()?;
    let verifier = SigstoreVerifier::new(CI_ISSUER, &real.identity());
    for Signed {
        name,
        payload: signed,
        bundle,
    } in real.pairs()?
    {
        for (label, material) in [
            ("empty", Vec::new()),
            ("not json", b"not a bundle".to_vec()),
            ("empty object", b"{}".to_vec()),
            ("truncated", bundle[..bundle.len() / 2].to_vec()),
        ] {
            assert!(
                verifier.verify(&signed, &material).is_err(),
                "{name}: {label} bundle accepted"
            );
        }
    }
    Ok(())
}
