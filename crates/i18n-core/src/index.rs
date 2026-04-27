//! In-memory index mapping translation keys to their values across locales.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::config::ProjectConfig;
use crate::locale::{Locale, LocaleFile, LocaleLayout};
use crate::parser::{parse_file, ParseError};
use crate::position::Range;
use crate::scan::UsageIndex;

/// One translation value at a specific location in a locale file.
#[derive(Clone, Debug)]
pub struct LocalizedValue {
    pub value: String,
    pub file: PathBuf,
    /// Range of the value literal inside the file. Used by hover and goto.
    pub range: Range,
    /// Range of the leaf key (the property name) inside the file. Used by
    /// diagnostics that decorate the key itself, e.g. unused translations.
    pub key_range: Range,
}

/// Tree of keys: `{ common: { submit: Leaf("Submit"), cancel: Leaf("Cancel") } }`.
#[derive(Clone, Debug, Default)]
pub struct KeyTree {
    pub children: BTreeMap<String, KeyNode>,
}

#[derive(Clone, Debug)]
pub enum KeyNode {
    Leaf(LocalizedValue),
    Branch(KeyTree),
}

impl KeyTree {
    /// Insert a value at the given dotted path. Conflicts with an existing
    /// branch/leaf are silently dropped in Phase 1 — Phase 3 will surface them
    /// as diagnostics.
    pub fn insert(&mut self, path: &[String], value: LocalizedValue) {
        let Some((head, rest)) = path.split_first() else {
            return;
        };
        if rest.is_empty() {
            self.children.insert(head.clone(), KeyNode::Leaf(value));
            return;
        }
        let entry = self
            .children
            .entry(head.clone())
            .or_insert_with(|| KeyNode::Branch(KeyTree::default()));
        if let KeyNode::Branch(sub) = entry {
            sub.insert(rest, value);
        }
    }

    pub fn lookup(&self, path: &[String]) -> Option<&LocalizedValue> {
        let (head, rest) = path.split_first()?;
        match self.children.get(head)? {
            KeyNode::Leaf(v) if rest.is_empty() => Some(v),
            KeyNode::Branch(sub) => sub.lookup(rest),
            _ => None,
        }
    }
}

/// The complete index across every discovered locale in a workspace.
#[derive(Clone, Debug, Default)]
pub struct LocaleIndex {
    pub trees: BTreeMap<Locale, KeyTree>,
    pub files: Vec<LocaleFile>,
    pub layout: Option<LocaleLayout>,
    pub source_locale: Locale,
    /// Snapshot of the project configuration the index was built with. Kept
    /// on the index so downstream consumers (code actions, diagnostics) can
    /// see exactly what semantics were applied without reloading the file.
    pub config: ProjectConfig,
}

impl LocaleIndex {
    /// Resolve a dotted key across every locale.
    pub fn lookup(&self, key: &str) -> BTreeMap<&Locale, &LocalizedValue> {
        let path: Vec<String> = key.split('.').map(str::to_string).collect();
        self.trees
            .iter()
            .filter_map(|(locale, tree)| tree.lookup(&path).map(|v| (locale, v)))
            .collect()
    }

    /// Union of all keys, across every locale.
    pub fn all_keys(&self) -> Vec<String> {
        let mut out = BTreeSet::new();
        for tree in self.trees.values() {
            collect_keys(tree, &mut Vec::new(), &mut out);
        }
        out.into_iter().collect()
    }

    /// Keys present in the source locale but missing from `locale`.
    pub fn missing_keys(&self, locale: &Locale) -> Vec<String> {
        let Some(source) = self.trees.get(&self.source_locale) else {
            return Vec::new();
        };
        let target = self.trees.get(locale);
        let mut missing = Vec::new();
        diff_tree(source, target, &mut Vec::new(), &mut missing);
        missing
    }

    /// Group every leaf in the index by its defining file. The returned
    /// vectors hold `(dotted_key, &LocalizedValue)` pairs in the natural
    /// traversal order (alphabetical, since the tree is a `BTreeMap`).
    ///
    /// Convenient for diagnostics that need to walk a single locale file's
    /// keys with their source ranges already attached.
    pub fn entries_by_file(&self) -> HashMap<PathBuf, Vec<(String, &LocalizedValue)>> {
        let mut out: HashMap<PathBuf, Vec<_>> = HashMap::new();
        for tree in self.trees.values() {
            collect_entries_by_file(tree, &mut Vec::new(), &mut out);
        }
        out
    }

    /// Subset of [`Self::all_keys`] that no source file in `usages`
    /// references. Best-effort: dynamic keys (`t(name)`, template literals)
    /// look unused to this scan and the LSP surfaces them at `Hint`
    /// severity to avoid false-positive noise.
    pub fn unused_keys(&self, usages: &UsageIndex) -> Vec<String> {
        self.all_keys()
            .into_iter()
            .filter(|k| !usages.is_key_used(k))
            .collect()
    }

