//! `update`: replace the running standalone executable with a verified
//! release (DIST-02, issue #80 policy).
//!
//! Flow: resolve the release → fetch `checksums*.txt` → (unless `--insecure`)
//! verify its signature → download the role archive → compare its SHA-256 →
//! extract exactly one entry in memory → stage beside the installed file and
//! rename over it. Any failure reports the error and leaves the installed
//! executable untouched. A running tunnel is never stopped; the operator is
//! told to restart it.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _};
use sha2::{Digest as _, Sha256};

use crate::error::CliError;
use crate::update_owner::Owner;

/// Release location; overridable for tests and mirrors (same variable as `install.sh`).
pub const DEFAULT_BASE_URL: &str = "https://github.com/naicoi92/iroh-tunnel/releases";
pub const BASE_URL_ENV: &str = "IROH_TUNNEL_BASE_URL";
/// Upper bounds on downloaded/extracted data (RS-09).
const MAX_CHECKSUMS_BYTES: usize = 1 << 20;
const MAX_ARCHIVE_BYTES: usize = 256 << 20;
const MAX_EXECUTABLE_BYTES: u64 = 512 << 20;

/// Verifies the authenticity of the release checksums (keyless CI signature).
///
/// `signed` is the exact `checksums*.txt` content, `bundle` its verification
/// material fetched from the release.
pub trait SignatureVerifier {
    /// # Errors
    /// Returns an error unless `bundle` proves that the pinned CI identity
    /// signed `signed`.
    fn verify(&self, signed: &[u8], bundle: &[u8]) -> anyhow::Result<()>;
}

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

/// Everything `update` needs from its surroundings.
pub struct UpdateContext<'a> {
    /// Release base, e.g. `https://github.com/<owner>/<repo>/releases`.
    pub base_url: String,
    /// Executable being replaced, e.g. `iroh-tunnel-serve`.
    pub exe_name: &'a str,
    /// Platform id as in `install.sh`, e.g. `linux-amd64-musl`.
    pub target: &'a str,
    /// Version of the running executable.
    pub current_version: &'a str,
    /// Absolute path of the installed executable.
    pub install_path: PathBuf,
    /// Checks the signature of the release checksums (skipped by `--insecure`).
    pub verifier: &'a dyn SignatureVerifier,
    /// Who owns the installed file (decides delegation, see [`update_owner`]).
    pub owner: Owner,
}

/// Whether the downloaded release is verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verification {
    /// Signature and checksum are verified (default).
    Verified,
    /// Signature and checksum verification is skipped (`--insecure`).
    Insecure,
}

/// What to do with a file a package manager owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwritePolicy {
    /// Delegate to the owner (default).
    RespectOwner,
    /// Replace a package-owned file anyway (`--yes`); never applies to containers.
    ReplacePackageOwned,
}

/// What the operator asked `update` to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRequest {
    /// Release to install, e.g. `0.4.2`; `None` is the latest stable.
    pub to: Option<String>,
    /// Whether the release is verified.
    pub verification: Verification,
    /// Ownership policy, consulted by [`check_owner`].
    pub overwrite: OverwritePolicy,
}

/// Whether `update` may replace the installed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerDecision {
    /// Replace the file; `overriding_package` when a package-owned file is replaced anyway.
    Proceed {
        /// The file is owned by a package manager and the operator overrode it.
        overriding_package: bool,
    },
    /// Update through the owner instead; nothing is replaced.
    Delegate {
        /// Instructions naming the owner's update command.
        guidance: String,
        /// Whether `ReplacePackageOwned` would have allowed replacing the file.
        overridable: bool,
    },
}

/// Result of a completed `update`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// The requested or latest release is already installed.
    AlreadyCurrent {
        /// The installed version.
        version: String,
    },
    /// `latest` is older than the installed version and no `--to` was given.
    NewerThanLatest {
        /// The installed version.
        current: String,
        /// The latest published version.
        latest: String,
    },
    /// The executable was replaced.
    Updated {
        /// The replaced file.
        path: PathBuf,
        /// Previous version.
        from: String,
        /// Installed version.
        to: String,
    },
}

