//! Framework definitions and key-usage detection.
//!
//! A *framework* describes how a given i18n library (vue-i18n, i18next, …) invokes
//! translation functions in source code. We use regex patterns to extract the key
//! from calls like `t("common.submit")`, `$t("…")`, `useTranslation("ns")`, etc.
//!
//! Built-in frameworks are hard-coded here for simplicity; custom user frameworks
//! (from `.zed/i18n-ally-custom-framework.yml`) will be loaded in Phase 2.5.

use std::sync::LazyLock;

use regex::Regex;

use crate::position::{LineIndex, Range};

/// A translation key usage detected in a source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyUsage {
    /// The resolved translation key (including any scope prefix).
    pub key: String,
    /// Byte/line/UTF-16 range of the *key literal* inside the string quotes.
    pub range: Range,
    /// The effective scope/namespace at the call site (from `useTranslation("ns")`).
    pub scope: Option<String>,
    /// Which framework matched this usage.
    pub framework_id: &'static str,
}

/// Definition of an i18n framework — describes how to recognise its translation
/// function calls in source code.
pub struct Framework {
    pub id: &'static str,
    pub name: &'static str,
    /// Zed language names this framework applies to.
    pub language_ids: &'static [&'static str],
    /// Regex patterns whose first capture group is the translation key.
    pub usage_patterns: Vec<Regex>,
    /// Optional regex to detect scope/namespace declarations
    /// (e.g. `useTranslation("forms")` → scope `"forms"`).
    pub scope_pattern: Option<Regex>,
    /// Templates used when extracting strings to keys (`$1` = new key).
    pub refactor_templates: &'static [&'static str],
}

impl Framework {
    pub fn applies_to(&self, language_id: &str) -> bool {
        self.language_ids.contains(&language_id)
    }

    /// Scan `source` and return every key usage matched by this framework's patterns.
    pub fn find_usages(&self, source: &str) -> Vec<KeyUsage> {
        let lines = LineIndex::new(source);
        let scopes = collect_scopes(source, self.scope_pattern.as_ref());

        let mut out = Vec::new();
        for pattern in &self.usage_patterns {
            for caps in pattern.captures_iter(source) {
                let Some(key_match) = caps.get(1) else {
                    continue;
                };
                let scope = nearest_scope_before(&scopes, key_match.start());
                let scoped_key = match &scope {
                    Some(ns) => format!("{ns}.{}", key_match.as_str()),
                    None => key_match.as_str().to_string(),
                };
                out.push(KeyUsage {
                    key: scoped_key,
                    range: lines.range(key_match.start(), key_match.end()),
                    scope,
                    framework_id: self.id,
                });
            }
        }
        out
    }
}

/// Every built-in framework, compiled once.
pub static BUILTIN_FRAMEWORKS: LazyLock<Vec<Framework>> = LazyLock::new(|| {
    vec![
        framework_vue_i18n(),
        framework_nuxt_i18n(),
        framework_i18next(),
        framework_react_intl(),
    ]
});

/// Find every usage of a translation function in the given source, across all
/// applicable built-in frameworks.
///
/// If multiple frameworks match at the same source offset (e.g. both vue-i18n and
/// nuxt-i18n recognise `$t(...)`), the match with a resolved `scope` wins, and
/// the first framework in registration order otherwise.
pub fn find_usages(source: &str, language_id: &str) -> Vec<KeyUsage> {
    let all: Vec<KeyUsage> = BUILTIN_FRAMEWORKS
        .iter()
        .filter(|f| f.applies_to(language_id))
        .flat_map(|f| f.find_usages(source))
        .collect();
    dedupe_by_offset(all)
}

/// Merge usages that cover the exact same byte range, preferring scoped matches.
fn dedupe_by_offset(mut usages: Vec<KeyUsage>) -> Vec<KeyUsage> {
    usages.sort_by_key(|u| (u.range.start.offset, u.range.end.offset));
    let mut out: Vec<KeyUsage> = Vec::with_capacity(usages.len());
    for u in usages {
        match out.last_mut() {
            Some(prev)
                if prev.range.start.offset == u.range.start.offset
                    && prev.range.end.offset == u.range.end.offset =>
            {
                if prev.scope.is_none() && u.scope.is_some() {
                    *prev = u;
                }
            }
            _ => out.push(u),
        }
    }
    out
}

