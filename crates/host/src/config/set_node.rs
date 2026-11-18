//! `config node`: edit the `[node]` relay settings.

use anyhow::Result;
use toml_edit::{DocumentMut, Item, TableLike, Value};

use super::mutate::{mutate_deciding, validate_role_doc, Edit};
use super::relay_list::{ensure_unique, read_list, write_list};
use super::resolve_config_path::resolve_config_path;
use super::store::{node_table_mut, resolve_target};
use super::types::{NodeChange, RelayEdit, SetNode, SetNodeOutcome, TokenChange, TokenEdit};
use crate::error::CliError;
use crate::role::Role;

/// Change `[node].relay_urls` and `[node].relay_token` as `req` asks.
///
/// Absent keys and empty values are distinct: `Clear` on `relay_urls = []`
/// is a change that removes the key. When nothing differs the file is left
/// untouched, but still validated so an invalid config keeps failing.
///
/// # Errors
///
/// Fails when the config cannot be read, locked or written, or the result does not validate.
pub fn set_node(role: Role, req: &SetNode) -> Result<SetNodeOutcome> {
    let path = resolve_target(&resolve_config_path(role, req.config.as_deref())?);
    mutate_deciding(role, &path, |doc| {
        let (urls, token) = current(doc);
        let plan = plan(urls, token, req)?;
        if plan.changes.is_empty() {
            validate_role_doc(role, &doc.to_string(), &path)?;
            return Ok(Edit::Keep(SetNodeOutcome::Unchanged { path: path.clone() }));
        }
        apply(doc, &path, &plan)?;
        Ok(Edit::Apply(SetNodeOutcome::Updated {
            path: path.clone(),
            changes: plan.changes,
        }))
    })
}

/// What one `[node]` key should become.
#[derive(PartialEq, Eq)]
enum Slot<T> {
    /// Leave the key as it is.
    Keep,
    /// Write this value.
    Set(T),
    /// Remove the key.
    Remove,
}

/// The planned edit of `[node]`: the per-key writes and the report.
/// Not `Debug`: `token` holds the secret and must never be formattable.
#[derive(PartialEq, Eq)]
struct NodePlan {
    relays: Slot<Vec<String>>,
    token: Slot<String>,
    changes: Vec<NodeChange>,
}

/// Current `(relay_urls, relay_token)` of `[node]`, without creating the table.
fn current(doc: &DocumentMut) -> (Option<Vec<String>>, Option<String>) {
    let Some(node) = doc.get("node").and_then(Item::as_table_like) else {
        return (None, None);
    };
    let token = node
        .get("relay_token")
        .and_then(Item::as_str)
        .map(str::to_owned);
    (read_list(node), token)
}

/// Compare the current values with `req` and decide each key's write.
fn plan(urls: Option<Vec<String>>, token: Option<String>, req: &SetNode) -> Result<NodePlan> {
    let urls_target = match &req.relays {
        RelayEdit::Keep => urls.clone(),
        RelayEdit::Replace(new) => {
            ensure_unique(new)?;
            Some(new.clone())
        }
        RelayEdit::Clear => None,
    };
    let token_target = match &req.token {
        TokenEdit::Keep => token.clone(),
        TokenEdit::Replace(new) => Some(new.clone()),
        TokenEdit::Delete => None,
    };
    if matches!(req.token, TokenEdit::Replace(_)) && urls_target.as_ref().is_none_or(Vec::is_empty)
    {
        return Err(
            CliError::Config("relay_token needs relay_urls; set them with -r".into()).into(),
        );
    }

    let mut changes = Vec::new();
    let relays = if urls_target == urls {
        Slot::Keep
    } else {
        changes.push(NodeChange::RelayUrls {
            old: urls.clone().unwrap_or_default(),
            new: urls_target.clone().unwrap_or_default(),
        });
        match urls_target {
            Some(list) => Slot::Set(list),
            None => Slot::Remove,
        }
    };
    let token_change = match (&token, &token_target) {
        (old, new) if old == new => None,
        (None, Some(_)) => Some(TokenChange::Set),
        (Some(_), None) => Some(TokenChange::Deleted),
        _ => Some(TokenChange::Replaced),
    };
    let token_slot = match (token_change, token_target) {
        (None, _) => Slot::Keep,
        (Some(change), target) => {
            changes.push(NodeChange::RelayToken(change));
            target.map_or(Slot::Remove, Slot::Set)
        }
    };
    Ok(NodePlan {
        relays,
        token: token_slot,
        changes,
    })
}

