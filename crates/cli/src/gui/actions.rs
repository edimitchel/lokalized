//! File mutations triggered from the GUI.

use std::path::Path;

use anyhow::{bail, Context as _};
use i18n_core::{
    insert_key_json, remove_key_json, rename_key_json, set_key_json, Locale, LocaleIndex,
    ProjectSnapshot,
};

pub fn write_value(
    index: &LocaleIndex,
    key: &str,
    locale: &Locale,
    value: &str,
) -> anyhow::Result<()> {
    let path = index
        .locale_file_path(locale)
        .with_context(|| format!("no locale file for `{locale}`"))?;
    ensure_json(&path)?;
    let segments: Vec<&str> = key.split('.').collect();
    let content = std::fs::read_to_string(&path)?;
    let new_content =
        set_key_json(&content, &segments, value).map_err(|e| anyhow::anyhow!("{e}"))?;
    std::fs::write(&path, new_content)?;
    Ok(())
}

pub fn rename_key_project(
    snapshot: &ProjectSnapshot,
    old_key: &str,
    new_key: &str,
) -> anyhow::Result<usize> {
    if old_key == new_key {
        bail!("new key is the same as the old key");
    }
    if new_key.trim().is_empty() || new_key.contains(' ') {
        bail!("key must be a non-empty dotted path without spaces");
    }

    let old_parts: Vec<&str> = old_key.split('.').collect();
    let new_parts: Vec<&str> = new_key.split('.').collect();
    let mut files_touched = 0usize;

    for locale in snapshot.index.trees.keys() {
        if snapshot.index.lookup(old_key).get(locale).is_none() {
            continue;
        }
        let path = snapshot
            .index
            .locale_file_path(locale)
            .with_context(|| format!("no file for locale `{locale}`"))?;
        ensure_json(&path)?;
        let content = std::fs::read_to_string(&path)?;
        let new_content = rename_key_json(&content, &old_parts, &new_parts)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        std::fs::write(&path, new_content)?;
        files_touched += 1;
    }

    if files_touched == 0 {
        bail!("key `{old_key}` not found in any locale");
    }
    Ok(files_touched)
}

pub fn delete_key_project(snapshot: &ProjectSnapshot, key: &str) -> anyhow::Result<usize> {
    let parts: Vec<&str> = key.split('.').collect();
    let mut files_touched = 0usize;

    for locale in snapshot.index.trees.keys() {
        if snapshot.index.lookup(key).get(locale).is_none() {
            continue;
        }
        let path = snapshot
            .index
            .locale_file_path(locale)
            .with_context(|| format!("no file for locale `{locale}`"))?;
        ensure_json(&path)?;
        let content = std::fs::read_to_string(&path)?;
        let new_content = remove_key_json(&content, &parts).map_err(|e| anyhow::anyhow!("{e}"))?;
        std::fs::write(&path, new_content)?;
        files_touched += 1;
    }

    if files_touched == 0 {
        bail!("key `{key}` not found");
    }
    Ok(files_touched)
}

/// Insert `key` in every locale file (source gets `value`, others get copy or empty).
pub fn add_key_project(
    snapshot: &ProjectSnapshot,
    key: &str,
    value_in_source: &str,
) -> anyhow::Result<usize> {
    if key.trim().is_empty() || key.contains(' ') {
        bail!("invalid key path");
    }
    let parts: Vec<&str> = key.split('.').collect();
    let source = &snapshot.index.source_locale;
    let mut files_touched = 0usize;

    for locale in snapshot.index.trees.keys() {
        let path = snapshot
            .index
            .locale_file_path(locale)
            .with_context(|| format!("no file for locale `{locale}`"))?;
        ensure_json(&path)?;
        let content = std::fs::read_to_string(&path)?;
        if value_at_path_exists(&content, &parts) {
            continue;
        }
        let value = if locale == source {
            value_in_source.to_string()
        } else {
            snapshot
                .index
                .lookup(key)
                .get(source)
                .map(|v| v.value.clone())
                .unwrap_or_default()
        };
        let new_content =
            insert_key_json(&content, &parts, &value).map_err(|e| anyhow::anyhow!("{e}"))?;
        std::fs::write(&path, new_content)?;
        files_touched += 1;
    }

    if files_touched == 0 {
        bail!("key already exists in all locales");
    }
    Ok(files_touched)
}

fn value_at_path_exists(content: &str, path: &[&str]) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(content) else {
        return false;
    };
    let mut current = &v;
    for part in path {
        let Some(obj) = current.as_object() else {
            return false;
        };
        current = match obj.get(*part) {
            Some(c) => c,
            None => return false,
        };
    }
    true
}

fn ensure_json(path: &Path) -> anyhow::Result<()> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if matches!(ext, "json" | "jsonc" | "json5") {
        Ok(())
    } else {
        bail!(
            "only JSON locale files are editable in the GUI (got .{ext} for {})",
            path.display()
        )
    }
}
