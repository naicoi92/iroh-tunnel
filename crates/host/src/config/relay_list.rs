//! Helpers for `relay_urls` lists shared by `[node]` and `[[services]]` edits.

use std::collections::HashSet;

use anyhow::Result;
use toml_edit::{Item, TableLike, Value};

use crate::error::CliError;

/// Reject a `relay_urls` list that repeats a URL.
pub(super) fn ensure_unique(urls: &[String]) -> Result<()> {
    let mut seen = HashSet::new();
    for url in urls {
        if !seen.insert(url.as_str()) {
            return Err(CliError::Config(format!("duplicate relay_url '{url}'")).into());
        }
    }
    Ok(())
}

/// Render a list as `[a, b]`, or `empty` when it has no URLs.
pub(super) fn display_list(urls: &[String], empty: &str) -> String {
    if urls.is_empty() {
        return empty.to_owned();
    }
    format!("[{}]", urls.join(", "))
}

/// Read `relay_urls`; `None` when the key is absent.
pub(super) fn read_list(table: &dyn TableLike) -> Option<Vec<String>> {
    let array = table.get("relay_urls")?.as_array()?;
    Some(
        array
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
    )
}

/// Write `relay_urls` in place as an inline array, keeping the key's decor.
pub(super) fn write_list(table: &mut dyn TableLike, urls: &[String]) {
    let mut value = Value::Array(urls.iter().map(String::as_str).collect());
    match table.get_mut("relay_urls") {
        Some(item) => {
            if let Some(old) = item.as_value() {
                *value.decor_mut() = old.decor().clone();
            }
            *item = Item::Value(value);
        }
        None => {
            table.insert("relay_urls", Item::Value(value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_unique_rejects_duplicate() {
        let urls = vec![
            "https://a.example".to_owned(),
            "https://a.example".to_owned(),
        ];
        let err = ensure_unique(&urls).unwrap_err();
        assert!(
            err.to_string()
                .contains("duplicate relay_url 'https://a.example'"),
            "{err:#}"
        );
    }

    #[test]
    fn display_list_formats() {
        let urls = vec![
            "https://a.example".to_owned(),
            "https://b.example".to_owned(),
        ];
        assert_eq!(
            display_list(&urls, "(default)"),
            "[https://a.example, https://b.example]"
        );
        assert_eq!(display_list(&[], "(default)"), "(default)");
    }
}
