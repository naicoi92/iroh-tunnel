//! Ownership policy of an update (#96).

use super::types::{OverwritePolicy, OwnerDecision, UpdateContext};

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

#[cfg(test)]
mod tests {
    use super::super::fixtures::{ctx_with, AcceptAll};
    use super::super::types::Owner;
    use super::*;
    use std::path::PathBuf;

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
}
