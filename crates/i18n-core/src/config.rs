//! Project configuration loaded from `.zed/lokalized.json` (with sensible auto-detection).

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::locale::Locale;

/// Directories commonly used to store locale files at the workspace root.
const CANDIDATE_LOCALE_DIRS: &[&str] = &[
    "locales",
    "src/locales",
    "i18n/locales",
    "i18n",
    "public/locales",
    "lib/l10n",
    "app/locales",
    "assets/locales",
];

/// Common locale roots in monorepos (front/back, apps/packages, …).
const MONOREPO_LOCALE_DIRS: &[&str] = &[
    "front/i18n/locales",
    "front/locales",
    "frontend/i18n/locales",
    "frontend/locales",
    "client/i18n/locales",
    "client/locales",
    "web/i18n/locales",
    "web/locales",
    "apps/web/locales",
    "apps/frontend/locales",
    "packages/app/locales",
];

const LOCALE_DIR_NAMES: &[&str] = &["locales", "l10n"];

/// Max depth when scanning a monorepo for nested locale directories.
const DISCOVER_MAX_DEPTH: usize = 8;

/// Directory names skipped while scanning for locale folders.
const DISCOVER_EXCLUDED_DIRS: &[&str] = &[
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
    ".pnpm-store",
    "vendor",
];

/// Configuration for a Lokalized-enabled workspace.
///
/// Loaded from `.zed/lokalized.json`. All fields are optional — missing values
/// fall back to filesystem heuristics. Field names mirror the i18n-ally VSCode
/// extension settings to simplify migration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProjectConfig {
    /// Directories (relative to the workspace root) to scan for locale files.
    pub locale_paths: Vec<String>,

    /// The source locale (defaults to `"en"`).
    pub source_locale: Option<String>,

    /// Explicitly enable/disable frameworks by id; empty means "auto".
    pub enabled_frameworks: Vec<String>,

    /// Key style: `nested` (a.b.c) vs `flat` (literal dotted key), or `auto`.
    pub key_style: Option<KeyStyle>,

    /// When using nested layouts (`fr/slots.json`), should the filename stem
    /// be prepended as the top-level namespace of every key ?
    ///
    /// - `Some(true)` (default) — keys in `slots.json` are indexed under `slots.*`
    ///   (matches i18n-ally with `namespace: true`).
    /// - `Some(false)` — keys are indexed exactly as they appear; use this when
    ///   each JSON already wraps its content in a namespace key (matches
    ///   `i18n-ally.namespace: false`).
    pub namespace: Option<bool>,
}

impl ProjectConfig {
    /// Whether the filename stem should be prepended as the top-level namespace.
    pub fn use_file_namespace(&self) -> bool {
        self.namespace.unwrap_or(true)
    }

    /// Resolve `locale_paths` (which are relative to the workspace root) to
    /// absolute paths, keeping only directories that actually exist.
    /// Useful for the filesystem watcher.
    pub fn resolved_locale_dirs(&self, workspace_root: &Path) -> Vec<std::path::PathBuf> {
        self.locale_paths
            .iter()
            .map(|p| workspace_root.join(p))
            .filter(|p| p.is_dir())
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyStyle {
    Nested,
    Flat,
    #[default]
    Auto,
}

impl ProjectConfig {
    /// Load project config from the workspace root.
    ///
    /// Order of resolution:
    /// 1. `.zed/lokalized.json` (legacy: `.zed/lokalize.json`)
    /// 2. Filesystem auto-detection (common locale directory names)
    pub fn load(workspace_root: &Path) -> Self {
        for config_name in [".zed/lokalized.json", ".zed/lokalize.json"] {
            if let Some(cfg) = Self::read_from_file(&workspace_root.join(config_name)) {
                return cfg.with_auto_detected_fallback(workspace_root);
            }
        }
        Self::auto_detect(workspace_root)
    }

    fn read_from_file(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Fill in `locale_paths` from auto-detection if the user did not set them.
    fn with_auto_detected_fallback(mut self, root: &Path) -> Self {
        if self.locale_paths.is_empty() {
            self.locale_paths = detect_locale_dirs(root);
        }
        self
    }

    /// Discover locale directories purely from filesystem heuristics.
    pub fn auto_detect(workspace_root: &Path) -> Self {
        Self {
            locale_paths: detect_locale_dirs(workspace_root),
            ..Default::default()
        }
    }

    /// Resolved source locale, defaulting to `"en"` when unset.
    pub fn resolved_source_locale(&self) -> Locale {
        self.source_locale
            .as_deref()
            .map(Locale::new)
            .unwrap_or_else(|| Locale::new("en"))
    }
}

fn detect_locale_dirs(root: &Path) -> Vec<String> {
    let mut found = HashSet::new();

    for candidate in CANDIDATE_LOCALE_DIRS
        .iter()
        .chain(MONOREPO_LOCALE_DIRS.iter())
    {
        let path = root.join(candidate);
        if looks_like_locale_dir(&path) {
            found.insert((*candidate).to_string());
        }
    }

    for path in discover_locale_dirs_deep(root) {
        found.insert(path);
    }

    let mut paths: Vec<_> = found.into_iter().collect();
    paths.sort();
    paths
}

/// Walk the workspace (e.g. monorepo root) and collect `**/locales` folders that
/// actually contain translation files.
fn discover_locale_dirs_deep(root: &Path) -> Vec<String> {
    let mut found = HashSet::new();
    let walker = WalkDir::new(root)
        .max_depth(DISCOVER_MAX_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_excluded_discover_dir(e.path()));

    for entry in walker.flatten() {
        if !entry.file_type().is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str() else {
            continue;
        };
        if !LOCALE_DIR_NAMES.contains(&name) {
            continue;
        }
        let path = entry.path();
        if !looks_like_locale_dir(path) {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            found.insert(rel.display().to_string());
        }
    }

    let mut paths: Vec<_> = found.into_iter().collect();
    paths.sort();
    paths
}

fn is_excluded_discover_dir(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| DISCOVER_EXCLUDED_DIRS.contains(&s))
    })
}

fn looks_like_locale_dir(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    WalkDir::new(dir)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .flatten()
        .any(|e| e.file_type().is_file() && is_locale_data_file(e.path()))
}

fn is_locale_data_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "json" | "jsonc" | "json5" | "arb" | "yml" | "yaml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_locale_defaults_to_en() {
        let cfg = ProjectConfig::default();
        assert_eq!(cfg.resolved_source_locale().as_str(), "en");
    }

    #[test]
    fn detects_monorepo_front_locale_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let locale_file = tmp.path().join("front/i18n/locales/fr/app.json");
        std::fs::create_dir_all(locale_file.parent().expect("parent")).expect("mkdir");
        std::fs::write(&locale_file, r#"{"hello":"bonjour"}"#).expect("write");

        let cfg = ProjectConfig::auto_detect(tmp.path());
        assert!(
            cfg.locale_paths.iter().any(|p| p == "front/i18n/locales"),
            "expected front/i18n/locales, got {:?}",
            cfg.locale_paths
        );
    }

    #[test]
    fn parses_camel_case_json() {
        let json = r#"{
            "localePaths": ["locales", "src/i18n"],
            "sourceLocale": "fr",
            "enabledFrameworks": ["vue-i18n"]
        }"#;
        let cfg: ProjectConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.locale_paths, vec!["locales", "src/i18n"]);
        assert_eq!(cfg.resolved_source_locale().as_str(), "fr");
        assert_eq!(cfg.enabled_frameworks, vec!["vue-i18n"]);
    }
}
