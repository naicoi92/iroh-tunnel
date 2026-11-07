//! Display name of the account owning a service registration.

use super::pin::PinOwner;

/// The account name for `owner`, or `"?"` when it cannot be resolved (no such
/// uid, no user database, or a SID this platform cannot look up). Used by
/// frontends to describe a registration's owner in prompts.
#[cfg(unix)]
pub fn account_name(owner: &PinOwner) -> String {
    match owner {
        PinOwner::Uid(uid) => {
            super::account::by_uid(*uid).map_or_else(|| "?".to_owned(), |a| a.name)
        }
        PinOwner::Sid(_) => "?".to_owned(),
    }
}

/// See the Unix variant. SID lookup arrives with the Win32 adapter.
#[cfg(not(unix))]
pub fn account_name(_owner: &PinOwner) -> String {
    "?".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unresolvable_sid_is_a_question_mark() {
        assert_eq!(account_name(&PinOwner::Sid("S-1-5-21-9-9-9-1".into())), "?");
    }

    #[cfg(unix)]
    #[test]
    fn root_is_named_and_a_name_is_never_empty() {
        assert_eq!(account_name(&PinOwner::Uid(0)), "root");
        assert!(!account_name(&PinOwner::Uid(u32::MAX - 1)).is_empty());
    }
}
