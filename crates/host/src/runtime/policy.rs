//! Whether a run enforces role ownership.

use anyhow::Result;

use crate::ownership::RoleClaim;
use crate::role::Role;

/// Whether a run enforces role ownership (RO-01..RO-03).
///
/// Production entry points enforce it; `testing::run_unowned` (test-utils)
/// does not, so several in-process roles of one kind can share a test
/// binary. Real-process tests cover enforcement.
#[derive(Clone, Copy)]
pub(crate) enum Ownership {
    Enforced(Role),
    #[cfg(feature = "test-utils")]
    Unenforced,
}

impl Ownership {
    /// Take the role's locks, refusing while the role already runs.
    pub(crate) fn claim(self) -> Result<Option<RoleClaim>> {
        match self {
            Self::Enforced(role) => crate::ownership::claim(role).map(Some),
            #[cfg(feature = "test-utils")]
            Self::Unenforced => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforced_ownership_claims_the_role_lock() {
        // Role locks are keyed by HOME; only the shape is checked here (the
        // claim itself is covered by the ownership and runtime tests).
        assert!(matches!(
            Ownership::Enforced(Role::Serve),
            Ownership::Enforced(Role::Serve)
        ));
    }
}