// ---------- Scope resolution ----------

struct ScopeMatch {
    start: usize,
    scope: String,
}

fn collect_scopes(source: &str, pattern: Option<&Regex>) -> Vec<ScopeMatch> {
    let Some(pattern) = pattern else {
        return Vec::new();
    };
    pattern
        .captures_iter(source)
        .filter_map(|caps| {
            let m = caps.get(1)?;
            Some(ScopeMatch {
                start: m.start(),
                scope: m.as_str().to_string(),
            })
        })
        .collect()
}

fn nearest_scope_before(scopes: &[ScopeMatch], offset: usize) -> Option<String> {
    scopes
        .iter()
        .rev()
        .find(|s| s.start < offset)
        .map(|s| s.scope.clone())
}

// ---------- Built-in framework definitions ----------

/// Regex fragment matching a translation-key literal (inside quotes).
const KEY_PATTERN: &str = r"[A-Za-z_][A-Za-z0-9_\-.:/\[\]]*";

fn compile(patterns: &[&str]) -> Vec<Regex> {
    patterns
        .iter()
        .map(|p| Regex::new(&p.replace("{key}", KEY_PATTERN)).expect("built-in regex must compile"))
        .collect()
}

fn framework_vue_i18n() -> Framework {
    Framework {
        id: "vue-i18n",
        name: "Vue I18n",
        language_ids: &["Vue.js", "TypeScript", "TSX", "JavaScript", "JSX"],
        usage_patterns: compile(&[
            // $t("key"), $tc("key"), $te("key")
            r#"(?:^|[^\w$.])\$t[ce]?\s*\(\s*['"`]({key})['"`]"#,
            // $rt("key")
            r#"(?:^|[^\w$.])\$rt\s*\(\s*['"`]({key})['"`]"#,
            // i18n.t("key"), i18n.global.t("key")
            r#"i18n(?:\.global)?\.t\s*\(\s*['"`]({key})['"`]"#,
            // <i18n-t keypath="key"> in templates
            r#"keypath\s*=\s*['"`]({key})['"`]"#,
        ]),
        scope_pattern: None,
        refactor_templates: &[r#"$t("$1")"#],
    }
}

fn framework_nuxt_i18n() -> Framework {
    // Nuxt i18n exposes `$t`, `useI18n`, `useTranslation` — same surface as vue-i18n
    // with a scope via `useI18n({ useScope: 'local' })` or `useTranslation('ns')`.
    Framework {
        id: "nuxt-i18n",
        name: "Nuxt I18n",
        language_ids: &["Vue.js", "TypeScript", "TSX", "JavaScript", "JSX"],
        usage_patterns: compile(&[
            r#"(?:^|[^\w$.])\$t[ce]?\s*\(\s*['"`]({key})['"`]"#,
            r#"(?:^|[^\w$.])t[ce]?\s*\(\s*['"`]({key})['"`]"#,
        ]),
        scope_pattern: Some(
            Regex::new(r#"useTranslation\s*\(\s*['"`]([A-Za-z0-9_.\-]+)['"`]"#)
                .expect("nuxt-i18n scope regex"),
        ),
        refactor_templates: &[r#"$t("$1")"#],
    }
}

fn framework_i18next() -> Framework {
    Framework {
        id: "i18next",
        name: "i18next",
        language_ids: &[
            "TypeScript",
            "TSX",
            "JavaScript",
            "JSX",
            "Vue.js",
            "HTML",
        ],
        usage_patterns: compile(&[
            // t("key") — not preceded by `$`, word chars or `.`
            r#"(?:^|[^\w$.])t\s*\(\s*['"`]({key})['"`]"#,
            // i18next.t("key")
            r#"i18next\.t\s*\(\s*['"`]({key})['"`]"#,
            // <Trans i18nKey="key">
            r#"i18nKey\s*=\s*['"`]({key})['"`]"#,
        ]),
        scope_pattern: Some(
            Regex::new(r#"useTranslation\s*\(\s*\[?\s*['"`]([A-Za-z0-9_.\-]+)['"`]"#)
                .expect("i18next scope regex"),
        ),
        refactor_templates: &[r#"t("$1")"#],
    }
}

fn framework_react_intl() -> Framework {
    Framework {
        id: "react-intl",
        name: "React Intl",
        language_ids: &["TypeScript", "TSX", "JavaScript", "JSX"],
        usage_patterns: compile(&[
            // formatMessage({ id: "key" })
            r#"formatMessage\s*\(\s*\{\s*id\s*:\s*['"`]({key})['"`]"#,
            // <FormattedMessage id="key">
            r#"<FormattedMessage[^>]*\bid\s*=\s*['"`]({key})['"`]"#,
        ]),
        scope_pattern: None,
        refactor_templates: &[r#"intl.formatMessage({ id: "$1" })"#],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vue_i18n_detects_dollar_t() {
        let src = r#"<template><button>{{ $t("common.submit") }}</button></template>"#;
        let usages = find_usages(src, "Vue.js");
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].key, "common.submit");
        assert_eq!(usages[0].framework_id, "vue-i18n");
    }

    #[test]
    fn vue_i18n_detects_composition_t() {
        let src = r#"const greeting = t("hello.world");"#;
        let usages = find_usages(src, "TypeScript");
        assert!(usages.iter().any(|u| u.key == "hello.world"));
    }

    #[test]
    fn vue_i18n_detects_keypath_attribute() {
        let src = r#"<i18n-t keypath="actions.save" :tag="false" />"#;
        let usages = find_usages(src, "Vue.js");
        assert!(usages.iter().any(|u| u.key == "actions.save"));
    }

    #[test]
    fn i18next_detects_trans_component() {
        let src = r#"<Trans i18nKey="welcome.message">Welcome!</Trans>"#;
        let usages = find_usages(src, "TSX");
        assert!(usages.iter().any(|u| u.key == "welcome.message"));
    }

    #[test]
    fn i18next_scope_from_use_translation() {
        let src = r#"
            const { t } = useTranslation('forms');
            const label = t('submit');
        "#;
        let usages = find_usages(src, "TypeScript");
        let submit = usages
            .iter()
            .find(|u| u.key.ends_with("submit"))
            .expect("submit usage");
        assert_eq!(submit.key, "forms.submit");
        assert_eq!(submit.scope.as_deref(), Some("forms"));
    }

    #[test]
    fn react_intl_detects_format_message() {
        let src = r#"intl.formatMessage({ id: "greeting.hello" })"#;
        let usages = find_usages(src, "TSX");
        assert!(usages.iter().any(|u| u.key == "greeting.hello"));
    }

    #[test]
    fn react_intl_detects_formatted_message() {
        let src = r#"<FormattedMessage id="greeting.hello" defaultMessage="Hi" />"#;
        let usages = find_usages(src, "TSX");
        assert!(usages.iter().any(|u| u.key == "greeting.hello"));
    }

    #[test]
    fn ignores_unrelated_function_calls() {
        let src = r#"const x = toString(other); const y = format("not a key");"#;
        let usages = find_usages(src, "TypeScript");
        assert_eq!(usages, vec![]);
    }

    #[test]
    fn records_precise_key_range() {
        let src = r#"t("abc.def")"#;
        let usages = find_usages(src, "TypeScript");
        let u = &usages[0];
        // The key literal "abc.def" starts at offset 3 and ends at 10 (inside the quotes)
        assert_eq!(&src[u.range.start.offset..u.range.end.offset], "abc.def");
    }

    #[test]
    fn skips_frameworks_for_unknown_language() {
        let src = r#"t("key.value")"#;
        let usages = find_usages(src, "Python");
        assert_eq!(usages, vec![]);
    }
}
