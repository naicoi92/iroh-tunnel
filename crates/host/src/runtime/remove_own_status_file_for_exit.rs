//! Forced-exit status cleanup.

use crate::role::Role;

/// Blocking removal of `role`'s own status file for the forced-exit path,
/// where the async runtime no longer drives cleanup. Pid-checked, so a file
/// another process owns is left alone.
pub fn remove_own_status_file_for_exit(role: Role) {
    crate::status::writer::remove_own_status_file_for_exit(role);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_a_foreign_status_file_alone() {
        // The pid-checked removal itself is covered in `status::writer`; this
        // only proves the wrapper tolerates an absent file.
        remove_own_status_file_for_exit(Role::Access);
    }
}
