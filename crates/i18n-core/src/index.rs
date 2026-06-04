//! In-memory index mapping translation keys to their values across locales.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::config::ProjectConfig;
use crate::locale::{Locale, LocaleFile, LocaleLayout};
use crate::parser::{parse_file, parse_with_extension, ParseError};
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

    /// Recursively remove every leaf whose [`LocalizedValue::file`] matches
    /// `target`, and garbage-collect branches that become empty as a result.
    /// Returns `true` when at least one leaf was removed.
    ///
    /// Used by [`LocaleIndex::update_file_from_buffer`] to replace all entries
    /// sourced from a single file before re-inserting the freshly parsed ones.
    fn prune_leaves_from_file(&mut self, target: &Path) -> bool {
        let mut changed = false;
        self.children.retain(|_, node| match node {
            KeyNode::Leaf(v) => {
                if v.file == target {
                    changed = true;
                    false
                } else {
                    true
                }
            }
            KeyNode::Branch(sub) => {
                if sub.prune_leaves_from_file(target) {
                    changed = true;
                }
                // Drop empty branches so the tree stays tidy; otherwise
                // `all_keys` would keep returning stale prefixes.
                !sub.children.is_empty()
            }
        });
        changed
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

    /// Replace every leaf previously sourced from `path` with whatever
    /// parsing `content` yields, in place. Intended to be called from the
    /// LSP when a locale JSON buffer changes so source-side diagnostics
    /// (missing-key / missing-source) immediately reflect the edit without
    /// waiting for a full index rebuild.
    ///
    /// Returns `true` when the index content actually changed, allowing the
    /// caller to short-circuit diagnostic republishing when nothing moved
    /// (e.g. pure whitespace edits on a file with a parse error).
    ///
    /// If `path` isn't a known locale file, the call is a no-op returning
    /// `Ok(false)`.
    pub fn update_file_from_buffer(
        &mut self,
        path: &Path,
        content: &str,
    ) -> Result<bool, ParseError> {
        // Find the matching LocaleFile so we know which locale tree to
        // touch and how to prefix the parsed key paths.
        let Some(locale_file) = self.files.iter().find(|f| f.path == path).cloned() else {
            return Ok(false);
        };

        // Parsing may fail on transient invalid JSON while the user is
        // typing; surface the error so the caller can decide whether to
        // keep the previous index or show a diagnostic.
        let entries = parse_with_extension(content, path)?;

        let tree = self.trees.entry(locale_file.locale.clone()).or_default();
        let had_leaves = tree.prune_leaves_from_file(path);

        let nested = matches!(self.layout, Some(LocaleLayout::Nested));
        let prepend_ns = nested && self.config.use_file_namespace();
        let mut inserted = 0usize;
        for entry in entries {
            let mut full_path: Vec<String> = Vec::with_capacity(entry.key_path.len() + 1);
            if prepend_ns {
                if let Some(ns) = &locale_file.namespace {
                    full_path.push(ns.clone());
                }
            }
            full_path.extend(entry.key_path);
            tree.insert(
                &full_path,
                LocalizedValue {
                    value: entry.value,
                    file: path.to_path_buf(),
                    range: entry.range,
                    key_range: entry.key_range,
                },
            );
            inserted += 1;
        }

        Ok(had_leaves || inserted > 0)
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
    for entry in walkdir::WalkDir::new(dir).max_depth(3).follow_links(false) {
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
        usages.update_file(PathBuf::from("/x.ts"), r#"t("used");"#, "TypeScript");

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

    #[test]
    fn update_file_from_buffer_replaces_leaves_and_refreshes_missing_keys() {
        use std::fs;
        use tempfile::TempDir;

        // Project with two flat locale files; only `fr.json` starts out
        // missing the `common.cancel` key.
        let dir = TempDir::new().unwrap();
        let en_path = dir.path().join("en.json");
        let fr_path = dir.path().join("fr.json");
        fs::write(
            &en_path,
            r#"{"common":{"submit":"Submit","cancel":"Cancel"}}"#,
        )
        .unwrap();
        fs::write(&fr_path, r#"{"common":{"submit":"Envoyer"}}"#).unwrap();

        let config = ProjectConfig {
            source_locale: Some("en".into()),
            locale_paths: vec![".".into()],
            ..ProjectConfig::default()
        };
        let mut idx = IndexBuilder::new(dir.path(), &config).build().unwrap();

        // Baseline: `common.cancel` missing in fr → present in `missing_keys`.
        let fr = Locale::new("fr");
        assert!(idx.missing_keys(&fr).iter().any(|k| k == "common.cancel"));

        // Simulate the user adding the missing key in the buffer.
        let changed = idx
            .update_file_from_buffer(
                &fr_path,
                r#"{"common":{"submit":"Envoyer","cancel":"Annuler"}}"#,
            )
            .unwrap();
        assert!(changed);

        // After the buffer-level update, the key is no longer missing
        // without having to rebuild the index from disk.
        assert!(!idx.missing_keys(&fr).iter().any(|k| k == "common.cancel"));

        // And looking the key up returns the fresh value.
        let values = idx.lookup("common.cancel");
        assert_eq!(values.get(&fr).map(|v| v.value.as_str()), Some("Annuler"));
    }

    #[test]
    fn update_file_from_buffer_prunes_deleted_keys() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let en_path = dir.path().join("en.json");
        fs::write(&en_path, r#"{"a":"A","b":"B"}"#).unwrap();

        let config = ProjectConfig {
            source_locale: Some("en".into()),
            locale_paths: vec![".".into()],
            ..ProjectConfig::default()
        };
        let mut idx = IndexBuilder::new(dir.path(), &config).build().unwrap();
        assert!(idx.lookup("a").contains_key(&Locale::new("en")));

        idx.update_file_from_buffer(&en_path, r#"{"b":"B"}"#)
            .unwrap();
        assert!(idx.lookup("a").is_empty(), "deleted key should vanish");
        assert!(idx.lookup("b").contains_key(&Locale::new("en")));
    }

    #[test]
    fn update_file_from_buffer_is_noop_for_unknown_path() {
        let mut idx = LocaleIndex::default();
        let changed = idx
            .update_file_from_buffer(&PathBuf::from("/nope/en.json"), r#"{"a":"1"}"#)
            .unwrap();
        assert!(!changed);
    }

    #[test]
    fn update_file_from_buffer_propagates_parse_errors() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let en_path = dir.path().join("en.json");
        fs::write(&en_path, r#"{"a":"A"}"#).unwrap();

        let config = ProjectConfig {
            source_locale: Some("en".into()),
            locale_paths: vec![".".into()],
            ..ProjectConfig::default()
        };
        let mut idx = IndexBuilder::new(dir.path(), &config).build().unwrap();

        // Mid-edit garbage: parser error must bubble up so the caller can
        // keep the previous tree instead of corrupting the index.
        let err = idx.update_file_from_buffer(&en_path, r#"{"a": "#);
        assert!(err.is_err());
        // And the pre-existing leaf survives untouched.
        assert_eq!(idx.lookup("a").get(&Locale::new("en")).unwrap().value, "A");
    }

    // Suppress the unused-import warning from the enclosing module when this
    // test module is compiled alone.
    #[allow(dead_code)]
    fn _assert_position_used(_p: Position) {}
}
