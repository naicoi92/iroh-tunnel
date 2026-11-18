//! The privilege tier a process runs in.

/// Environment variable (test builds only) that forces the tier:
/// `system` or `user`; any other value falls back to privilege.
#[cfg(feature = "test-utils")]
const ENV_TEST_TIER: &str = "IROH_TUNNEL_TEST_TIER";

/// Where a process lives: a user session or the machine-wide system.
///
/// The tier is a fact of the running process, derived from privilege: an
/// elevated process (root, Administrator) is [`Tier::System`], any other is
/// [`Tier::User`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// A non-elevated process; files live under the user's home.
    User,
    /// An elevated process; files live in machine-wide locations.
    System,
}

impl Tier {
    /// Map the elevation state to a tier (`true` → [`Tier::System`]).
    #[must_use]
    pub fn from_elevated(elevated: bool) -> Self {
        if elevated {
            Self::System
        } else {
            Self::User
        }
    }

    /// The tier of the current process. Test builds (`test-utils`) honour
    /// `IROH_TUNNEL_TEST_TIER=system|user`; production builds never do.
    #[must_use]
    pub fn current() -> Self {
        #[cfg(feature = "test-utils")]
        match std::env::var(ENV_TEST_TIER).as_deref() {
            Ok("system") => return Self::System,
            Ok("user") => return Self::User,
            _ => {}
        }
        Self::from_elevated(crate::service::is_elevated())
    }

    /// Human-readable name: `"user session"` or `"system service"`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "user session",
            Self::System => "system service",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_elevated_maps_privilege() {
        assert_eq!(Tier::from_elevated(true), Tier::System);
        assert_eq!(Tier::from_elevated(false), Tier::User);
    }

    #[test]
    fn labels_name_the_tiers() {
        assert_eq!(Tier::User.label(), "user session");
        assert_eq!(Tier::System.label(), "system service");
    }
}
