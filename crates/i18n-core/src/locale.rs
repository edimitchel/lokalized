//! Locale descriptors (BCP-47 codes, source vs target, directory layouts).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// BCP-47 locale code (e.g. `"en"`, `"fr-FR"`).
///
/// Normalised to lowercase with `-` as separator.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Locale(String);

impl Locale {
    pub fn new(code: impl Into<String>) -> Self {
        let code = code.into();
        Self(code.to_ascii_lowercase().replace('_', "-"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&str> for Locale {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl std::fmt::Display for Locale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// How locale files are organised on disk under a single locale path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocaleLayout {
    /// One file per locale: `en.json`, `fr.json`, `app_en.arb`.
    Flat,
    /// One subdirectory per locale, each containing namespaced files:
    /// `en/common.json`, `en/auth.json`, `fr/common.json`, …
    Nested,
}

/// A locale file discovered on disk.
#[derive(Clone, Debug)]
pub struct LocaleFile {
    pub locale: Locale,
    /// Namespace derived from the filename stem (only for nested layouts).
    pub namespace: Option<String>,
    pub path: PathBuf,
}
