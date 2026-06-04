use std::path::PathBuf;

use anyhow::{bail, Context as _};
use i18n_core::{
    parse_linked_message, resolve_value, set_key_json, Locale, ProjectError, ProjectSnapshot,
    ResolvedValue,
};
use serde::Serialize;

use crate::output::{emit, emit_check, OutputFormat};

pub struct Context {
    pub format: OutputFormat,
    pub snapshot: ProjectSnapshot,
}

impl Context {
    pub fn load(workspace: PathBuf, format: OutputFormat) -> anyhow::Result<Self> {
        let snapshot = ProjectSnapshot::load(&workspace)
            .map_err(|e| map_project_error(workspace.clone(), e))?;
        Ok(Self { format, snapshot })
    }
}

fn map_project_error(workspace: PathBuf, err: ProjectError) -> anyhow::Error {
    match err {
        ProjectError::WorkspaceNotFound(p) => {
            anyhow::anyhow!("workspace not found: {}", p.display())
        }
        ProjectError::NoLocalePaths => anyhow::anyhow!(
            "no locale directories under {}; add locales/ or .zed/lokalized.json",
            workspace.display()
        ),
        ProjectError::Index(e) => anyhow::Error::from(e),
        ProjectError::Parse { path, source } => {
            anyhow::anyhow!("{}: {}", path.display(), source)
        }
    }
}

pub fn run_check(ctx: &Context, locale: Option<&str>) -> anyhow::Result<i32> {
    let only = locale.map(Locale::new);
    let missing = ctx.snapshot.missing_by_locale(only.as_ref());
    let unused = ctx.snapshot.unused_keys();
    let parse_errors = ctx.snapshot.validate();
    let ok = missing.is_empty() && unused.is_empty() && parse_errors.is_empty();
    emit_check(ctx.format, missing, unused, parse_errors)?;
    Ok(if ok { 0 } else { 1 })
}

pub fn run_validate(ctx: &Context) -> anyhow::Result<i32> {
    let issues = ctx.snapshot.validate();
    let payload: Vec<_> = issues
        .iter()
        .map(|i| serde_json::json!({ "path": i.path.display().to_string(), "message": i.message }))
        .collect();
    emit(ctx.format, &payload)?;
    Ok(if issues.is_empty() { 0 } else { 1 })
}

pub fn run_missing(ctx: &Context, locale: Option<&str>) -> anyhow::Result<i32> {
    let only = locale.map(Locale::new);
    let missing = ctx.snapshot.missing_by_locale(only.as_ref());
    emit(ctx.format, &missing)?;
    let empty = missing.values().all(|v| v.is_empty());
    Ok(if empty { 0 } else { 1 })
}

pub fn run_unused(ctx: &Context) -> anyhow::Result<i32> {
    let unused = ctx.snapshot.unused_keys();
    emit(ctx.format, &unused)?;
    Ok(if unused.is_empty() { 0 } else { 1 })
}

pub fn run_keys_list(
    ctx: &Context,
    prefix: Option<&str>,
    locale: Option<&str>,
) -> anyhow::Result<i32> {
    let prefix = prefix.unwrap_or("");
    let locale_filter = locale.map(Locale::new);
    let mut keys: Vec<String> = ctx
        .snapshot
        .index
        .all_keys()
        .into_iter()
        .filter(|k| prefix.is_empty() || k.starts_with(prefix))
        .filter(|k| {
            locale_filter
                .as_ref()
                .is_none_or(|loc| ctx.snapshot.index.lookup(k).contains_key(loc))
        })
        .collect();
    keys.sort();
    emit(ctx.format, &keys)?;
    Ok(0)
}

#[derive(Serialize)]
struct GetResponse {
    key: String,
    locale: String,
    value: String,
    file: String,
    resolved: Option<String>,
    link: Option<LinkInfo>,
}

#[derive(Serialize)]
struct LinkInfo {
    target: String,
    chain: Vec<String>,
}

