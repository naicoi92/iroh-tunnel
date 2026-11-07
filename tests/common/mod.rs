//! Helpers shared by integration tests.
// Each test crate uses a different subset of these helpers.
#![allow(dead_code)]

pub mod release_server;

use sha2::{Digest, Sha256};
use std::io::Write as _;
use std::path::Path;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Publish `version` for `target`: one `<exe>-<version>-<target>.zip` per
/// `(exe, entry, contents)`, plus one `checksums.txt` listing them all.
pub fn publish_zips(
    root: &Path,
    version: &str,
    target: &str,
    exes: &[(&str, &str, &[u8])],
) -> anyhow::Result<()> {
    let dir = root.join(version);
    std::fs::create_dir_all(&dir)?;
    let mut sums = String::new();
    for (exe, entry, contents) in exes {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            zip.start_file(*entry, zip::write::SimpleFileOptions::default())?;
            zip.write_all(contents)?;
            zip.finish()?;
        }
        let name = format!("{exe}-{version}-{target}.zip");
        std::fs::write(dir.join(&name), &buf)?;
        sums.push_str(&format!("{}  {name}\n", hex(&Sha256::digest(&buf))));
    }
    std::fs::write(dir.join("checksums.txt"), sums)?;
    std::fs::write(dir.join("checksums.txt.sigstore.json"), "{}")?;
    Ok(())
}

/// Role-ownership runtime state (#87): lock and owner records in the default
/// state directory. They are neither configuration nor keys.
pub fn is_ownership_artifact(path: &Path) -> bool {
    // `machine` is the test-build stand-in for the machine-wide lock dir.
    let in_state_dir = path.parent().is_some_and(|dir| {
        dir.ends_with(".local/state/iroh-tunnel")
            || dir.ends_with(".local/state/iroh-tunnel/machine")
    });
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    in_state_dir && (name.ends_with(".lock") || name.contains(".owner.json"))
}
