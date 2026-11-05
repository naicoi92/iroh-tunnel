//! The scope an unspecified request falls back to.

use super::types::ServiceScope;

/// Scope for verbs that act on an existing service when no flag is given:
/// the elevated installer owns a system service, everyone else a user one.
pub fn default_scope(elevated: bool) -> ServiceScope {
    if elevated {
        ServiceScope::System
    } else {
        ServiceScope::User
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scope_follows_privilege() {
        assert_eq!(default_scope(true), ServiceScope::System);
        assert_eq!(default_scope(false), ServiceScope::User);
    }
}
