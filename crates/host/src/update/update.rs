//! `update`: replace the running standalone executable with a verified
//! release (DIST-02, issue #80 policy).

use std::io::Read as _;
use std::path::Path;

use anyhow::{bail, Context as _};
use sha2::{Digest as _, Sha256};

use super::replace::{classify_replace_error, replace_executable};
use super::types::{UpdateContext, UpdateOutcome, UpdateRequest, Verification};
#[cfg(test)]
#[path = "../../../../tests/common/release_server.rs"]
mod release_server;

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

/// Upper bounds on downloaded/extracted data (RS-09).
const MAX_CHECKSUMS_BYTES: usize = 1 << 20;

const MAX_ARCHIVE_BYTES: usize = 256 << 20;

const MAX_EXECUTABLE_BYTES: u64 = 512 << 20;

#[cfg(test)]
mod tests {
    use super::super::fixtures::{ctx_with, AcceptAll};
    use super::super::types::{OverwritePolicy, Owner};
    use super::release_server;
    use super::*;

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
    fn older_detection_ignores_dev_suffix_and_handles_garbage() {
        assert!(is_older("0.4.1", "0.4.2"));
        assert!(!is_older("0.4.2", "0.4.2-dev"));
        assert!(!is_older("0.4.3", "0.4.2"));
        assert!(!is_older("x", "0.4.2"));
    }
}
