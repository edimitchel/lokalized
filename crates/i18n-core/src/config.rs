//! Project configuration loaded from `.zed/lokalize.json` (with sensible auto-detection).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::locale::Locale;

/// Directories commonly used to store locale files, checked in order during auto-detection.
const CANDIDATE_LOCALE_DIRS: &[&str] = &[
    "locales",
    "src/locales",
    "i18n",
    "public/locales",
    "lib/l10n",
    "app/locales",
    "assets/locales",
];

/// Configuration for a Lokalize-enabled workspace.
///
/// Loaded from `.zed/lokalize.json`. All fields are optional — missing values
/// fall back to filesystem heuristics.
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
    /// 1. `.zed/lokalize.json`
    /// 2. Filesystem auto-detection (common locale directory names)
    pub fn load(workspace_root: &Path) -> Self {
        if let Some(cfg) = Self::read_from_file(&workspace_root.join(".zed/lokalize.json")) {
            return cfg.with_auto_detected_fallback(workspace_root);
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
    CANDIDATE_LOCALE_DIRS
        .iter()
        .filter(|c| root.join(c).is_dir())
        .map(|c| (*c).to_string())
        .collect()
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
