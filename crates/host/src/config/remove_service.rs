//! `config remove`: delete a `[[services]]` entry by name.

use std::path::Path;

use anyhow::{Context, Result};

use super::mutate::mutate;
use super::require_existing::require_existing;
use super::resolve_config_path::resolve_config_path;
use super::store::resolve_target;
use super::types::Mutation;
use crate::error::CliError;
use crate::role::Role;

/// Delete the `[[services]]` entry whose `name` matches.
pub fn remove_service(role: Role, config: Option<&Path>, name: &str) -> Result<Mutation> {
    let path = resolve_target(&resolve_config_path(role, config)?);
    // Nothing to remove from a missing config: fail without creating files.
    require_existing(role, &path)?;
    mutate(role, &path, |doc| {
        // No `[services]` table at all → treat as "not found" (config error,
        // exit 2) rather than a generic internal error.
        let not_found =
            || CliError::Config(format!("no service named '{name}' in {}", path.display()));
        let idx = doc
            .get("services")
            .and_then(|i| i.as_array_of_tables())
            .ok_or_else(not_found)?
            .iter()
            .position(|t| t.get("name").and_then(|v| v.as_str()) == Some(name))
            .ok_or_else(not_found)?;
        doc["services"]
            .as_array_of_tables_mut()
            .context("internal: [services] is not an array-of-tables")?
            .remove(idx);
        Ok(())
    })?;
    Ok(Mutation {
        service: name.to_owned(),
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_a_service_drops_its_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        super::super::add_service::add_service(
            Role::Serve,
            &super::super::fixtures::add_web(&path),
        )
        .unwrap();
        let outcome = remove_service(Role::Serve, Some(&path), "web").unwrap();
        assert_eq!(outcome.service, "web");
        assert!(!std::fs::read_to_string(&path).unwrap().contains("web"));
    }

    #[test]
    fn removing_an_unknown_service_is_a_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, "# empty\n").unwrap();
        let err = remove_service(Role::Serve, Some(&path), "web").unwrap_err();
        assert!(err.downcast_ref::<CliError>().is_some(), "{err:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# empty\n");
    }
}
