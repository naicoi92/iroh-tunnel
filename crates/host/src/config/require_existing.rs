//! The read commands' existence check on a config path.

use std::path::Path;

use anyhow::Result;

use super::store::read_optional;
use crate::error::CliError;
use crate::role::Role;

/// Read `path`, which a read command needs to exist (D3).
///
/// # Errors
///
/// [`CliError::Config`] with guidance naming the path and the commands that
/// create a configuration. Nothing is created and no identity is generated.
pub fn require_existing(role: Role, path: &Path) -> Result<String> {
    read_optional(path)?.ok_or_else(|| {
        CliError::Config(format!(
            "no configuration at {path}. Create one with `config add` or `config keygen` \
             (`iroh-tunnel {role} config add`, or `iroh-tunnel-{role} config add`), or pass --config",
            path = path.display(),
            role = role.name()
        ))
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_is_a_config_error_naming_the_path_and_commands() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        let err = require_existing(Role::Serve, &path).unwrap_err();
        let Some(CliError::Config(msg)) = err.downcast_ref::<CliError>() else {
            panic!("expected Config, got {err:?}");
        };
        assert!(msg.contains(&path.display().to_string()));
        assert!(msg.contains("config add"));
        assert!(!path.exists(), "nothing may be created");
    }
}
