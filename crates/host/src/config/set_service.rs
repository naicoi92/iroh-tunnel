//! `config set`: create or update one `[[services]]` entry by name.

use anyhow::Result;
use toml_edit::{DocumentMut, Item, Table, Value};

use super::display_addr::display_addr;
use super::mutate::{find_service_index, mutate_deciding, Edit};
use super::resolve_config_path::resolve_config_path;
use super::store::resolve_target;
use super::types::{FieldChange, ServiceField, SetError, SetOutcome, SetService};
use crate::error::CliError;
use crate::role::Role;

/// Host assumed when an entry has no `host` key (same default as the schema).
const DEFAULT_HOST: &str = "127.0.0.1";
/// Protocol assumed when an entry has no `protocol` key.
const DEFAULT_PROTOCOL: &str = "tcp";

/// Create the service `req.name`, or update the fields that were passed.
///
/// A missing entry needs the address (and, for access, the node id); a
/// present entry changes only fields whose value differs. The whole document is
/// validated through the role schema before anything is written, so an invalid
/// result is a config error and the file is left unchanged.
pub fn set_service(role: Role, req: &SetService) -> Result<SetOutcome> {
    let path = resolve_target(&resolve_config_path(role, req.config.as_deref())?);
    mutate_deciding(role, &path, |doc| {
        let service = req.name.clone();
        match find_service_index(doc, &req.name, &path)? {
            None => {
                create(doc, role, req)?;
                Ok(Edit::Apply(SetOutcome::Created {
                    service,
                    path: path.clone(),
                }))
            }
            Some(index) => {
                let changes = update(doc, index, req, &path)?;
                if changes.is_empty() {
                    Ok(Edit::Keep(SetOutcome::Unchanged {
                        service,
                        path: path.clone(),
                    }))
                } else {
                    Ok(Edit::Apply(SetOutcome::Updated {
                        service,
                        path: path.clone(),
                        changes,
                    }))
                }
            }
        }
    })
}

fn create(doc: &mut DocumentMut, role: Role, req: &SetService) -> Result<()> {
    let mut missing = Vec::new();
    if req.port.is_none() {
        missing.push(match role {
            Role::Serve => "--target",
            Role::Access => "--listen",
        });
    }
    if role == Role::Access && req.node_id.is_none() {
        missing.push("--node-id");
    }
    let Some(port) = req.port.filter(|_| missing.is_empty()) else {
        return Err(SetError::MissingForCreate {
            name: req.name.clone(),
            flags: missing,
        }
        .into());
    };

    let mut entry = Table::new();
    entry["name"] = toml_edit::value(req.name.as_str());
    entry["protocol"] = toml_edit::value(req.protocol.map_or(DEFAULT_PROTOCOL, |p| p.as_str()));
    entry["host"] = toml_edit::value(req.host.as_deref().unwrap_or(DEFAULT_HOST));
    entry["port"] = toml_edit::value(i64::from(port));
    if role == Role::Access {
        if let Some(node_id) = req.node_id.as_deref() {
            entry["node_id"] = toml_edit::value(node_id);
        }
    }
    doc["services"].or_insert(Item::ArrayOfTables(Default::default()));
    if let Some(services) = doc["services"].as_array_of_tables_mut() {
        services.push(entry);
    }
    Ok(())
}

