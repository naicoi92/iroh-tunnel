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

/// See the Unix variant: a SID resolves to `DOMAIN\name` via
/// `LookupAccountSidW`; a uid has no meaning here.
#[cfg(windows)]
pub fn account_name(owner: &PinOwner) -> String {
    use super::trust_windows::{account_of, model::Sid};
    match owner {
        PinOwner::Sid(sid) => account_of(&Sid(sid.clone())).unwrap_or_else(|| "?".to_owned()),
        PinOwner::Uid(_) => "?".to_owned(),
    }
}

/// See the Unix variant; no account database on this target.
#[cfg(all(not(unix), not(windows)))]
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

    #[cfg(windows)]
    #[test]
    fn system_sid_is_named() {
        assert!(account_name(&PinOwner::Sid("S-1-5-18".into())).ends_with("\\SYSTEM"));
    }
}
