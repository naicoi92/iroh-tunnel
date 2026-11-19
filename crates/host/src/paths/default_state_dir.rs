//! Default directory of the status files in the current process tier.

use std::path::PathBuf;

use anyhow::Result;

use super::default_state_dir_for;
use crate::Tier;

/// Default directory holding the `<role>-status.json` files, in the tier of
/// the current process ([`Tier::current`]); see [`default_state_dir_for`].
///
/// # Errors
///
/// As [`default_state_dir_for`].
pub fn default_state_dir() -> Result<PathBuf> {
    default_state_dir_for(Tier::current())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_the_current_tier() {
        let expected = default_state_dir_for(Tier::current());
        let actual = default_state_dir();
        assert_eq!(actual.is_ok(), expected.is_ok());
        if let (Ok(a), Ok(e)) = (actual, expected) {
            assert_eq!(a, e);
        }
    }
}