fn update(
    doc: &mut DocumentMut,
    index: usize,
    req: &SetService,
    path: &std::path::Path,
) -> Result<Vec<FieldChange>> {
    let entry = doc["services"]
        .as_array_of_tables_mut()
        .and_then(|s| s.get_mut(index))
        .ok_or_else(|| CliError::Config(shape_message(path)))?;
    let mut changes = Vec::new();

    if let Some(port) = req.port {
        let cur_host = str_of(entry, "host").unwrap_or(DEFAULT_HOST).to_owned();
        let cur_port = entry
            .get("port")
            .and_then(Item::as_integer)
            .and_then(|p| u16::try_from(p).ok())
            .ok_or_else(|| {
                CliError::Config(format!(
                    "invalid config at {}: service '{}' has no valid port",
                    path.display(),
                    req.name
                ))
            })?;
        let new_host = req.host.as_deref().unwrap_or(&cur_host);
        if new_host != cur_host || port != cur_port {
            if new_host != cur_host {
                put(entry, "host", new_host);
            }
            if port != cur_port {
                put_int(entry, "port", i64::from(port));
            }
            changes.push(FieldChange {
                field: ServiceField::Address,
                old: display_addr(&cur_host, cur_port),
                new: display_addr(new_host, port),
            });
        }
    }

    if let Some(protocol) = req.protocol {
        let cur = str_of(entry, "protocol")
            .unwrap_or(DEFAULT_PROTOCOL)
            .to_owned();
        if cur != protocol.as_str() {
            put(entry, "protocol", protocol.as_str());
            changes.push(FieldChange {
                field: ServiceField::Protocol,
                old: cur,
                new: protocol.as_str().to_owned(),
            });
        }
    }

    if let Some(node_id) = req.node_id.as_deref() {
        let cur = str_of(entry, "node_id").unwrap_or_default().to_owned();
        if cur != node_id {
            put(entry, "node_id", node_id);
            changes.push(FieldChange {
                field: ServiceField::NodeId,
                old: cur,
                new: node_id.to_owned(),
            });
        }
    }
    Ok(changes)
}

fn shape_message(path: &std::path::Path) -> String {
    format!(
        "invalid config at {}: [services] must be an array of tables",
        path.display()
    )
}

fn str_of<'a>(entry: &'a Table, key: &str) -> Option<&'a str> {
    entry.get(key).and_then(Item::as_str)
}

fn put(entry: &mut Table, key: &str, text: &str) {
    replace(entry, key, Value::from(text));
}

fn put_int(entry: &mut Table, key: &str, number: i64) {
    replace(entry, key, Value::from(number));
}