/// Apply the ownership policy (#96): a file owned by a package manager or a
/// container image is updated through its owner. A package-owned file can
/// still be replaced with [`OverwritePolicy::ReplacePackageOwned`]; a
/// container image never is.
#[must_use]
pub fn check_owner(ctx: &UpdateContext<'_>, overwrite: OverwritePolicy) -> OwnerDecision {
    let Some(guidance) = ctx.owner.guidance(ctx.exe_name) else {
        return OwnerDecision::Proceed {
            overriding_package: false,
        };
    };
    let overridable = ctx.owner.overwrite_allowed_with_confirmation();
    if overridable && overwrite == OverwritePolicy::ReplacePackageOwned {
        return OwnerDecision::Proceed {
            overriding_package: true,
        };
    }
    OwnerDecision::Delegate {
        guidance,
        overridable,
    }
}

/// Core of `update`, with the environment injected. Does not check ownership
/// (see [`check_owner`]) and prints nothing.
///
/// # Errors
/// Fails on network or verification errors and replacement errors; the
/// installed executable is unchanged in every case.
pub async fn update(req: &UpdateRequest, ctx: &UpdateContext<'_>) -> anyhow::Result<UpdateOutcome> {
    let insecure = req.verification == Verification::Insecure;
    let requested = req.to.as_deref().map(parse_version).transpose()?;
    let client = http_client()?;
    let release_url = match &requested {
        Some(v) => format!("{}/download/v{v}", ctx.base_url.trim_end_matches('/')),
        None => format!("{}/latest/download", ctx.base_url.trim_end_matches('/')),
    };
    let checksums_name = checksums_name(ctx.target);
    // An unverified install of a named version needs no checksums file at all.
    let skip_checksums = insecure && requested.is_some();
    let checksums = if skip_checksums {
        String::new()
    } else {
        let bytes = fetch(
            &client,
            &format!("{release_url}/{checksums_name}"),
            MAX_CHECKSUMS_BYTES,
        )
        .await
        .with_context(|| format!("cannot fetch {checksums_name}"))?;
        if !insecure {
            let bundle = fetch(
                &client,
                &format!("{release_url}/{checksums_name}.sigstore.json"),
                MAX_CHECKSUMS_BYTES,
            )
            .await
            .context("cannot fetch the signature bundle; pass --insecure to skip verification")?;
            ctx.verifier.verify(&bytes, &bundle).context(
                "signature verification failed; the installed executable was not changed",
            )?;
        }
        String::from_utf8(bytes).context("checksums file is not UTF-8")?
    };
    let explicit = requested.is_some();
    let version = match requested {
        Some(v) => v,
        None => latest_version(&checksums, ctx.exe_name, ctx.target)?,
    };
    if version == ctx.current_version {
        return Ok(UpdateOutcome::AlreadyCurrent { version });
    }
    if !explicit && is_older(&version, ctx.current_version) {
        return Ok(UpdateOutcome::NewerThanLatest {
            current: ctx.current_version.to_owned(),
            latest: version,
        });
    }

    let archive_name = archive_name(ctx.exe_name, &version, ctx.target);
    let want = expected_sum(&checksums, &archive_name);
    if want.is_none() && !insecure {
        bail!("release {version} does not publish {archive_name}");
    }
    let archive = fetch(
        &client,
        &format!("{release_url}/{archive_name}"),
        MAX_ARCHIVE_BYTES,
    )
    .await
    .with_context(|| format!("cannot download {archive_name}"))?;
    if !insecure {
        let got = hex(&Sha256::digest(&archive));
        if want != Some(got.as_str()) {
            bail!(
                "checksum mismatch for {archive_name} (expected {}, got {got}); \
                 the installed executable was not changed",
                want.unwrap_or("none")
            );
        }
    }

    let exe_name = ctx.exe_name.to_owned();
    let entry = entry_name(&exe_name, ctx.target);
    let contents =
        tokio::task::spawn_blocking(move || extract_entry(&archive, &archive_name, &entry))
            .await
            .context("extraction task failed")??;
    let install_path = ctx.install_path.clone();
    tokio::task::spawn_blocking(move || replace_executable(&install_path, &contents))
        .await
        .context("replacement task failed")?
        .map_err(classify_replace_error)?;

    Ok(UpdateOutcome::Updated {
        path: ctx.install_path.clone(),
        from: ctx.current_version.to_owned(),
        to: version,
    })
}

