//! Domain error categories for exit-code mapping (Page 06 v5 §6).
//!
//! - 0 = success
//! - 1 = general error (anyhow, unclassified)
//! - 2 = config error   → [`CliError::Config`]
//! - 3 = permission     → [`CliError::Permission`]
//! - 4 = iroh endpoint  → [`CliError::Iroh`]
//! - 5 = service        → [`CliError::Service`]

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)] // variants are constructed in later tasks (T-02..T-12)
pub enum CliError {
    #[error("config error: {0}")]
    Config(String),

    #[error("permission error: {0}")]
    Permission(String),

    #[error("iroh error: {0}")]
    Iroh(String),

    #[error("service error: {0}")]
    Service(String),
}