pub fn run_get(ctx: &Context, key: &str, locale: &str) -> anyhow::Result<i32> {
    let loc = Locale::new(locale);
    let values = ctx.snapshot.index.lookup(key);
    let Some(value) = values.get(&loc) else {
        bail!("key `{key}` not defined for locale `{locale}`");
    };
    let (resolved, link) = match resolve_value(&ctx.snapshot.index, &loc, &value.value) {
        ResolvedValue::Literal { text } => (Some(text.to_string()), None),
        ResolvedValue::Linked {
            display,
            target_key,
            chain,
            ..
        } => (
            Some(display),
            Some(LinkInfo {
                target: target_key,
                chain,
            }),
        ),
        ResolvedValue::Broken { target_key, reason } => (
            Some(format!("(broken link to `{target_key}`: {reason})")),
            None,
        ),
    };
    let out = GetResponse {
        key: key.to_string(),
        locale: locale.to_string(),
        value: value.value.clone(),
        file: value.file.display().to_string(),
        resolved,
        link,
    };
    if parse_linked_message(&value.value).is_some() {
        // keep raw value in `value`; resolved shows chain
    }
    emit(ctx.format, &out)?;
    Ok(0)
}

#[derive(Serialize)]
struct StatsRow {
    locale: String,
    keys: usize,
    missing_from_source: usize,
    coverage_percent: f64,
}

pub fn run_stats(ctx: &Context) -> anyhow::Result<i32> {
    let source = &ctx.snapshot.index.source_locale;
    let total_source = count_keys_in_tree(ctx.snapshot.index.trees.get(source));
    let mut rows = Vec::new();
    for locale in ctx.snapshot.locales() {
        let key_count = count_keys_in_tree(ctx.snapshot.index.trees.get(locale));
        let missing = if locale == source {
            0
        } else {
            ctx.snapshot.index.missing_keys(locale).len()
        };
        let coverage = if total_source == 0 {
            100.0
        } else {
            ((total_source.saturating_sub(missing)) as f64 / total_source as f64) * 100.0
        };
        rows.push(StatsRow {
            locale: locale.as_str().to_string(),
            keys: key_count,
            missing_from_source: missing,
            coverage_percent: (coverage * 10.0).round() / 10.0,
        });
    }
    emit(ctx.format, &rows)?;
    Ok(0)
}

fn count_keys_in_tree(tree: Option<&i18n_core::KeyTree>) -> usize {
    let Some(tree) = tree else {
        return 0;
    };
    let mut n = 0;
    walk_tree(tree, &mut n);
    n
}

fn walk_tree(tree: &i18n_core::KeyTree, count: &mut usize) {
    for node in tree.children.values() {
        match node {
            i18n_core::KeyNode::Leaf(_) => *count += 1,
            i18n_core::KeyNode::Branch(sub) => walk_tree(sub, count),
        }
    }
}

pub fn run_set(ctx: &Context, key: &str, locale: &str, value: &str) -> anyhow::Result<i32> {
    let loc = Locale::new(locale);
    let path = ctx
        .snapshot
        .index
        .locale_file_path(&loc)
        .with_context(|| format!("no locale file for `{locale}`"))?;

    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if !matches!(ext, "json" | "jsonc" | "json5") {
        bail!("set only supports JSON locale files right now (got .{ext})");
    }

    let content = std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    let segments: Vec<&str> = key.split('.').collect();
    let new_content =
        set_key_json(&content, &segments, value).map_err(|e| anyhow::anyhow!("{e}"))?;
    std::fs::write(&path, &new_content).with_context(|| path.display().to_string())?;

    let out = serde_json::json!({
        "key": key,
        "locale": locale,
        "file": path.display().to_string(),
        "written": true,
    });
    emit(ctx.format, &out)?;
    Ok(0)
}

pub fn workspace_path(workspace: Option<PathBuf>) -> PathBuf {
    workspace.unwrap_or_else(|| std::env::current_dir().expect("cwd"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn nested_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../i18n-core/tests/fixtures/nested_project")
    }

    #[test]
    fn check_fails_on_missing_keys() {
        let ctx = Context::load(nested_fixture(), OutputFormat::Json).unwrap();
        let code = run_check(&ctx, None).unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn validate_and_missing_pass_on_flat_project() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../i18n-core/tests/fixtures/flat_project");
        let ctx = Context::load(root, OutputFormat::Text).unwrap();
        assert_eq!(run_validate(&ctx).unwrap(), 0);
        assert_eq!(run_missing(&ctx, None).unwrap(), 0);
    }

    #[test]
    fn check_fails_when_keys_are_unused_without_source_scan_hits() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../i18n-core/tests/fixtures/flat_project");
        let ctx = Context::load(root, OutputFormat::Text).unwrap();
        // Fixture has locale files only — every key appears unused.
        assert_eq!(run_check(&ctx, None).unwrap(), 1);
    }
}
