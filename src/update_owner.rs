//! Who owns the installed executable (issue #96, AC-DIST-04/07).
//!
//! The standalone updater replaces only files it installed itself. A file that
//! a package manager or a container image owns is updated through that owner,
//! so `update` prints the owner's command instead (default) or, after an
//! explicit confirmation, replaces it anyway.

use std::path::Path;

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

/// Where the Linux packages install executables (`packaging/nfpm.yaml`).
const PACKAGE_BIN_DIR: &str = "/usr/bin";
/// Environment variable the project's images set (see the Dockerfiles).
///
/// Only this variable marks a container, not `/.dockerenv`: that file exists in
/// every Docker container, including one where the operator installed the
/// binary by hand or from a package, which is not an immutable image.
pub const CONTAINER_ENV: &str = "IROH_TUNNEL_CONTAINER";

/// Facts about the system that decide ownership; injectable for tests.
pub struct Probe<'a> {
    /// Canonical path of the running executable.
    pub exe: &'a Path,
    /// `IROH_TUNNEL_CONTAINER` value, if set.
    pub container_env: Option<&'a str>,
    /// Whether `exe` is in the Homebrew prefix (`Caskroom`/`Cellar`/`opt/homebrew`).
    pub homebrew_prefix: bool,
}

/// Classify the installed executable. Container wins over everything else:
/// inside an image nothing is updated in place.
#[must_use]
pub fn classify(probe: &Probe<'_>) -> Owner {
    if probe.container_env.is_some_and(|v| v == "1") {
        return Owner::Container;
    }
    if probe.homebrew_prefix {
        return Owner::Homebrew;
    }
    if probe.exe.parent() == Some(Path::new(PACKAGE_BIN_DIR)) {
        if let Some(name) = probe.exe.file_name().and_then(|n| n.to_str()) {
            return Owner::LinuxPackage {
                package: name.to_owned(),
            };
        }
    }
    Owner::Standalone
}

/// True when `exe` lies below a Homebrew prefix.
#[must_use]
pub fn in_homebrew_prefix(exe: &Path) -> bool {
    exe.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some("Caskroom" | "Cellar" | "homebrew")
        )
    })
}

/// Detect the owner of the running executable on this system.
#[must_use]
pub fn detect(exe: &Path) -> Owner {
    let env = std::env::var(CONTAINER_ENV).ok();
    classify(&Probe {
        exe,
        container_env: env.as_deref(),
        homebrew_prefix: in_homebrew_prefix(exe),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(exe: &str) -> Probe<'_> {
        Probe {
            exe: Path::new(exe),
            container_env: None,
            homebrew_prefix: false,
        }
    }

    #[test]
    fn usr_local_bin_is_standalone() {
        assert_eq!(
            classify(&probe("/usr/local/bin/iroh-tunnel")),
            Owner::Standalone
        );
        assert_eq!(classify(&probe("/home/u/bin/its")), Owner::Standalone);
    }

    #[test]
    fn usr_bin_is_a_linux_package_named_after_the_binary() {
        assert_eq!(
            classify(&probe("/usr/bin/iroh-tunnel-serve")),
            Owner::LinuxPackage {
                package: "iroh-tunnel-serve".into()
            }
        );
    }

    #[test]
    fn the_container_variable_wins_over_the_install_path() {
        let mut p = probe("/usr/bin/iroh-tunnel");
        p.container_env = Some("1");
        assert_eq!(classify(&p), Owner::Container);
        let mut p = probe("/usr/local/bin/iroh-tunnel");
        p.container_env = Some("1");
        assert_eq!(classify(&p), Owner::Container);
        let mut p = probe("/usr/local/bin/iroh-tunnel");
        p.container_env = Some("0");
        assert_eq!(classify(&p), Owner::Standalone);
    }

    #[test]
    fn homebrew_prefixes_are_recognised() {
        for path in [
            "/opt/homebrew/Caskroom/iroh-tunnel/0.4.2/iroh-tunnel",
            "/opt/homebrew/bin/iroh-tunnel",
            "/usr/local/Cellar/iroh-tunnel/0.4.2/bin/iroh-tunnel",
        ] {
            assert!(in_homebrew_prefix(Path::new(path)), "{path}");
        }
        assert!(!in_homebrew_prefix(Path::new("/usr/local/bin/iroh-tunnel")));
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
}
