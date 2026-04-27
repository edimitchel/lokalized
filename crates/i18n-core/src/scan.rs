//! Project-wide scan of source files for translation key usages.
//!
//! The index built here is the *reverse* of [`crate::LocaleIndex`]: instead
//! of "which file defines a key", it tracks "which keys are referenced from
//! a given source file". The two together drive the unused-translation
//! diagnostic — keys present in locale files but absent from every scanned
//! source.
//!
//! Limitations (intentional):
//!
//! - Only **static literal** keys are detected. Calls like `t(myKey)` where
//!   `myKey` is a variable, or `t(`errors.${code}`)` with template
//!   interpolation, are invisible. The diagnostic is therefore best-effort
//!   and emitted at `Hint` severity by the LSP.
//! - File discovery walks the workspace once at startup. Incremental
//!   updates happen through [`UsageIndex::update_file`], typically called
//!   on `did_open`/`did_change` of source documents.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::framework::find_usages;

/// Directory names skipped during the initial project scan. Matches both
/// JS/TS toolchains (`node_modules`, `.nuxt`, `.next`, …) and generic build
/// outputs (`dist`, `build`, `target`, …).
const DEFAULT_EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    "dist",
    "build",
    "out",
    "target",
    "coverage",
    ".git",
    ".nuxt",
    ".output",
    ".next",
    ".svelte-kit",
    ".turbo",
    ".vercel",
    ".cache",
    ".idea",
    ".vscode",
];

/// Reverse-index: source file → set of translation keys referenced in it.
///
/// Cheap to clone in tests; the LSP keeps a single instance behind an
/// `RwLock` and mutates it incrementally.
#[derive(Clone, Debug, Default)]
pub struct UsageIndex {
    per_file: HashMap<PathBuf, HashSet<String>>,
}

impl UsageIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Walk `root`, scan every supported file, and return the resulting index.
    /// Errors on individual files are silently swallowed — a single unreadable
    /// file should never poison the whole scan.
    pub fn build_from_project(root: &Path) -> Self {
        let mut index = Self::default();
        let walker = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !is_excluded_dir(e.path()));
        for entry in walker {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let Some(lang) = language_id_for(path) else {
                continue;
            };
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            index.update_file(path.to_path_buf(), &content, lang);
        }
        index
    }

    /// Replace the recorded keys for `path` with whatever `content` contains.
    /// If the file has no detectable usages, the entry is removed entirely so
    /// `file_count()` stays accurate.
    pub fn update_file(&mut self, path: PathBuf, content: &str, language_id: &str) {
        let keys: HashSet<String> = find_usages(content, language_id)
            .into_iter()
            .map(|u| u.key)
            .collect();
        if keys.is_empty() {
            self.per_file.remove(&path);
        } else {
            self.per_file.insert(path, keys);
        }
    }

    /// Forget everything recorded for `path` (e.g. file was deleted).
    pub fn remove_file(&mut self, path: &Path) {
        self.per_file.remove(path);
    }

    /// Borrowed view of every key referenced anywhere in the project.
    pub fn used_keys(&self) -> HashSet<&str> {
        self.per_file
            .values()
            .flat_map(|set| set.iter().map(String::as_str))
            .collect()
    }

    /// Cheap membership check that avoids materialising the full union.
    pub fn is_key_used(&self, key: &str) -> bool {
        self.per_file.values().any(|set| set.contains(key))
    }

    pub fn file_count(&self) -> usize {
        self.per_file.len()
    }

    pub fn total_usages(&self) -> usize {
        self.per_file.values().map(HashSet::len).sum()
    }
}

/// Map a path to the Zed language id used by the framework regexes, or
/// `None` if the extension is not one we scan.
fn language_id_for(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    match ext {
        "vue" => Some("Vue.js"),
        "ts" | "mts" | "cts" => Some("TypeScript"),
        "tsx" => Some("TSX"),
        "js" | "mjs" | "cjs" => Some("JavaScript"),
        "jsx" => Some("JSX"),
        "html" | "htm" => Some("HTML"),
        _ => None,
    }
}

/// True if the directory entry at `path` should be skipped during the walk.
/// Used by `WalkDir::filter_entry` to prune subtrees, so subfiles are never
/// touched when their parent matches.
fn is_excluded_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    DEFAULT_EXCLUDED_DIRS.iter().any(|d| *d == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn update_file_collects_literal_keys_from_vue() {
        let mut idx = UsageIndex::new();
        idx.update_file(
            PathBuf::from("/x.vue"),
            r#"<template>{{ $t("common.submit") }}{{ $t("common.cancel") }}</template>"#,
            "Vue.js",
        );
        let used = idx.used_keys();
        assert!(used.contains("common.submit"));
        assert!(used.contains("common.cancel"));
        assert_eq!(idx.file_count(), 1);
        assert_eq!(idx.total_usages(), 2);
    }

    #[test]
    fn update_file_with_no_usages_drops_entry() {
        let mut idx = UsageIndex::new();
        idx.update_file(
            PathBuf::from("/a.ts"),
            r#"const x = format("hello");"#,
            "TypeScript",
        );
        assert_eq!(idx.file_count(), 0);
    }

    #[test]
    fn update_file_replaces_previous_keys() {
        let mut idx = UsageIndex::new();
        let p = PathBuf::from("/a.ts");
        idx.update_file(p.clone(), r#"t("a.b");"#, "TypeScript");
        idx.update_file(p.clone(), r#"t("c.d");"#, "TypeScript");
        assert!(!idx.is_key_used("a.b"));
        assert!(idx.is_key_used("c.d"));
    }

    #[test]
    fn remove_file_drops_keys() {
        let mut idx = UsageIndex::new();
        let p = PathBuf::from("/a.ts");
        idx.update_file(p.clone(), r#"t("a.b");"#, "TypeScript");
        assert!(idx.is_key_used("a.b"));
        idx.remove_file(&p);
        assert!(!idx.is_key_used("a.b"));
    }

    #[test]
    fn build_from_project_walks_supported_extensions() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("a.vue"),
            r#"<template>{{ $t("a.x") }}</template>"#,
        )
        .unwrap();
        fs::write(dir.path().join("b.ts"), r#"t("b.y");"#).unwrap();
        fs::write(dir.path().join("c.md"), "ignored").unwrap();

        let idx = UsageIndex::build_from_project(dir.path());
        assert!(idx.is_key_used("a.x"));
        assert!(idx.is_key_used("b.y"));
        assert_eq!(idx.file_count(), 2);
    }

    #[test]
    fn build_from_project_skips_excluded_dirs() {
        let dir = TempDir::new().unwrap();
        let nm = dir.path().join("node_modules/lib");
        fs::create_dir_all(&nm).unwrap();
        fs::write(nm.join("dep.vue"), r#"$t("vendor.key")"#).unwrap();
        fs::write(dir.path().join("app.vue"), r#"$t("app.key")"#).unwrap();

        let idx = UsageIndex::build_from_project(dir.path());
        assert!(idx.is_key_used("app.key"));
        assert!(!idx.is_key_used("vendor.key"));
    }
}
