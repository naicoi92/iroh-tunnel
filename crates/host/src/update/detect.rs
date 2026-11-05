//! Which party owns the installed executable.

use std::path::Path;

use super::types::Owner;

/// Where the Linux packages install executables (`packaging/nfpm.yaml`).
const PACKAGE_BIN_DIR: &str = "/usr/bin";

/// Environment variable the project's images set (see the Dockerfiles).
///
/// Only this variable marks a container, not `/.dockerenv`: that file exists in
/// every Docker container, including one where the operator installed the
/// binary by hand or from a package, which is not an immutable image.
pub(crate) const CONTAINER_ENV: &str = "IROH_TUNNEL_CONTAINER";

/// Facts about the system that decide ownership; injectable for tests.
pub(crate) struct Probe<'a> {
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
pub(crate) fn classify(probe: &Probe<'_>) -> Owner {
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
pub(crate) fn in_homebrew_prefix(exe: &Path) -> bool {
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
}
