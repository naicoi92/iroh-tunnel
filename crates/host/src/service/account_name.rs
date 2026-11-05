//! Display name of the account owning a service registration.

/// The user name for `uid`, or `"?"` when no such account exists (or on
/// platforms without a user database). Used by frontends to describe a
/// registration's owner in prompts.
#[cfg(unix)]
pub fn account_name(uid: u32) -> String {
    super::account::by_uid(uid).map_or_else(|| "?".to_owned(), |a| a.name)
}

/// See the Unix variant; there is no user database to consult here.
#[cfg(not(unix))]
pub fn account_name(_uid: u32) -> String {
    "?".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(unix))]
    #[test]
    fn without_a_user_database_the_name_is_a_question_mark() {
        assert_eq!(account_name(0), "?");
    }

    #[cfg(unix)]
    #[test]
    fn root_is_named_and_a_name_is_never_empty() {
        assert_eq!(account_name(0), "root");
        assert!(!account_name(u32::MAX - 1).is_empty());
    }
}