    /// Compose the full dotted key for an entry parsed from `file`, applying
    /// the same namespace prefixing rules [`IndexBuilder`] uses.
    ///
    /// This lets diagnostics that re-parse a live buffer (instead of trusting
    /// the on-disk index) reconstruct the same dotted keys the source-side
    /// scanner sees, so usage lookups stay consistent.
    pub fn compose_full_key(&self, file: &LocaleFile, key_path: &[String]) -> String {
        let mut full: Vec<String> = Vec::with_capacity(key_path.len() + 1);
        let nested = matches!(self.layout, Some(LocaleLayout::Nested));
        if nested && self.config.use_file_namespace() {
            if let Some(ns) = &file.namespace {
                full.push(ns.clone());
            }
        }
        full.extend(key_path.iter().cloned());
        full.join(".")
    }
}

fn collect_keys(tree: &KeyTree, path: &mut Vec<String>, out: &mut BTreeSet<String>) {
    for (name, node) in &tree.children {
        path.push(name.clone());
        match node {
            KeyNode::Leaf(_) => {
                out.insert(path.join("."));
            }
            KeyNode::Branch(sub) => collect_keys(sub, path, out),
        }
        path.pop();
    }
}

fn collect_entries_by_file<'a>(
    tree: &'a KeyTree,
    path: &mut Vec<String>,
    out: &mut HashMap<PathBuf, Vec<(String, &'a LocalizedValue)>>,
) {
    for (name, node) in &tree.children {
        path.push(name.clone());
        match node {
            KeyNode::Leaf(v) => {
                out.entry(v.file.clone())
                    .or_default()
                    .push((path.join("."), v));
            }
            KeyNode::Branch(sub) => collect_entries_by_file(sub, path, out),
        }
        path.pop();
    }
}

fn diff_tree(
    source: &KeyTree,
    target: Option<&KeyTree>,
    path: &mut Vec<String>,
    out: &mut Vec<String>,
) {
    for (name, node) in &source.children {
        path.push(name.clone());
        let other = target.and_then(|t| t.children.get(name));
        match (node, other) {
            (KeyNode::Leaf(_), None) => out.push(path.join(".")),
            (KeyNode::Leaf(_), Some(KeyNode::Leaf(_))) => {}
            (KeyNode::Branch(sub), Some(KeyNode::Branch(tgt))) => {
                diff_tree(sub, Some(tgt), path, out);
            }
            (KeyNode::Branch(sub), None) => diff_tree(sub, None, path, out),
            _ => out.push(path.join(".")),
        }
        path.pop();
    }
}

// ---------- Builder ----------

#[derive(thiserror::Error, Debug)]
pub enum IndexError {
    #[error("no locale files discovered in workspace")]
    NoLocalesFound,
    #[error("failed to scan {path}: {source}")]
    Scan {
        path: PathBuf,
        #[source]
        source: walkdir::Error,
    },
    #[error(transparent)]
    Parse(#[from] ParseError),
}

pub struct IndexBuilder<'a> {
    workspace_root: &'a Path,
    config: &'a ProjectConfig,
}

impl<'a> IndexBuilder<'a> {
    pub fn new(workspace_root: &'a Path, config: &'a ProjectConfig) -> Self {
        Self {
            workspace_root,
            config,
        }
    }

    pub fn build(&self) -> Result<LocaleIndex, IndexError> {
        let files = self.discover_files()?;
        if files.is_empty() {
            return Err(IndexError::NoLocalesFound);
        }

        let layout = Self::detect_layout(&files);
        let source_locale = self.config.resolved_source_locale();
        let use_file_namespace = self.config.use_file_namespace();

        let mut trees: BTreeMap<Locale, KeyTree> = BTreeMap::new();
        for file in &files {
            let entries = parse_file(&file.path)?;
            let tree = trees.entry(file.locale.clone()).or_default();
            for entry in entries {
                let mut full_path = Vec::new();
                // Only prepend the filename stem when the user opts into the
                // `namespace = true` i18n-ally semantics. Projects where each
                // JSON already wraps its content (`{ "slots": {...} }`) should
                // set `namespace: false` to avoid double-prefixing.
                if layout == LocaleLayout::Nested && use_file_namespace {
                    if let Some(ns) = &file.namespace {
                        full_path.push(ns.clone());
                    }
                }
                full_path.extend(entry.key_path);

                tree.insert(
                    &full_path,
                    LocalizedValue {
                        value: entry.value,
                        file: file.path.clone(),
                        range: entry.range,
                        key_range: entry.key_range,
                    },
                );
            }
        }

        Ok(LocaleIndex {
            trees,
            files,
            layout: Some(layout),
            source_locale,
            config: self.config.clone(),
        })
    }

    fn discover_files(&self) -> Result<Vec<LocaleFile>, IndexError> {
        let mut files = Vec::new();
        for path in &self.config.locale_paths {
            let dir = self.workspace_root.join(path);
            if !dir.is_dir() {
                continue;
            }
            scan_locale_dir(&dir, &mut files)?;
        }
        Ok(files)
    }

    fn detect_layout(files: &[LocaleFile]) -> LocaleLayout {
        if files.iter().any(|f| f.namespace.is_some()) {
            LocaleLayout::Nested
        } else {
            LocaleLayout::Flat
        }
    }
}

