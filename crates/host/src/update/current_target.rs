//! The platform id matching release asset names.

use anyhow::bail;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_platform_has_a_release_target_or_a_clear_error() {
        match current_target() {
            Ok(target) => assert!(target.contains('-'), "{target}"),
            Err(e) => assert!(e.to_string().contains("no release is published")),
        }
    }
}
