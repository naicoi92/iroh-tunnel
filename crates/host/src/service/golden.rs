//! Golden files pinning the exact bytes of every generated registration.
//!
//! Each backend renders every role × invocation × scope case into one text
//! and compares it with `tests/fixtures/registration/<name>`. Re-bless from
//! the current code with `GOLDEN_BLESS=1 cargo test -p iroh-tunnel-host`.

use std::path::{Path, PathBuf};

use super::pin::{Pin, PinOwner};
use super::types::{ServiceInvocation, ServiceScope};

/// One registration case of the golden matrix.
pub(crate) struct Case {
    pub role: &'static str,
    pub invocation: ServiceInvocation,
    pub scope: ServiceScope,
}

impl Case {
    /// The header line separating this case in a golden file.
    pub(crate) fn header(&self) -> String {
        format!(
            "==== role={} invocation={:?} scope={:?} ====\n",
            self.role, self.invocation, self.scope
        )
    }
}

/// Every role × invocation × scope case, in a fixed order.
pub(crate) fn cases() -> Vec<Case> {
    let mut all = Vec::new();
    for role in ["serve", "access"] {
        for invocation in [ServiceInvocation::Combined, ServiceInvocation::Standalone] {
            for scope in [ServiceScope::User, ServiceScope::System] {
                all.push(Case {
                    role,
                    invocation,
                    scope,
                });
            }
        }
    }
    all
}

/// A Unix path set with characters every backend must quote or escape.
pub(crate) struct UnixPaths {
    pub binary: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
}

pub(crate) fn unix_paths(role: &str) -> UnixPaths {
    UnixPaths {
        binary: PathBuf::from("/opt/iroh tunnel/bin/iroh-tunnel"),
        config: PathBuf::from(format!("/home/a b/it's & <cfg>/{role} $x%i.toml")),
        state: PathBuf::from("/var/lib/iroh $state%n"),
    }
}

/// The pin a system registration records; `None` for a user one.
pub(crate) fn unix_pin(scope: ServiceScope, config: &Path) -> Option<Pin> {
    (scope == ServiceScope::System).then(|| Pin {
        config: config.to_path_buf(),
        owner: PinOwner::Uid(1000),
    })
}

/// Compare `actual` with the golden file `name`, or write it under
/// `GOLDEN_BLESS=1`.
pub(crate) fn assert_golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/registration")
        .join(name);
    if std::env::var_os("GOLDEN_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {} ({e}); run with GOLDEN_BLESS=1",
            path.display()
        )
    });
    assert_eq!(actual, expected, "golden mismatch: {}", path.display());
}
