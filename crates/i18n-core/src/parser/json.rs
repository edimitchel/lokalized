//! JSON / JSONC / JSON5 / ARB parser using `jsonc-parser`, preserving source positions.

use jsonc_parser::ast::{Object, Value};
use jsonc_parser::common::Ranged;
use jsonc_parser::{CollectOptions, CommentCollectionStrategy, ParseOptions};

use super::{LocaleEntry, LocaleParser, ParseError};
use crate::position::LineIndex;

pub struct JsonParser;

impl LocaleParser for JsonParser {
    fn parse(&self, source: &str) -> Result<Vec<LocaleEntry>, ParseError> {
        let parse_result = jsonc_parser::parse_to_ast(
            source,
            &CollectOptions {
                comments: CommentCollectionStrategy::Off,
                tokens: false,
            },
            &ParseOptions::default(),
        )
        .map_err(|e| ParseError::Syntax {
            offset: e.range().start,
            message: e.kind().to_string(),
        })?;

        let line_index = LineIndex::new(source);
        let mut entries = Vec::new();

        if let Some(root) = parse_result.value {
            let mut path = Vec::new();
            walk_value(&root, &mut path, None, &line_index, &mut entries);
        }

        Ok(entries)
    }
}

fn walk_value(
    value: &Value,
    path: &mut Vec<String>,
    key_range: Option<crate::position::Range>,
    lines: &LineIndex,
    out: &mut Vec<LocaleEntry>,
) {
    match value {
        Value::StringLit(lit) => {
            // Top-level string with no parent property is meaningless as a
            // translation entry — skip it. Every real entry has a key range
            // because the parser only descends into properties of objects.
            let Some(kr) = key_range else { return };
            out.push(LocaleEntry {
                key_path: path.clone(),
                value: lit.value.to_string(),
                range: lines.range(lit.start(), lit.end()),
                key_range: kr,
            });
        }
        Value::Object(obj) => walk_object(obj, path, lines, out),
        // Numbers / booleans / null / arrays are ignored for Phase 1 (ARB metadata,
        // pluralisation, etc. will be handled in later phases).
        _ => {}
    }
}

fn walk_object(
    obj: &Object,
    path: &mut Vec<String>,
    lines: &LineIndex,
    out: &mut Vec<LocaleEntry>,
) {
    for prop in &obj.properties {
        let name = prop.name.as_str();
        // ARB metadata keys start with `@` — skip them.
        if name.starts_with('@') {
            continue;
        }
        let key_range = lines.range(prop.name.start(), prop.name.end());
        path.push(name.to_string());
        walk_value(&prop.value, path, Some(key_range), lines, out);
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(src: &str) -> Vec<LocaleEntry> {
        JsonParser.parse(src).expect("parse ok")
    }

    #[test]
    fn flat_object() {
        let src = r#"{ "hello": "Hi", "bye": "Bye" }"#;
        let e = entries(src);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].key_path, vec!["hello".to_string()]);
        assert_eq!(e[0].value, "Hi");
        assert_eq!(e[1].key_path, vec!["bye".to_string()]);
    }

    #[test]
    fn nested_object() {
        let src = r#"{ "common": { "submit": "Submit", "cancel": "Cancel" } }"#;
        let e = entries(src);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].key_path, vec!["common".to_string(), "submit".to_string()]);
        assert_eq!(e[1].key_path, vec!["common".to_string(), "cancel".to_string()]);
    }

    #[test]
    fn ignores_arb_metadata() {
        let src = r#"{
            "@@locale": "en",
            "hello": "Hi",
            "@hello": { "description": "greeting" }
        }"#;
        let e = entries(src);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].key_path, vec!["hello".to_string()]);
    }

    #[test]
    fn records_source_range() {
        let src = r#"{"x":"Hi"}"#;
        let e = entries(src);
        // "Hi" starts at byte 5 (including opening quote), ends at 9
        assert_eq!(e[0].range.start.offset, 5);
        assert_eq!(e[0].range.end.offset, 9);
        assert_eq!(e[0].range.start.line, 0);
        assert_eq!(e[0].range.start.character, 5);
    }

    #[test]
    fn records_key_range_separately_from_value_range() {
        let src = r#"{"hello": "Hi"}"#;
        let e = entries(src);
        // The key `"hello"` (with quotes) spans bytes 1..8.
        assert_eq!(&src[e[0].key_range.start.offset..e[0].key_range.end.offset], "\"hello\"");
        // And it must not overlap with the value range.
        assert!(e[0].key_range.end.offset <= e[0].range.start.offset);
    }

    #[test]
    fn key_range_points_at_leaf_property_in_nested_object() {
        let src = r#"{ "common": { "submit": "Submit" } }"#;
        let e = entries(src);
        // The leaf entry's key range should point at `"submit"`, not `"common"`.
        assert_eq!(
            &src[e[0].key_range.start.offset..e[0].key_range.end.offset],
            "\"submit\"",
        );
    }

    #[test]
    fn jsonc_comments_allowed() {
        let src = r#"{
            // greeting
            "hello": "Hi"
        }"#;
        let e = entries(src);
        assert_eq!(e.len(), 1);
    }

    #[test]
    fn reports_syntax_errors() {
        let src = r#"{ "hello": }"#;
        let err = JsonParser.parse(src).unwrap_err();
        match err {
            ParseError::Syntax { .. } => {}
            _ => panic!("expected syntax error"),
        }
    }
}
