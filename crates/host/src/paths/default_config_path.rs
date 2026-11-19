//! Default config file of a role in the current process tier.

use std::path::PathBuf;

use anyhow::Result;

use super::default_config_path_for;
use crate::Tier;

/// Default config file for `role` in the tier of the current process
/// ([`Tier::current`]); see [`default_config_path_for`].
///
/// # Errors
///
/// As [`default_config_path_for`].
pub fn default_config_path(role: &str) -> Result<PathBuf> {
    default_config_path_for(role, Tier::current())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_the_current_tier() {
        // Resolving a system path never fails off-Windows; user may lack HOME.
        let expected = default_config_path_for("serve", Tier::current());
        let actual = default_config_path("serve");
        assert_eq!(actual.is_ok(), expected.is_ok());
        if let (Ok(a), Ok(e)) = (actual, expected) {
            assert_eq!(a, e);
        }
    }
}