fn scan_locale_dir(dir: &Path, out: &mut Vec<LocaleFile>) -> Result<(), IndexError> {
    for entry in walkdir::WalkDir::new(dir)
        .max_depth(3)
        .follow_links(false)
    {
        let entry = entry.map_err(|e| IndexError::Scan {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        if !matches!(ext, "json" | "jsonc" | "json5" | "arb") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let parent = path.parent().unwrap_or(dir);

        let (locale, namespace) = if parent == dir {
            // Flat: `en.json`, `fr.json`, or ARB `app_en.arb`
            (extract_locale_from_stem(&stem), None)
        } else {
            // Nested: `<locale>/<namespace>.json`
            let locale_name = parent
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            (Locale::new(locale_name), Some(stem))
        };

        if locale.is_empty() {
            continue;
        }

        out.push(LocaleFile {
            locale,
            namespace,
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn extract_locale_from_stem(stem: &str) -> Locale {
    // Handle ARB naming conventions like `app_en`, `intl_en_US`.
    if let Some(rest) = stem.strip_prefix("app_") {
        return Locale::new(rest);
    }
    if let Some(rest) = stem.strip_prefix("intl_") {
        return Locale::new(rest);
    }
    if let Some(rest) = stem.strip_prefix("messages_") {
        return Locale::new(rest);
    }
    Locale::new(stem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::{Position, Range};

    fn value(text: &str) -> LocalizedValue {
        LocalizedValue {
            value: text.to_string(),
            file: PathBuf::from("/tmp/en.json"),
            range: Range::default(),
            key_range: Range::default(),
        }
    }

    #[test]
    fn tree_insert_and_lookup() {
        let mut t = KeyTree::default();
        t.insert(&["a".into(), "b".into()], value("AB"));
        t.insert(&["a".into(), "c".into()], value("AC"));
        assert_eq!(t.lookup(&["a".into(), "b".into()]).unwrap().value, "AB");
        assert_eq!(t.lookup(&["a".into(), "c".into()]).unwrap().value, "AC");
        assert!(t.lookup(&["a".into()]).is_none()); // it's a branch, not a leaf
    }

    #[test]
    fn index_missing_keys() {
        let mut idx = LocaleIndex {
            source_locale: Locale::new("en"),
            ..LocaleIndex::default()
        };

        let mut en = KeyTree::default();
        en.insert(&["hello".into()], value("Hi"));
        en.insert(&["bye".into()], value("Bye"));

        let mut fr = KeyTree::default();
        fr.insert(&["hello".into()], value("Salut"));

        idx.trees.insert(Locale::new("en"), en);
        idx.trees.insert(Locale::new("fr"), fr);

        let missing = idx.missing_keys(&Locale::new("fr"));
        assert_eq!(missing, vec!["bye".to_string()]);
    }

    #[test]
    fn index_all_keys_union() {
        let mut idx = LocaleIndex::default();
        let mut en = KeyTree::default();
        en.insert(&["a".into(), "b".into()], value("1"));
        let mut fr = KeyTree::default();
        fr.insert(&["a".into(), "c".into()], value("2"));
        idx.trees.insert(Locale::new("en"), en);
        idx.trees.insert(Locale::new("fr"), fr);

        assert_eq!(idx.all_keys(), vec!["a.b".to_string(), "a.c".to_string()]);
    }

    #[test]
    fn index_unused_keys_filters_against_usage_index() {
        let mut idx = LocaleIndex::default();
        let mut en = KeyTree::default();
        en.insert(&["used".into()], value("U"));
        en.insert(&["dead".into()], value("D"));
        idx.trees.insert(Locale::new("en"), en);

        let mut usages = UsageIndex::new();
        usages.update_file(
            PathBuf::from("/x.ts"),
            r#"t("used");"#,
            "TypeScript",
        );

        assert_eq!(idx.unused_keys(&usages), vec!["dead".to_string()]);
    }

    #[test]
    fn index_entries_by_file_groups_leaves() {
        let mut idx = LocaleIndex::default();

        let en_value = LocalizedValue {
            value: "Hi".into(),
            file: PathBuf::from("/en.json"),
            range: Range::default(),
            key_range: Range::default(),
        };
        let fr_value = LocalizedValue {
            value: "Salut".into(),
            file: PathBuf::from("/fr.json"),
            range: Range::default(),
            key_range: Range::default(),
        };

        let mut en = KeyTree::default();
        en.insert(&["hello".into()], en_value);
        let mut fr = KeyTree::default();
        fr.insert(&["hello".into()], fr_value);
        idx.trees.insert(Locale::new("en"), en);
        idx.trees.insert(Locale::new("fr"), fr);

        let by_file = idx.entries_by_file();
        assert_eq!(by_file.len(), 2);
        assert_eq!(by_file[&PathBuf::from("/en.json")][0].0, "hello");
        assert_eq!(by_file[&PathBuf::from("/fr.json")][0].0, "hello");
    }

    // Suppress the unused-import warning from the enclosing module when this
    // test module is compiled alone.
    #[allow(dead_code)]
    fn _assert_position_used(_p: Position) {}
}
