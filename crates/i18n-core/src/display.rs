//! Human-friendly formatting of translation values.
//!
//! Translation strings are rarely plain: Vue I18n separates plural forms with
//! `|`, ICU MessageFormat uses nested braces, and most libraries support
//! `{name}` interpolation. This module parses these patterns so hover popups
//! and inlay hints can render them nicely.

/// A parsed translation value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedValue<'a> {
    /// Individual plural forms (vue-i18n style). A non-pluralised string has
    /// exactly one form.
    pub forms: Vec<&'a str>,
    /// The raw source string.
    pub raw: &'a str,
    /// Whether the string contains `{placeholder}` interpolations.
    pub has_interpolation: bool,
    /// Whether the string looks like ICU MessageFormat (`{count, plural, ...}`).
    pub has_icu: bool,
}

impl<'a> ParsedValue<'a> {
    /// Parse a translation value.
    pub fn parse(raw: &'a str) -> Self {
        let has_icu = raw.contains(", plural,")
            || raw.contains(", select,")
            || raw.contains(", selectordinal,");
        // Splitting on `|` is the Vue I18n convention. Don't split when the
        // value looks like ICU (it uses `|` inside `{}` and splitting would
        // break it).
        let forms: Vec<&str> = if has_icu {
            vec![raw]
        } else {
            raw.split('|').map(str::trim).collect()
        };
        let has_interpolation = raw.contains('{') && raw.contains('}');
        Self {
            forms,
            raw,
            has_interpolation,
            has_icu,
        }
    }

    /// Is this a pluralised value (more than one form)?
    pub fn is_plural(&self) -> bool {
        self.forms.len() > 1
    }

    /// The form most representative of the translation, used for compact
    /// previews like inlay hints. For plurals, prefer the singular (second
    /// form in `"no | one | other"` patterns; first when only two forms).
    pub fn primary_form(&self) -> &str {
        match self.forms.as_slice() {
            [single] => single,
            [_zero, one, ..] if self.forms.len() >= 3 => one,
            [one, _other] => one,
            _ => self.raw,
        }
    }

    /// Human-readable label for a given plural form index, following the
    /// vue-i18n convention (2 forms = singular/plural, 3 forms = zero/one/other).
    pub fn form_label(&self, index: usize) -> &'static str {
        match (self.forms.len(), index) {
            (2, 0) => "one",
            (2, 1) => "other",
            (3, 0) => "zero",
            (3, 1) => "one",
            (3, 2) => "other",
            (_, _) => "form",
        }
    }
}

/// Truncate `s` to at most `max` characters (Unicode-aware), appending `…`.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let prefix: String = s.chars().take(max).collect();
        format!("{prefix}…")
    }
}

/// Escape special characters for safe inclusion in markdown.
pub fn escape_md(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace('*', "\\*")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_value_has_one_form() {
        let p = ParsedValue::parse("Hello");
        assert_eq!(p.forms, vec!["Hello"]);
        assert!(!p.is_plural());
        assert!(!p.has_interpolation);
    }

    #[test]
    fn detects_interpolation() {
        let p = ParsedValue::parse("Welcome, {name}!");
        assert!(p.has_interpolation);
        assert!(!p.is_plural());
    }

    #[test]
    fn splits_vue_i18n_plural() {
        let p = ParsedValue::parse("no apples | one apple | {n} apples");
        assert_eq!(p.forms.len(), 3);
        assert_eq!(p.forms, vec!["no apples", "one apple", "{n} apples"]);
        assert!(p.is_plural());
    }

    #[test]
    fn primary_form_prefers_singular() {
        let p = ParsedValue::parse("no apples | one apple | {n} apples");
        assert_eq!(p.primary_form(), "one apple");

        let p2 = ParsedValue::parse("item | items");
        assert_eq!(p2.primary_form(), "item");

        let p3 = ParsedValue::parse("Hello");
        assert_eq!(p3.primary_form(), "Hello");
    }

    #[test]
    fn form_labels_follow_vue_convention() {
        let p = ParsedValue::parse("a | b | c");
        assert_eq!(p.form_label(0), "zero");
        assert_eq!(p.form_label(1), "one");
        assert_eq!(p.form_label(2), "other");

        let p2 = ParsedValue::parse("a | b");
        assert_eq!(p2.form_label(0), "one");
        assert_eq!(p2.form_label(1), "other");
    }

    #[test]
    fn icu_values_are_not_split_on_pipe() {
        let src = "{count, plural, one {# item} other {# items | yes}}";
        let p = ParsedValue::parse(src);
        assert!(p.has_icu);
        assert!(!p.is_plural());
        assert_eq!(p.forms, vec![src]);
    }

    #[test]
    fn truncate_preserves_short_strings() {
        assert_eq!(truncate_chars("hi", 10), "hi");
    }

    #[test]
    fn truncate_appends_ellipsis() {
        assert_eq!(truncate_chars("hello world", 5), "hello…");
    }

    #[test]
    fn truncate_handles_multibyte_chars() {
        assert_eq!(truncate_chars("héllo", 4), "héll…");
        // Should NOT panic on emoji
        assert_eq!(truncate_chars("😀😀😀😀", 2), "😀😀…");
    }

    #[test]
    fn escape_md_escapes_markdown_specials() {
        assert_eq!(escape_md("a*b_c`d"), "a\\*b\\_c\\`d");
    }
}
