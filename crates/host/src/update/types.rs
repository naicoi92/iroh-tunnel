//! Request, context and outcome types of the update operations.

use std::path::PathBuf;

/// Release location; overridable for tests and mirrors (same variable as `install.sh`).
pub const DEFAULT_BASE_URL: &str = "https://github.com/naicoi92/iroh-tunnel/releases";

/// Environment variable overriding [`DEFAULT_BASE_URL`].
pub const BASE_URL_ENV: &str = "IROH_TUNNEL_BASE_URL";

/// Who is responsible for updating the installed executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    /// Installed by `install.sh` / `install.ps1` or unpacked by hand.
    Standalone,
    /// Installed by a Linux package (`.deb` or `.apk`); update with the manager.
    LinuxPackage {
        /// Package name, e.g. `iroh-tunnel-serve`.
        package: String,
    },
    /// Installed by the Homebrew cask.
    Homebrew,
    /// Runs inside a container image; redeploy the image.
    Container,
}

impl Owner {
    /// Guidance printed instead of updating, or `None` for a standalone file.
    #[must_use]
    pub fn guidance(&self, exe_name: &str) -> Option<String> {
        match self {
            Owner::Standalone => None,
            Owner::LinuxPackage { package } => Some(format!(
                "{exe_name} is owned by the package {package:?}; update it with your package \
                 manager (for example `apt-get install --only-upgrade {package}` or \
                 `apk upgrade {package}`)"
            )),
            Owner::Homebrew => Some(format!(
                "{exe_name} is managed by Homebrew; update it with `brew upgrade --cask iroh-tunnel`"
            )),
            Owner::Container => Some(format!(
                "{exe_name} runs in a container image and is not updated in place; pull the new \
                 image and redeploy the container"
            )),
        }
    }

    /// Whether the file may be replaced after a confirmation. A container
    /// image is immutable by design and is never overwritten.
    #[must_use]
    pub fn overwrite_allowed_with_confirmation(&self) -> bool {
        matches!(self, Owner::LinuxPackage { .. } | Owner::Homebrew)
    }
}

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
    /// Who owns the installed file (decides delegation, see [`check_owner`](super::check_owner)).
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
    /// Ownership policy, consulted by [`super::check_owner`].
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_managed_files_may_be_overwritten_and_never_a_container() {
        assert!(!Owner::Standalone.overwrite_allowed_with_confirmation());
        assert!(Owner::Homebrew.overwrite_allowed_with_confirmation());
        assert!(Owner::LinuxPackage {
            package: "x".into()
        }
        .overwrite_allowed_with_confirmation());
        assert!(!Owner::Container.overwrite_allowed_with_confirmation());
    }

    #[test]
    fn guidance_names_the_owner_command_and_standalone_has_none() {
        assert!(Owner::Standalone.guidance("it").is_none());
        let deb = Owner::LinuxPackage {
            package: "iroh-tunnel".into(),
        }
        .guidance("iroh-tunnel")
        .unwrap();
        assert!(
            deb.contains("apt-get install --only-upgrade iroh-tunnel"),
            "{deb}"
        );
        assert!(deb.contains("apk upgrade iroh-tunnel"), "{deb}");
        assert!(Owner::Homebrew
            .guidance("it")
            .unwrap()
            .contains("brew upgrade"));
        assert!(Owner::Container
            .guidance("it")
            .unwrap()
            .contains("redeploy"));
    }
}
