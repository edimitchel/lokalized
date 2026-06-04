//! Vue I18n linked messages (`@:other.key`, `@.lower:other.key`).
//!
//! See <https://vue-i18n.intlify.dev/guide/essentials/syntax#linked-messages>.

use crate::locale::Locale;
use crate::LocaleIndex;

/// A parsed linked-message reference in a locale value string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedMessage<'a> {
    pub target_key: &'a str,
    pub modifier: Option<&'a str>,
}

/// Result of resolving a locale value, following link chains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedValue<'a> {
    Literal {
        text: &'a str,
    },
    Linked {
        /// Text after following the link (and applying a modifier).
        display: String,
        /// Immediate link target (first hop).
        target_key: String,
        /// Full chain `a → b → c` when nested.
        chain: Vec<String>,
        /// Leaf definition that supplied `display`.
        raw: &'a str,
    },
    Broken {
        target_key: String,
        reason: &'static str,
    },
}

const MAX_LINK_DEPTH: usize = 12;

/// Parse `@:key.path` or `@.modifier:key.path`.
pub fn parse_linked_message(raw: &str) -> Option<LinkedMessage<'_>> {
    let trimmed = raw.trim();
    if !trimmed.starts_with('@') {
        return None;
    }
    let rest = trimmed.strip_prefix('@')?;
    if let Some(key) = rest.strip_prefix(':') {
        let key = key.trim();
        return (!key.is_empty()).then_some(LinkedMessage {
            target_key: key,
            modifier: None,
        });
    }
    if let Some(rest) = rest.strip_prefix('.') {
        let (modifier, key) = rest.split_once(':')?;
        let key = key.trim();
        let modifier = modifier.trim();
        if key.is_empty() || modifier.is_empty() {
            return None;
        }
        return Some(LinkedMessage {
            target_key: key,
            modifier: Some(modifier),
        });
    }
    None
}

/// Apply a vue-i18n link modifier to resolved text.
pub fn apply_modifier(text: &str, modifier: Option<&str>) -> String {
    let Some(name) = modifier else {
        return text.to_string();
    };
    match name {
        "lower" | "lowercase" => text.to_lowercase(),
        "upper" | "uppercase" => text.to_uppercase(),
        "capitalize" => {
            let mut chars = text.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        }
        _ => text.to_string(),
    }
}

/// Resolve a stored locale value for display / navigation (follows nested links).
pub fn resolve_value<'a>(
    index: &'a LocaleIndex,
    locale: &Locale,
    raw: &'a str,
) -> ResolvedValue<'a> {
    let mut chain: Vec<String> = Vec::new();
    let mut current = raw;
    let mut pending_modifier: Option<&str> = None;

    loop {
        if chain.len() > MAX_LINK_DEPTH {
            return ResolvedValue::Broken {
                target_key: chain.last().cloned().unwrap_or_default(),
                reason: "link chain too deep",
            };
        }

        let Some(link) = parse_linked_message(current) else {
            return if chain.is_empty() {
                ResolvedValue::Literal { text: current }
            } else {
                ResolvedValue::Linked {
                    display: apply_modifier(current, pending_modifier),
                    target_key: chain.first().cloned().unwrap_or_default(),
                    chain,
                    raw: current,
                }
            };
        };

        let target_key = link.target_key.to_string();
        if chain.contains(&target_key) {
            return ResolvedValue::Broken {
                target_key,
                reason: "circular link",
            };
        }
        chain.push(target_key.clone());
        pending_modifier = link.modifier.or(pending_modifier);

        let Some(leaf) = index.lookup(&target_key).get(locale).copied() else {
            return ResolvedValue::Broken {
                target_key,
                reason: "target key not found",
            };
        };
        current = &leaf.value;
    }
}

