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

use crate::framework::{find_usages, KeyUsage};
use crate::position::Range;

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

/// Reverse-index: source file → list of translation key usages in it.
///
/// Each usage carries the source range of the literal so we can power
/// `textDocument/references` and the per-key occurrence count inlay hint
/// directly from this index, without having to re-scan files on demand.
///
/// Cheap to clone in tests; the LSP keeps a single instance behind an
/// `RwLock` and mutates it incrementally.
#[derive(Clone, Debug, Default)]
pub struct UsageIndex {
    per_file: HashMap<PathBuf, Vec<KeyUsage>>,
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

    /// Replace the recorded usages for `path` with whatever `content` contains.
    /// If the file has no detectable usages, the entry is removed entirely so
    /// `file_count()` stays accurate.
    pub fn update_file(&mut self, path: PathBuf, content: &str, language_id: &str) {
        let usages = find_usages(content, language_id);
        if usages.is_empty() {
            self.per_file.remove(&path);
        } else {
            self.per_file.insert(path, usages);
        }
    }

    /// Forget everything recorded for `path` (e.g. file was deleted).
    pub fn remove_file(&mut self, path: &Path) {
        self.per_file.remove(path);
    }

    /// Borrowed view of every key referenced anywhere in the project. Keys
    /// referenced multiple times are reported once.
    pub fn used_keys(&self) -> HashSet<&str> {
        self.per_file
            .values()
            .flat_map(|usages| usages.iter().map(|u| u.key.as_str()))
            .collect()
    }

    /// Cheap membership check that avoids materialising the full union.
    pub fn is_key_used(&self, key: &str) -> bool {
        self.per_file
            .values()
            .any(|usages| usages.iter().any(|u| u.key == key))
    }

    /// Total number of references to `key` across every scanned file.
    pub fn count_for_key(&self, key: &str) -> usize {
        self.per_file
            .values()
            .map(|usages| usages.iter().filter(|u| u.key == key).count())
            .sum()
    }

    /// Every (file, range) pair where `key` is referenced. The order is not
    /// stable across calls (HashMap iteration), callers that need
    /// determinism should sort.
    pub fn locations_for_key<'a>(&'a self, key: &str) -> Vec<(&'a Path, &'a Range)> {
        let mut out = Vec::new();
        for (path, usages) in &self.per_file {
            for u in usages {
                if u.key == key {
                    out.push((path.as_path(), &u.range));
                }
            }
        }
        out
    }

    /// Aggregate counts for every key in a single pass. Intended for
    /// per-key inlay hints on a locale file: O(N) once instead of N×M
    /// `count_for_key` calls.
    pub fn counts_by_key(&self) -> HashMap<&str, usize> {
        let mut out: HashMap<&str, usize> = HashMap::new();
        for usages in self.per_file.values() {
            for u in usages {
                *out.entry(u.key.as_str()).or_insert(0) += 1;
            }
        }
        out
    }

    pub fn file_count(&self) -> usize {
        self.per_file.len()
    }

    pub fn total_usages(&self) -> usize {
        self.per_file.values().map(Vec::len).sum()
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

    #[test]
    fn count_for_key_includes_duplicates_within_a_file() {
        let mut idx = UsageIndex::new();
        idx.update_file(
            PathBuf::from("/a.vue"),
            r#"<template>{{ $t("common.submit") }}{{ $t("common.submit") }}</template>"#,
            "Vue.js",
        );
        idx.update_file(
            PathBuf::from("/b.vue"),
            r#"<template>{{ $t("common.submit") }}</template>"#,
            "Vue.js",
        );
        assert_eq!(idx.count_for_key("common.submit"), 3);
        assert_eq!(idx.count_for_key("missing"), 0);
    }

    #[test]
    fn locations_for_key_returns_each_occurrence() {
        let mut idx = UsageIndex::new();
        let p = PathBuf::from("/a.vue");
        idx.update_file(
            p.clone(),
            r#"<template>{{ $t("a.x") }}{{ $t("a.x") }}{{ $t("b.y") }}</template>"#,
            "Vue.js",
        );
        let xs = idx.locations_for_key("a.x");
        assert_eq!(xs.len(), 2);
        assert!(xs.iter().all(|(path, _)| *path == p.as_path()));
        // Both ranges must be distinct so callers can produce two LSP
        // Locations rather than collapsing them.
        assert_ne!(xs[0].1.start.offset, xs[1].1.start.offset);

        let ys = idx.locations_for_key("b.y");
        assert_eq!(ys.len(), 1);

        assert!(idx.locations_for_key("nope").is_empty());
    }

    #[test]
    fn counts_by_key_aggregates_all_files_in_one_pass() {
        let mut idx = UsageIndex::new();
        idx.update_file(
            PathBuf::from("/a.vue"),
            r#"{{ $t("a.x") }}{{ $t("b.y") }}"#,
            "Vue.js",
        );
        idx.update_file(
            PathBuf::from("/b.vue"),
            r#"{{ $t("a.x") }}{{ $t("a.x") }}"#,
            "Vue.js",
        );
        let counts = idx.counts_by_key();
        assert_eq!(counts.get("a.x"), Some(&3));
        assert_eq!(counts.get("b.y"), Some(&1));
        assert!(counts.get("missing").is_none());
    }
}