/// Accepts `1.2.3` or `v1.2.3` (plus pre-release/build suffix); the result is
/// embedded in URLs and file names, so anything else is rejected.
fn parse_version(raw: &str) -> anyhow::Result<String> {
    let v = raw.strip_prefix('v').unwrap_or(raw);
    let valid = regex::Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.\-]+)?$")
        .expect("static regex")
        .is_match(v);
    if !valid {
        bail!("invalid version {raw:?}; expected e.g. 0.4.2");
    }
    Ok(v.to_owned())
}

/// What to do when the OS denies replacing the executable. On Windows "access
/// denied" is also what a running image reports, and `sudo` does not exist.
#[cfg(windows)]
const REPLACE_DENIED_HINT: &str =
    "if this executable is running (a tunnel, or this very command), \
     stop it and run the update again from another copy; otherwise check the file permissions";
#[cfg(not(windows))]
const REPLACE_DENIED_HINT: &str = "rerun with sufficient privileges (for example sudo)";

/// A permission failure while replacing is reported as [`CliError::Permission`]
/// (exit 3, RS-06); everything else keeps its unclassified exit 1.
fn classify_replace_error(err: anyhow::Error) -> anyhow::Error {
    let denied = err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
    });
    if denied {
        CliError::Permission(format!("{err:#}; {REPLACE_DENIED_HINT}")).into()
    } else {
        err
    }
}

/// True when `version` is strictly older than `current` (numeric core only;
/// a `-dev` or pre-release suffix of `current` is ignored).
fn is_older(version: &str, current: &str) -> bool {
    fn core(v: &str) -> Option<[u64; 3]> {
        let mut parts = v.split(['-', '+']).next()?.split('.');
        let mut out = [0; 3];
        for slot in &mut out {
            *slot = parts.next()?.parse().ok()?;
        }
        Some(out)
    }
    matches!((core(version), core(current)), (Some(v), Some(c)) if v < c)
}

/// Platform id matching the release asset names (`install.sh` `detect_target`).
///
/// # Errors
/// Fails when no release is published for this platform.
pub fn current_target() -> anyhow::Result<&'static str> {
    let target = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "darwin-arm64"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-amd64"
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        "windows-arm64"
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "musl"
    )) {
        "linux-amd64-musl"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-amd64"
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "aarch64",
        target_env = "musl"
    )) {
        "linux-arm64-musl"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "linux-arm64"
    } else if cfg!(all(target_os = "linux", target_arch = "riscv64")) {
        "linux-riscv64gc-musl"
    } else {
        bail!("no release is published for this platform; use the installer instead");
    };
    Ok(target)
}

fn checksums_name(target: &str) -> &'static str {
    if target.starts_with("darwin-") {
        "checksums-darwin.txt"
    } else {
        "checksums.txt"
    }
}

fn is_windows(target: &str) -> bool {
    target.starts_with("windows-")
}

fn archive_name(exe: &str, version: &str, target: &str) -> String {
    let ext = if is_windows(target) { "zip" } else { "tar.gz" };
    format!("{exe}-{version}-{target}.{ext}")
}

/// File name of the executable inside the archive.
fn entry_name(exe: &str, target: &str) -> String {
    if is_windows(target) {
        format!("{exe}.exe")
    } else {
        exe.to_owned()
    }
}

