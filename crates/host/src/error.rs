//! Domain error categories for exit-code mapping (Page 06 v5 §6).
//!
//! - 0 = success
//! - 1 = general error (anyhow, unclassified)
//! - 2 = config error   → [`CliError::Config`]
//! - 3 = permission     → [`CliError::Permission`]
//! - 4 = iroh endpoint  → [`CliError::Iroh`]
//! - 5 = service        → [`CliError::Service`]
//! - 6 = conflict       → [`CliError::Conflict`] (role or identity already running)

/// The error categories a frontend maps to exit codes.
///
/// Host operations return them wrapped in [`anyhow::Error`]; a frontend
/// downcasts to classify. Every variant carries the complete user-facing
/// message (without the category prefix, which `Display` adds).
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Invalid, missing or unusable configuration (exit code 2).
    #[error("config error: {0}")]
    Config(String),

    /// The OS refused access, or a path failed the trust policy (exit code 3).
    #[error("permission error: {0}")]
    Permission(String),

    /// The iroh endpoint could not be created or reached (exit code 4).
    #[error("iroh error: {0}")]
    Iroh(String),

    /// Service management failed or is unsupported here (exit code 5).
    #[error("service error: {0}")]
    Service(String),

    /// A start was refused because the role, or its public identity, is
    /// already running (RO-01..RO-03).
    #[error("start refused: {0}")]
    Conflict(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_prefixes_the_category() {
        assert_eq!(CliError::Config("x".into()).to_string(), "config error: x");
        assert_eq!(
            CliError::Permission("x".into()).to_string(),
            "permission error: x"
        );
        assert_eq!(CliError::Iroh("x".into()).to_string(), "iroh error: x");
        assert_eq!(
            CliError::Service("x".into()).to_string(),
            "service error: x"
        );
        assert_eq!(
            CliError::Conflict("x".into()).to_string(),
            "start refused: x"
        );
    }
}