/// Set `key`, keeping the old value's surrounding whitespace and comment.
fn replace(entry: &mut Table, key: &str, mut value: Value) {
    if let Some(old) = entry.get(key).and_then(Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    }
    entry[key] = Item::Value(value);
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{set_web, VALID_SERVE};
    use super::super::Protocol;
    use super::*;

    const NODE: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn file(text: Option<&str>) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg.toml");
        if let Some(text) = text {
            std::fs::write(&path, text).unwrap();
        }
        (dir, path)
    }

    fn req(path: &std::path::Path, name: &str) -> SetService {
        SetService {
            name: name.into(),
            port: None,
            ..set_web(path)
        }
    }

    fn assert_config_err(err: &anyhow::Error) {
        assert!(
            matches!(err.downcast_ref::<CliError>(), Some(CliError::Config(_))),
            "{err:#}"
        );
    }

    #[test]
    fn create_applies_defaults() {
        let (_d, path) = file(None);
        let r = SetService {
            port: Some(3000),
            ..req(&path, "api")
        };
        let out = set_service(Role::Serve, &r).unwrap();
        assert_eq!(
            out,
            SetOutcome::Created {
                service: "api".into(),
                path: path.clone()
            }
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("protocol = \"tcp\""), "{text}");
        assert!(text.contains("host = \"127.0.0.1\""), "{text}");
        assert!(text.contains("port = 3000"), "{text}");
    }

    #[test]
    fn create_access_requires_listen_and_node_id() {
        let before = "# keep\n";
        let (_d, path) = file(Some(before));
        let err = set_service(Role::Access, &req(&path, "api")).unwrap_err();
        match err.downcast_ref::<SetError>() {
            Some(SetError::MissingForCreate { flags, .. }) => {
                assert_eq!(flags, &vec!["--listen", "--node-id"]);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn create_serve_requires_target() {
        let (_d, path) = file(None);
        let err = set_service(Role::Serve, &req(&path, "api")).unwrap_err();
        match err.downcast_ref::<SetError>() {
            Some(SetError::MissingForCreate { flags, .. }) => {
                assert_eq!(flags, &vec!["--target"]);
            }
            other => panic!("unexpected {other:?}"),
        }
        // The parent dir and `.lock` may exist; the config itself must not.
        assert!(!path.exists());
    }

    #[test]
    fn update_port_keeps_host_and_protocol() {
        let (_d, path) = file(Some(
            "[[services]]\nname = \"web\"\nprotocol = \"udp\"\nhost = \"10.0.0.5\"\nport = 3000\n",
        ));
        let r = SetService {
            port: Some(3001),
            ..req(&path, "web")
        };
        let out = set_service(Role::Serve, &r).unwrap();
        assert_eq!(
            out,
            SetOutcome::Updated {
                service: "web".into(),
                path: path.clone(),
                changes: vec![FieldChange {
                    field: ServiceField::Address,
                    old: "10.0.0.5:3000".into(),
                    new: "10.0.0.5:3001".into(),
                }],
            }
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("protocol = \"udp\""), "{text}");
        assert!(text.contains("host = \"10.0.0.5\""), "{text}");
    }

    #[test]
    fn update_host_and_port() {
        let (_d, path) = file(Some(
            "[[services]]\nname = \"web\"\nprotocol = \"tcp\"\nhost = \"10.0.0.5\"\nport = 3000\n",
        ));
        let r = SetService {
            host: Some("127.0.0.1".into()),
            port: Some(4000),
            ..req(&path, "web")
        };
        set_service(Role::Serve, &r).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("host = \"127.0.0.1\""), "{text}");
        assert!(text.contains("port = 4000"), "{text}");
    }

    #[test]
    fn set_bare_port_keeps_ipv6_host() {
        let (_d, path) = file(Some(
            "[[services]]\nname = \"web\"\nprotocol = \"tcp\"\nhost = \"::1\"\nport = 3000\n",
        ));
        let r = SetService {
            port: Some(3001),
            ..req(&path, "web")
        };
        let SetOutcome::Updated { changes, .. } = set_service(Role::Serve, &r).unwrap() else {
            panic!("expected Updated");
        };
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].old, "[::1]:3000");
        assert_eq!(changes[0].new, "[::1]:3001");
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("host = \"::1\""));
    }

    #[test]
    fn absent_host_key_compares_as_default() {
        let (_d, path) = file(Some(
            "[[services]]\nname = \"web\"\nprotocol = \"tcp\"\nport = 3000\n",
        ));
        let r = SetService {
            host: Some("127.0.0.1".into()),
            port: Some(3000),
            ..req(&path, "web")
        };
        assert!(matches!(
            set_service(Role::Serve, &r).unwrap(),
            SetOutcome::Unchanged { .. }
        ));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("host"));
    }

    #[test]
    fn update_keeps_udp_settings_comments_and_decor() {
        let text = "[[services]]\nname = \"web\"\nprotocol = \"udp\"\n# keep\nmax_flows = 5\nport = 3000 # why\n";
        let (_d, path) = file(Some(text));
        let r = SetService {
            port: Some(3001),
            ..req(&path, "web")
        };
        set_service(Role::Serve, &r).unwrap();
        let got = std::fs::read_to_string(&path).unwrap();
        assert!(got.contains("max_flows = 5"), "{got}");
        assert!(got.contains("# keep\n"), "{got}");
        assert!(got.contains("port = 3001 # why"), "{got}");
    }

    #[test]
    fn update_to_tcp_with_udp_settings_is_rejected() {
        let text = "[[services]]\nname = \"web\"\nprotocol = \"udp\"\nmax_flows = 5\nport = 3000\n";
        let (_d, path) = file(Some(text));
        let r = SetService {
            protocol: Some(Protocol::Tcp),
            ..req(&path, "web")
        };
        let err = set_service(Role::Serve, &r).unwrap_err();
        assert_config_err(&err);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn equal_values_are_unchanged_and_not_written() {
        let (_d, path) = file(Some(VALID_SERVE));
        let before_time = std::fs::metadata(&path).unwrap().modified().unwrap();
        let out = set_service(Role::Serve, &set_web(&path)).unwrap();
        assert!(matches!(out, SetOutcome::Unchanged { .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), VALID_SERVE);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before_time
        );
    }

    #[test]
    fn name_only_on_existing_is_unchanged() {
        let (_d, path) = file(Some(VALID_SERVE));
        let out = set_service(Role::Serve, &req(&path, "web")).unwrap();
        assert!(matches!(out, SetOutcome::Unchanged { .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), VALID_SERVE);
    }

    #[test]
    fn set_on_inline_services_array_is_config_error() {
        let text = "services = []\n";
        let (_d, path) = file(Some(text));
        for r in [set_web(&path), req(&path, "web")] {
            let err = set_service(Role::Serve, &r).unwrap_err();
            assert_config_err(&err);
            assert!(err.downcast_ref::<SetError>().is_none());
            assert!(
                err.to_string()
                    .contains("[services] must be an array of tables"),
                "{err:#}"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
    }

    #[test]
    fn invalid_name_is_config_error() {
        let (_d, path) = file(None);
        let r = SetService {
            port: Some(3000),
            ..req(&path, "BAD")
        };
        assert_config_err(&set_service(Role::Serve, &r).unwrap_err());
    }

    #[test]
    fn create_preserves_comments_and_entries() {
        let text = format!("# top\n{VALID_SERVE}");
        let (_d, path) = file(Some(&text));
        let r = SetService {
            port: Some(3000),
            ..req(&path, "api")
        };
        set_service(Role::Serve, &r).unwrap();
        let got = std::fs::read_to_string(&path).unwrap();
        assert!(got.starts_with(&text), "{got}");
        assert!(got.contains("name = \"api\""), "{got}");
    }

    #[test]
    fn access_update_with_invalid_node_id_is_rejected() {
        let text = format!(
            "[[services]]\nname = \"db\"\nprotocol = \"tcp\"\nnode_id = \"{NODE}\"\nport = 5432\n"
        );
        let (_d, path) = file(Some(&text));
        let r = SetService {
            node_id: Some("x".repeat(3)),
            ..req(&path, "db")
        };
        // Short ids fail schema validation; the file must stay unchanged.
        assert_config_err(&set_service(Role::Access, &r).unwrap_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn access_update_node_id_is_reported_and_written() {
        let new = "1".repeat(64);
        let text = format!(
            "[[services]]\nname = \"db\"\nprotocol = \"tcp\"\nnode_id = \"{NODE}\"\nport = 5432\n"
        );
        let (_d, path) = file(Some(&text));
        let r = SetService {
            node_id: Some(new.clone()),
            ..req(&path, "db")
        };
        let out = set_service(Role::Access, &r).unwrap();
        assert_eq!(
            out,
            SetOutcome::Updated {
                service: "db".into(),
                path: path.clone(),
                changes: vec![FieldChange {
                    field: ServiceField::NodeId,
                    old: NODE.into(),
                    new: new.clone(),
                }],
            }
        );
        let got = std::fs::read_to_string(&path).unwrap();
        assert!(got.contains(&format!("node_id = \"{new}\"")), "{got}");
        assert!(!got.contains(NODE), "{got}");
    }

    #[test]
    fn update_protocol_only_is_reported() {
        let (_d, path) = file(Some(
            "[[services]]\nname = \"web\"\nprotocol = \"tcp\"\nport = 3000\n",
        ));
        let r = SetService {
            protocol: Some(Protocol::Udp),
            ..req(&path, "web")
        };
        let SetOutcome::Updated { changes, .. } = set_service(Role::Serve, &r).unwrap() else {
            panic!("expected Updated");
        };
        assert_eq!(
            changes,
            vec![FieldChange {
                field: ServiceField::Protocol,
                old: "tcp".into(),
                new: "udp".into(),
            }]
        );
        let got = std::fs::read_to_string(&path).unwrap();
        assert!(got.contains("protocol = \"udp\""), "{got}");
    }
}