impl LocaleIndex {
    /// All dotted keys whose value links to `target_key` in any locale.
    pub fn keys_linking_to(&self, target_key: &str) -> Vec<String> {
        let mut out = Vec::new();
        for key in self.all_keys() {
            for (_locale, value) in self.lookup(&key) {
                if let Some(link) = parse_linked_message(&value.value) {
                    if link.target_key == target_key {
                        out.push(key);
                        break;
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Resolved preview string for hover / inlay hints.
    pub fn display_for_locale(&self, locale: &Locale, raw: &str) -> String {
        match resolve_value(self, locale, raw) {
            ResolvedValue::Literal { text } => text.to_string(),
            ResolvedValue::Linked { display, .. } => display,
            ResolvedValue::Broken { target_key, reason } => {
                format!("⛔ {reason}: `{target_key}`")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{IndexBuilder, KeyTree, LocaleIndex, LocalizedValue};
    use crate::locale::Locale;
    use crate::position::Range;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn parses_colon_link() {
        let l = parse_linked_message("@:global.providers.payfip").unwrap();
        assert_eq!(l.target_key, "global.providers.payfip");
        assert_eq!(l.modifier, None);
    }

    #[test]
    fn parses_modifier_link() {
        let l = parse_linked_message("@.lower:common.greeting").unwrap();
        assert_eq!(l.target_key, "common.greeting");
        assert_eq!(l.modifier, Some("lower"));
    }

    #[test]
    fn resolves_linked_chain() {
        let mut trees = BTreeMap::new();
        let mut fr = KeyTree::default();
        fr.insert(
            &["global".into(), "providers".into(), "payfip".into()],
            LocalizedValue {
                value: "PayFiP".into(),
                file: PathBuf::from("/fr.json"),
                range: Range::default(),
                key_range: Range::default(),
            },
        );
        fr.insert(
            &["global".into(), "payfip".into()],
            LocalizedValue {
                value: "@:global.providers.payfip".into(),
                file: PathBuf::from("/fr.json"),
                range: Range::default(),
                key_range: Range::default(),
            },
        );
        trees.insert(Locale::new("fr"), fr);
        let idx = LocaleIndex {
            trees,
            files: vec![],
            layout: None,
            source_locale: Locale::new("fr"),
            config: Default::default(),
        };
        match resolve_value(&idx, &Locale::new("fr"), "@:global.providers.payfip") {
            ResolvedValue::Linked { display, target_key, .. } => {
                assert_eq!(target_key, "global.providers.payfip");
                assert_eq!(display, "PayFiP");
            }
            other => panic!("expected Linked, got {other:?}"),
        }
        match resolve_value(&idx, &Locale::new("fr"), "@:global.payfip") {
            ResolvedValue::Linked { display, chain, .. } => {
                assert_eq!(display, "PayFiP");
                assert!(chain.len() >= 2);
            }
            other => panic!("expected Linked, got {other:?}"),
        }
    }

    #[test]
    fn applies_modifier_after_link_chain() {
        let mut trees = BTreeMap::new();
        let mut fr = KeyTree::default();
        fr.insert(
            &["common".into(), "greeting".into()],
            LocalizedValue {
                value: "HELLO".into(),
                file: PathBuf::from("/fr.json"),
                range: Range::default(),
                key_range: Range::default(),
            },
        );
        fr.insert(
            &["title".into()],
            LocalizedValue {
                value: "@.lower:common.greeting".into(),
                file: PathBuf::from("/fr.json"),
                range: Range::default(),
                key_range: Range::default(),
            },
        );
        trees.insert(Locale::new("fr"), fr);
        let idx = LocaleIndex {
            trees,
            files: vec![],
            layout: None,
            source_locale: Locale::new("fr"),
            config: Default::default(),
        };
        match resolve_value(&idx, &Locale::new("fr"), "@.lower:common.greeting") {
            ResolvedValue::Linked { display, .. } => assert_eq!(display, "hello"),
            other => panic!("expected Linked, got {other:?}"),
        }
    }

    #[test]
    fn detects_broken_link() {
        let idx = LocaleIndex::default();
        match resolve_value(&idx, &Locale::new("fr"), "@:missing.key") {
            ResolvedValue::Broken { .. } => {}
            other => panic!("expected Broken, got {other:?}"),
        }
    }

    #[test]
    fn keys_linking_to_finds_aliases() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let fr = dir.path().join("locales/fr");
        fs::create_dir_all(&fr).unwrap();
        fs::write(
            fr.join("global.json"),
            r#"{"global":{"providers":{"payfip":"PayFiP"},"payfip":"@:global.providers.payfip"}}"#,
        )
        .unwrap();

        let config = crate::config::ProjectConfig {
            locale_paths: vec!["locales".into()],
            namespace: Some(false),
            source_locale: Some("fr".into()),
            ..Default::default()
        };
        let idx = IndexBuilder::new(dir.path(), &config).build().unwrap();
        let links = idx.keys_linking_to("global.providers.payfip");
        assert!(links.iter().any(|k| k == "global.payfip"));
    }
}