/// Write `plan` into `[node]`, creating the table only when a key is set.
fn apply(doc: &mut DocumentMut, path: &std::path::Path, plan: &NodePlan) -> Result<()> {
    let node = node_table_mut(doc, path)?;
    match &plan.relays {
        Slot::Keep => {}
        Slot::Set(urls) => write_list(node, urls),
        Slot::Remove => {
            node.remove("relay_urls");
        }
    }
    match &plan.token {
        Slot::Keep => {}
        Slot::Set(token) => write_str(node, "relay_token", token),
        Slot::Remove => {
            node.remove("relay_token");
        }
    }
    Ok(())
}

/// Set a string key in place, keeping the old value's decor (comments).
fn write_str(table: &mut dyn TableLike, key: &str, text: &str) {
    let mut value = Value::from(text);
    match table.get_mut(key) {
        Some(item) => {
            if let Some(old) = item.as_value() {
                *value.decor_mut() = old.decor().clone();
            }
            *item = Item::Value(value);
        }
        None => {
            table.insert(key, Item::Value(value));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::config::{NodeChange, RelayEdit, TokenChange, TokenEdit};
    use crate::error::CliError;

    const A: &str = "https://a.example";
    const B: &str = "https://b.example";

    fn file(text: Option<&str>) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        if let Some(text) = text {
            std::fs::write(&path, text).unwrap();
        }
        (dir, path)
    }

    fn req(path: &Path, relays: RelayEdit, token: TokenEdit) -> SetNode {
        SetNode {
            config: Some(path.to_path_buf()),
            relays,
            token,
        }
    }

    fn replace(urls: &[&str]) -> RelayEdit {
        RelayEdit::Replace(urls.iter().map(|u| (*u).to_owned()).collect())
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    fn updated(out: SetNodeOutcome) -> Vec<NodeChange> {
        match out {
            SetNodeOutcome::Updated { changes, .. } => changes,
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    fn assert_config_err(err: &anyhow::Error, needle: &str) {
        assert!(
            matches!(err.downcast_ref::<CliError>(), Some(CliError::Config(_))),
            "{err:#}"
        );
        assert!(err.to_string().contains(needle), "{err:#}");
    }

    #[test]
    fn replace_relay_list() {
        let (_d, path) = file(Some("[node]\nrelay_urls = [\"https://old.example\"]\n"));
        let out = set_node(Role::Serve, &req(&path, replace(&[A, B]), TokenEdit::Keep)).unwrap();
        let doc: toml::Table = toml::from_str(&read(&path)).unwrap();
        let urls = doc["node"]["relay_urls"].as_array().unwrap();
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].as_str(), Some(A));
        assert_eq!(urls[1].as_str(), Some(B));
        assert_eq!(
            updated(out),
            vec![NodeChange::RelayUrls {
                old: vec!["https://old.example".into()],
                new: vec![A.into(), B.into()],
            }]
        );
    }

    #[test]
    fn relays_without_token_delete_token() {
        let (_d, path) = file(Some("[node]\nrelay_token = \"t1\"\n"));
        let out = set_node(Role::Serve, &req(&path, replace(&[A]), TokenEdit::Delete)).unwrap();
        assert!(!read(&path).contains("relay_token"));
        assert_eq!(
            updated(out).last(),
            Some(&NodeChange::RelayToken(TokenChange::Deleted))
        );
    }

    #[test]
    fn relays_and_token_replaced() {
        let (_d, path) = file(Some("[node]\nrelay_token = \"t1\"\n"));
        let out = set_node(
            Role::Serve,
            &req(&path, replace(&[A]), TokenEdit::Replace("t2".into())),
        )
        .unwrap();
        let doc: toml::Table = toml::from_str(&read(&path)).unwrap();
        assert_eq!(doc["node"]["relay_token"].as_str(), Some("t2"));
        assert_eq!(
            updated(out).last(),
            Some(&NodeChange::RelayToken(TokenChange::Replaced))
        );
    }

    #[test]
    fn token_only_keeps_relays() {
        let (_d, path) = file(Some("[node]\nrelay_urls = [\"https://a.example\"]\n"));
        let out = set_node(
            Role::Serve,
            &req(&path, RelayEdit::Keep, TokenEdit::Replace("t3".into())),
        )
        .unwrap();
        let doc: toml::Table = toml::from_str(&read(&path)).unwrap();
        assert_eq!(doc["node"]["relay_urls"][0].as_str(), Some(A));
        assert_eq!(doc["node"]["relay_token"].as_str(), Some("t3"));
        assert_eq!(updated(out), vec![NodeChange::RelayToken(TokenChange::Set)]);
    }

    #[test]
    fn token_only_with_existing_duplicates_is_accepted() {
        let (_d, path) = file(Some(
            "[node]\nrelay_urls = [\"https://a.example\", \"https://a.example\"]\n",
        ));
        let out = set_node(
            Role::Serve,
            &req(&path, RelayEdit::Keep, TokenEdit::Replace("t".into())),
        );
        assert!(out.is_ok(), "{out:?}");
    }

    #[test]
    fn clear_relays_keeps_secret_key() {
        let key = "k".repeat(44);
        let (_d, path) = file(Some(&format!(
            "[node]\nsecret_key = \"{key}\"\nrelay_urls = [\"https://a.example\"]\nrelay_token = \"t\"\n"
        )));
        set_node(
            Role::Serve,
            &req(&path, RelayEdit::Clear, TokenEdit::Delete),
        )
        .unwrap();
        let text = read(&path);
        assert!(!text.contains("relay_urls"), "{text}");
        assert!(!text.contains("relay_token"), "{text}");
        assert!(text.contains(&format!("secret_key = \"{key}\"")), "{text}");
    }

    #[test]
    fn clear_removes_explicit_empty_array() {
        let (_d, path) = file(Some("[node]\nrelay_urls = []\n"));
        let out = set_node(
            Role::Serve,
            &req(&path, RelayEdit::Clear, TokenEdit::Delete),
        )
        .unwrap();
        assert!(!read(&path).contains("relay_urls"));
        assert!(matches!(out, SetNodeOutcome::Updated { .. }));
    }

    #[test]
    fn services_and_comments_preserved() {
        let text = "# operator comment\n[node]\nrelay_urls = []\n\n[[services]]\nname = \"web\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 8080\n";
        let (_d, path) = file(Some(text));
        set_node(Role::Serve, &req(&path, replace(&[A]), TokenEdit::Keep)).unwrap();
        let got = read(&path);
        assert!(got.contains("# operator comment"), "{got}");
        assert!(got.contains("name = \"web\""), "{got}");
        assert!(got.contains(A), "{got}");
    }

    #[test]
    fn missing_file_created_with_node_only() {
        let (_d, path) = file(None);
        set_node(Role::Serve, &req(&path, replace(&[A]), TokenEdit::Keep)).unwrap();
        let doc: toml::Table = toml::from_str(&read(&path)).unwrap();
        assert_eq!(doc.len(), 1, "{doc:?}");
        assert_eq!(doc["node"]["relay_urls"][0].as_str(), Some(A));
    }

    #[test]
    fn clear_on_missing_file_is_unchanged() {
        let (_d, path) = file(None);
        let out = set_node(
            Role::Serve,
            &req(&path, RelayEdit::Clear, TokenEdit::Delete),
        )
        .unwrap();
        assert_eq!(out, SetNodeOutcome::Unchanged { path: path.clone() });
        assert!(!path.exists());
    }

    #[test]
    fn retired_multiplex_dropped() {
        let (_d, path) = file(Some(
            "[[services]]\nname = \"api\"\nprotocol = \"tcp\"\nnode_id = \"0000000000000000000000000000000000000000000000000000000000000000\"\nport = 9\nmultiplex = true\n",
        ));
        set_node(Role::Access, &req(&path, replace(&[A]), TokenEdit::Keep)).unwrap();
        assert!(!read(&path).contains("multiplex"));
    }

    #[test]
    fn duplicate_url_rejected() {
        let text = "[node]\nrelay_urls = []\n";
        let (_d, path) = file(Some(text));
        let err =
            set_node(Role::Serve, &req(&path, replace(&[A, A]), TokenEdit::Keep)).unwrap_err();
        assert_config_err(&err, "duplicate relay_url 'https://a.example'");
        assert_eq!(read(&path), text);
    }

    #[test]
    fn token_without_relays_rejected() {
        let text = "[node]\n";
        let (_d, path) = file(Some(text));
        let err = set_node(
            Role::Serve,
            &req(&path, RelayEdit::Keep, TokenEdit::Replace("t".into())),
        )
        .unwrap_err();
        assert_config_err(&err, "relay_token needs relay_urls");
        assert_eq!(read(&path), text);
    }

    #[test]
    fn insecure_url_rejected() {
        let text = "[node]\n";
        let (_d, path) = file(Some(text));
        let err = set_node(
            Role::Serve,
            &req(&path, replace(&["http://relay.example"]), TokenEdit::Keep),
        )
        .unwrap_err();
        assert_config_err(&err, "invalid relay_url 'http://relay.example'");
        assert_eq!(read(&path), text);
    }

    #[test]
    fn invalid_service_blocks_node_edit() {
        let text =
            "[[services]]\nname = \"BAD\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 8080\n";
        let (_d, path) = file(Some(text));
        let err = set_node(Role::Serve, &req(&path, replace(&[A]), TokenEdit::Keep)).unwrap_err();
        assert_config_err(&err, "invalid config");
        assert_eq!(read(&path), text);
    }

    #[test]
    fn invalid_service_blocks_noop_edit() {
        let text = format!(
            "[node]\nrelay_urls = [\"{A}\"]\n\n[[services]]\nname = \"BAD\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 8080\n"
        );
        let (_d, path) = file(Some(&text));
        let err = set_node(Role::Serve, &req(&path, replace(&[A]), TokenEdit::Delete)).unwrap_err();
        assert_config_err(&err, "invalid config");
        assert_eq!(read(&path), text);
    }

    #[test]
    fn unchanged_keeps_bytes_and_mtime() {
        let text = format!("[node]\nrelay_urls = [\"{A}\"]\n");
        let (_d, path) = file(Some(&text));
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        let out = set_node(Role::Serve, &req(&path, replace(&[A]), TokenEdit::Delete)).unwrap();
        assert_eq!(out, SetNodeOutcome::Unchanged { path: path.clone() });
        assert_eq!(read(&path), text);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );
    }

    #[test]
    fn same_token_is_unchanged() {
        let text = format!("[node]\nrelay_urls = [\"{A}\"]\nrelay_token = \"T\"\n");
        let (_d, path) = file(Some(&text));
        let out = set_node(
            Role::Serve,
            &req(&path, replace(&[A]), TokenEdit::Replace("T".into())),
        )
        .unwrap();
        assert!(matches!(out, SetNodeOutcome::Unchanged { .. }), "{out:?}");
    }

    #[test]
    fn inline_node_table() {
        let (_d, path) = file(Some("node = { relay_urls = [], secret_key = \"\" }\n"));
        set_node(Role::Serve, &req(&path, replace(&[A]), TokenEdit::Keep)).unwrap();
        let got = read(&path);
        assert!(got.contains("secret_key"), "{got}");
        assert!(got.contains(A), "{got}");
    }

    #[test]
    fn token_edit_debug_redacts() {
        let dbg = format!("{:?}", TokenEdit::Replace("s3cret".into()));
        assert!(!dbg.contains("s3cret"), "{dbg}");
    }

    #[test]
    fn token_change_report_never_carries_value() {
        // The plan holds the token, but nothing that is formatted or returned does.
        let (_d, path) = file(Some("[node]\n"));
        let out = set_node(
            Role::Serve,
            &req(&path, replace(&[A]), TokenEdit::Replace("s3cret".into())),
        )
        .unwrap();
        let dbg = format!("{out:?}");
        assert!(!dbg.contains("s3cret"), "{dbg}");
    }
}