/// SHA-256 listed for `file` in a `sha256sum`-style checksums file.
fn expected_sum<'a>(checksums: &'a str, file: &str) -> Option<&'a str> {
    checksums.lines().find_map(|line| {
        let (sum, name) = line.split_once(char::is_whitespace)?;
        (name.trim_start_matches([' ', '*']) == file && sum.len() == 64).then_some(sum)
    })
}

/// Newest published version of `exe` for `target`: the single one the
/// `latest` release lists.
fn latest_version(checksums: &str, exe: &str, target: &str) -> anyhow::Result<String> {
    let prefix = format!("{exe}-");
    let suffix_tgz = format!("-{target}.tar.gz");
    let suffix_zip = format!("-{target}.zip");
    checksums
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .map(|n| n.trim_start_matches('*'))
        .find_map(|n| {
            let mid = n.strip_prefix(&prefix)?;
            let v = mid
                .strip_suffix(&suffix_tgz)
                .or_else(|| mid.strip_suffix(&suffix_zip))?;
            v.chars().next()?.is_ascii_digit().then(|| v.to_owned())
        })
        .with_context(|| format!("the latest release has no {exe} archive for {target}"))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn http_client() -> anyhow::Result<reqwest::Client> {
    // Already installed (by another component) is fine.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(concat!("iroh-tunnel/", env!("CARGO_PKG_VERSION")))
        .no_gzip()
        .no_brotli()
        .no_zstd()
        .no_deflate()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .context("cannot create HTTP client")
}

/// GET `url`, failing on non-success status or when the body exceeds `max` bytes.
async fn fetch(client: &reqwest::Client, url: &str, max: usize) -> anyhow::Result<Vec<u8>> {
    let mut response = client.get(url).send().await?.error_for_status()?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > max {
            bail!("response from {url} exceeds {max} bytes");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Read exactly one regular-file entry named `entry` out of the archive.
/// Nothing is written to disk and no other entry is looked at (RS-11).
fn extract_entry(archive: &[u8], archive_name: &str, entry: &str) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    if archive_name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
            .with_context(|| format!("cannot read {archive_name}"))?;
        let file = zip
            .by_name(entry)
            .with_context(|| format!("{archive_name} does not contain {entry}"))?;
        if !file.is_file() {
            bail!("{entry} in {archive_name} is not a regular file");
        }
        file.take(MAX_EXECUTABLE_BYTES + 1).read_to_end(&mut out)?;
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        let mut found = false;
        for e in tar
            .entries()
            .with_context(|| format!("cannot read {archive_name}"))?
        {
            let e = e?;
            let path = e.path()?;
            if path.as_ref() == Path::new(entry) || path.as_ref() == Path::new("./").join(entry) {
                if !e.header().entry_type().is_file() {
                    bail!("{entry} in {archive_name} is not a regular file");
                }
                e.take(MAX_EXECUTABLE_BYTES + 1).read_to_end(&mut out)?;
                found = true;
                break;
            }
        }
        if !found {
            bail!("{archive_name} does not contain {entry}");
        }
    }
    if out.len() as u64 > MAX_EXECUTABLE_BYTES {
        bail!("{entry} in {archive_name} exceeds {MAX_EXECUTABLE_BYTES} bytes");
    }
    if out.is_empty() {
        bail!("{entry} in {archive_name} is empty");
    }
    Ok(out)
}

/// Staging file beside the target; removed on drop unless
/// [`disarm`](Self::disarm)ed after a successful rename.
struct StagedFile {
    path: PathBuf,
    armed: std::cell::Cell<bool>,
}

impl StagedFile {
    /// Create the file exclusively (`O_EXCL`, so a pre-planted file or symlink
    /// is never followed) and write `contents` through that single handle.
    fn create(
        dir: &Path,
        target: &Path,
        contents: &[u8],
        permissions: &std::fs::Permissions,
    ) -> anyhow::Result<Self> {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!(
            ".iroh-tunnel-update-{}-{nonce:08x}-{seq}.tmp",
            std::process::id()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("cannot stage update beside {}", target.display()))?;
        let staged = Self {
            path,
            armed: std::cell::Cell::new(true),
        };
        file.set_permissions(permissions.clone()).with_context(|| {
            format!(
                "cannot set permissions on staged update for {}",
                target.display()
            )
        })?;
        file.write_all(contents)
            .and_then(|()| file.sync_all())
            .with_context(|| format!("cannot write staged update for {}", target.display()))?;
        Ok(staged)
    }

    fn disarm(&self) {
        self.armed.set(false);
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        if self.armed.get() {
            // Best effort: the original error is what the caller reports.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Replace the installed executable at `target` with `contents`.
///
/// The bytes are written to a temporary file in the same directory (so the
/// final rename stays on one filesystem), given the permissions of the file
/// being replaced, then renamed over it.
///
/// # Errors
///
/// Returns the OS error, with context, when `target` does not exist, the
/// staging file cannot be created, written or given permissions, or the
/// rename fails (for example a file the OS keeps locked on Windows, where
/// replacing the executable that is currently running is expected to fail with
/// the OS error; policy #80, covered by Windows runner evidence in #97). In every
/// case the installed executable and its directory are left as they were.
pub fn replace_executable(target: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let installed = std::fs::metadata(target)
        .with_context(|| format!("cannot read installed executable {}", target.display()))?;
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    let staged = StagedFile::create(dir, target, contents, &installed.permissions())?;
    std::fs::rename(&staged.path, target)
        .with_context(|| format!("cannot replace {}", target.display()))?;
    staged.disarm();
    Ok(())
}

#[cfg(test)]
#[path = "../tests/common/release_server.rs"]
mod release_server;

#[cfg(test)]
mod tests {
    use super::release_server;
    use super::*;

    struct AcceptAll;
    impl SignatureVerifier for AcceptAll {
        fn verify(&self, _: &[u8], _: &[u8]) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn ctx_with<'a>(
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

    #[test]
    fn package_owned_target_returns_delegation() {
        let deb = || Owner::LinuxPackage {
            package: "iroh-tunnel-serve".to_owned(),
        };
        let ctx = ctx_with(deb(), PathBuf::from("/usr/bin/x"), &AcceptAll);
        assert!(matches!(
            check_owner(&ctx, OverwritePolicy::RespectOwner),
            OwnerDecision::Delegate {
                overridable: true,
                ..
            }
        ));
        assert_eq!(
            check_owner(&ctx, OverwritePolicy::ReplacePackageOwned),
            OwnerDecision::Proceed {
                overriding_package: true
            }
        );
        let ctx = ctx_with(Owner::Container, PathBuf::from("/x"), &AcceptAll);
        for policy in [
            OverwritePolicy::RespectOwner,
            OverwritePolicy::ReplacePackageOwned,
        ] {
            assert!(matches!(
                check_owner(&ctx, policy),
                OwnerDecision::Delegate {
                    overridable: false,
                    ..
                }
            ));
        }
        let ctx = ctx_with(Owner::Standalone, PathBuf::from("/x"), &AcceptAll);
        assert_eq!(
            check_owner(&ctx, OverwritePolicy::RespectOwner),
            OwnerDecision::Proceed {
                overriding_package: false
            }
        );
    }

    #[test]
    fn update_uses_supplied_install_path() {
        let releases = tempfile::tempdir().unwrap();
        let dir = releases.path().join("9.9.9");
        std::fs::create_dir_all(&dir).unwrap();
        let name = "iroh-tunnel-serve-9.9.9-linux-amd64.tar.gz";
        let mut tar_bytes = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut tar_bytes, flate2::Compression::fast());
            let mut builder = tar::Builder::new(gz);
            let body = b"NEW";
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "iroh-tunnel-serve", &body[..])
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        std::fs::write(dir.join(name), &tar_bytes).unwrap();
        std::fs::write(
            dir.join("checksums.txt"),
            format!("{}  {name}\n", hex(&Sha256::digest(&tar_bytes))),
        )
        .unwrap();
        std::fs::write(dir.join("checksums.txt.sigstore.json"), "{}").unwrap();
        let server =
            release_server::ReleaseServer::start(releases.path().to_owned(), "9.9.9").unwrap();

        let install_dir = tempfile::tempdir().unwrap();
        let install = install_dir.path().join("iroh-tunnel-serve");
        std::fs::write(&install, b"OLD").unwrap();
        let mut ctx = ctx_with(Owner::Standalone, install.clone(), &AcceptAll);
        ctx.base_url = server.url.clone();
        let req = UpdateRequest {
            to: None,
            verification: Verification::Verified,
            overwrite: OverwritePolicy::RespectOwner,
        };

        let outcome = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(update(&req, &ctx))
            .unwrap();

        assert_eq!(
            outcome,
            UpdateOutcome::Updated {
                path: install.clone(),
                from: "0.0.1".to_owned(),
                to: "9.9.9".to_owned(),
            }
        );
        assert_eq!(std::fs::read(&install).unwrap(), b"NEW");
    }

    #[test]
    fn replaces_executable_content_and_keeps_no_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("iroh-tunnel-serve");
        std::fs::write(&target, b"old").unwrap();

        replace_executable(&target, b"new").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1, "staging file must not remain");
    }

    #[cfg(unix)]
    #[test]
    fn preserves_executable_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("it");
        std::fs::write(&target, b"old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();

        replace_executable(&target, b"new").unwrap();

        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[cfg(unix)]
    #[test]
    fn failure_to_stage_reports_error_and_keeps_old_binary() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("it");
        std::fs::write(&target, b"old").unwrap();
        // Directory not writable: staging beside the target must fail.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::File::create(dir.path().join("probe")).is_ok() {
            return; // running as root: permission cannot be denied
        }

        let err = replace_executable(&target, b"new").unwrap_err();

        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(format!("{err:#}").contains("stage"), "{err:#}");
    }

    #[test]
    fn missing_target_is_an_error_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("absent");

        assert!(replace_executable(&target, b"new").is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn older_detection_ignores_dev_suffix_and_handles_garbage() {
        assert!(is_older("0.4.1", "0.4.2"));
        assert!(!is_older("0.4.2", "0.4.2-dev"));
        assert!(!is_older("0.4.3", "0.4.2"));
        assert!(!is_older("x", "0.4.2"));
    }

    #[test]
    fn permission_denied_becomes_exit_class_permission() {
        let io = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let err = classify_replace_error(anyhow::Error::new(io).context("cannot stage"));
        assert!(matches!(
            err.downcast_ref::<CliError>(),
            Some(CliError::Permission(_))
        ));
        let other = classify_replace_error(anyhow::anyhow!("boom"));
        assert!(other.downcast_ref::<CliError>().is_none());
    }

    #[test]
    fn staging_names_are_unique() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("t");
        std::fs::write(&target, b"o").unwrap();
        let perms = std::fs::metadata(&target).unwrap().permissions();
        let a = StagedFile::create(dir.path(), &target, b"a", &perms).unwrap();
        let b = StagedFile::create(dir.path(), &target, b"b", &perms).unwrap();
        assert_ne!(a.path, b.path);
    }

    // Real `cosign sign-blob` bundle (Sigstore public-good, issuer GitHub OAuth,
    // signer w.vollprecht@gmail.com), copied from sigstore-rust's test data.
    const BLOB: &[u8] = include_bytes!("../tests/fixtures/sigstore/cosign-v3-blob.txt");
    const BUNDLE: &[u8] =
        include_bytes!("../tests/fixtures/sigstore/cosign-v3-blob.sigstore.json");
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
