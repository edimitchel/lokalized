//! Load a workspace into a locale index and usage scan in one step.
//!
//! Shared by the CLI, MCP server, and (optionally) the LSP bootstrap.

use std::path::{Path, PathBuf};

use crate::config::ProjectConfig;
use crate::index::{IndexBuilder, IndexError, LocaleIndex};
use crate::parser::{parse_file, ParseError};
use crate::scan::UsageIndex;

/// Fully loaded i18n project state for tooling (CLI, MCP, checks).
#[derive(Clone, Debug)]
pub struct ProjectSnapshot {
    pub root: PathBuf,
    pub config: ProjectConfig,
    pub index: LocaleIndex,
    pub usages: UsageIndex,
}

/// Errors while loading or validating a project.
#[derive(thiserror::Error, Debug)]
pub enum ProjectError {
    #[error("workspace path does not exist: {0}")]
    WorkspaceNotFound(PathBuf),
    #[error("no locale directories configured or discovered")]
    NoLocalePaths,
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
}

/// One parse failure surfaced by [`ProjectSnapshot::validate`].
#[derive(Clone, Debug, serde::Serialize)]
pub struct ParseIssue {
    pub path: PathBuf,
    pub message: String,
}

impl ProjectSnapshot {
    /// Discover config, build the locale index, and scan source usages.
    pub fn load(root: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let root = root.as_ref().canonicalize().map_err(|_| {
            ProjectError::WorkspaceNotFound(root.as_ref().to_path_buf())
        })?;

        let config = ProjectConfig::load(&root);
        if config.locale_paths.is_empty() {
            return Err(ProjectError::NoLocalePaths);
        }

        let index = IndexBuilder::new(&root, &config).build()?;
        let usages = UsageIndex::build_from_project(&root);

        Ok(Self {
            root,
            config,
            index,
            usages,
        })
    }

    /// Re-parse every discovered locale file on disk.
    pub fn validate(&self) -> Vec<ParseIssue> {
        let mut issues = Vec::new();
        for file in &self.index.files {
            if let Err(e) = parse_file(&file.path) {
                issues.push(ParseIssue {
                    path: file.path.clone(),
                    message: e.to_string(),
                });
            }
        }
        issues
    }

    /// Locales that have a tree in the index (sorted).
    pub fn locales(&self) -> Vec<&crate::locale::Locale> {
        self.index.trees.keys().collect()
    }

    /// Target locales for missing-key checks: every locale except the source.
    pub fn target_locales(&self) -> Vec<crate::locale::Locale> {
        self.index
            .trees
            .keys()
            .filter(|l| **l != self.index.source_locale)
            .cloned()
            .collect()
    }

    /// Missing keys per target locale (or a single locale when `only` is set).
    pub fn missing_by_locale(
        &self,
        only: Option<&crate::locale::Locale>,
    ) -> std::collections::BTreeMap<String, Vec<String>> {
        use std::collections::BTreeMap;

        let all_targets = self.target_locales();
        let mut out = BTreeMap::new();
        match only {
            Some(locale) => {
                let missing = self.index.missing_keys(locale);
                out.insert(locale.as_str().to_string(), missing);
            }
            None => {
                for locale in &all_targets {
                    let missing = self.index.missing_keys(locale);
                    if !missing.is_empty() {
                        out.insert(locale.as_str().to_string(), missing);
                    }
                }
            }
        }
        out
    }

    pub fn unused_keys(&self) -> Vec<String> {
        let mut keys = self.index.unused_keys(&self.usages);
        keys.sort();
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    #[test]
    fn load_nested_fixture() {
        let snap = ProjectSnapshot::load(fixture("nested_project")).unwrap();
        assert_eq!(snap.index.source_locale.as_str(), "en");
        assert!(!snap.index.all_keys().is_empty());
        let missing = snap.missing_by_locale(None);
        assert!(missing.get("fr").is_some_and(|m| m.contains(&"common.actions.cancel".to_string())));
    }

    #[test]
    fn validate_succeeds_on_fixtures() {
        let snap = ProjectSnapshot::load(fixture("flat_project")).unwrap();
        assert!(snap.validate().is_empty());
    }
}