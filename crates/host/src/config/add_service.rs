//! `config add`: append one `[[services]]` entry.

use anyhow::Result;

use super::mutate::{find_service, mutate};
use super::resolve_config_path::resolve_config_path;
use super::store::resolve_target;
use super::types::{AddService, Mutation};
use crate::error::CliError;
use crate::role::Role;

/// Append a `[[services]]` entry.
///
/// Validates the new service (name regex/length, port range, node_id for
/// access, duplicate-name check) by re-loading the mutated document through the
/// schema. A duplicate or invalid entry is a config error → exit 2.
pub fn add_service(role: Role, req: &AddService) -> Result<Mutation> {
    let path = resolve_target(&resolve_config_path(role, req.config.as_deref())?);
    mutate(role, &path, |doc| {
        // Reject a duplicate name up front with a clear message.
        if find_service(doc, &req.name).is_some() {
            return Err(CliError::Config(format!(
                "a service named '{}' already exists in {}",
                req.name,
                path.display()
            ))
            .into());
        }

        let mut entry = toml_edit::Table::new();
        entry["name"] = toml_edit::value(req.name.as_str());
        entry["protocol"] = toml_edit::value(req.protocol.as_str());
        entry["host"] = toml_edit::value(req.host.as_str());
        entry["port"] = toml_edit::value(i64::from(req.port));
        if role == Role::Access {
            let node_id = req.node_id.as_deref().ok_or_else(|| {
                CliError::Config("--node-id is required when adding an access service".into())
            })?;
            entry["node_id"] = toml_edit::value(node_id);
        }

        // Append as an array-of-tables element: [[services]].
        doc["services"].or_insert(toml_edit::Item::ArrayOfTables(Default::default()));
        let services = doc["services"].as_array_of_tables_mut().ok_or_else(|| {
            CliError::Config(format!(
                "invalid config at {}: [services] must be an array of tables",
                path.display()
            ))
        })?;
        services.push(entry);
        Ok(())
    })?;
    Ok(Mutation {
        service: req.name.clone(),
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::add_web;
    use super::*;

    #[test]
    fn add_service_returns_name_and_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        let outcome = add_service(Role::Serve, &add_web(&path)).unwrap();
        assert_eq!(
            outcome,
            Mutation {
                service: "web".into(),
                path: path.clone()
            }
        );
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("name = \"web\""));
    }

    #[test]
    fn invalid_access_add_leaves_bytes_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("access.toml");
        let before = "# keep me\n";
        std::fs::write(&path, before).unwrap();
        let err = add_service(Role::Access, &add_web(&path)).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<CliError>(),
            Some(CliError::Config(_))
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn add_service_accepts_non_utf8_config_path() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join(std::ffi::OsStr::from_bytes(b"serve-\xff.toml"));
        // APFS (macOS) rejects non-UTF-8 names outright; there is nothing to
        // test on a filesystem that cannot hold such a path.
        if std::fs::write(&path, "").is_err() {
            return;
        }
        std::fs::remove_file(&path).unwrap();
        let outcome = add_service(Role::Serve, &add_web(&path)).unwrap();
        assert_eq!(outcome.path, path);
        assert!(path.exists());
    }
}
