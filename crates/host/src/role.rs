//! The two roles a host operation addresses.

/// Which role an operation addresses.
///
/// The two roles share every host mechanic (config file, status file,
/// ownership lock) and differ in file names and engine settings. It is
/// `Copy` and carries no resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The role that forwards tunnel connections to local targets.
    Serve,
    /// The role that exposes remote services on local listeners.
    Access,
}

impl Role {
    /// File name of this role's status file.
    pub fn status_file_name(self) -> &'static str {
        match self {
            Role::Serve => "serve-status.json",
            Role::Access => "access-status.json",
        }
    }

    /// The role's CLI name (`"serve"` / `"access"`).
    pub fn name(self) -> &'static str {
        match self {
            Role::Serve => "serve",
            Role::Access => "access",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_status_files_are_role_scoped() {
        assert_eq!(Role::Serve.name(), "serve");
        assert_eq!(Role::Access.name(), "access");
        assert_eq!(Role::Serve.status_file_name(), "serve-status.json");
        assert_eq!(Role::Access.status_file_name(), "access-status.json");
    }
}
