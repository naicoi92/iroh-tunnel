//! `config list`: the service rows of a role config.

use std::path::Path;

use anyhow::Result;

use super::require_existing::require_existing;
use super::resolve_config_path::resolve_config_path;
use super::store::parse_doc;
use super::types::{ServiceList, ServiceRow};
use crate::role::Role;

/// Read the configured services. The config must exist ([`require_existing`]).
pub fn list_services(role: Role, config: Option<&Path>) -> Result<ServiceList> {
    let path = resolve_config_path(role, config)?;
    let doc = parse_doc(&require_existing(role, &path)?, &path)?;
    let rows = doc
        .get("services")
        .and_then(|i| i.as_array_of_tables())
        .map(|services| {
            let text = |t: &toml_edit::Table, key: &str| {
                t.get(key).and_then(|v| v.as_str()).map(str::to_owned)
            };
            services
                .iter()
                .map(|t| ServiceRow {
                    name: text(t, "name"),
                    node_id: text(t, "node_id"),
                    protocol: text(t, "protocol"),
                    host: text(t, "host"),
                    port: t.get("port").and_then(|v| v.as_integer()),
                })
                .collect()
        });
    Ok(ServiceList { path, rows })
}

#[cfg(test)]
mod tests {
    use super::super::add_service::add_service;
    use super::super::fixtures::add_web;
    use super::*;

    #[test]
    fn list_services_returns_rows_without_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        add_service(Role::Serve, &add_web(&path)).unwrap();
        let list = list_services(Role::Serve, Some(&path)).unwrap();
        assert_eq!(list.path, path);
        assert_eq!(
            list.rows,
            Some(vec![ServiceRow {
                name: Some("web".into()),
                node_id: None,
                protocol: Some("tcp".into()),
                host: Some("127.0.0.1".into()),
                port: Some(8080),
            }])
        );

        let empty = dir.path().join("empty.toml");
        std::fs::write(&empty, "# nothing\n").unwrap();
        assert_eq!(list_services(Role::Serve, Some(&empty)).unwrap().rows, None);
    }
}
