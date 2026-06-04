//! YAML locale parser using `saphyr` (`MarkedYaml`) for source positions.

use saphyr::LoadableYamlNode;
use saphyr::{MarkedYaml, ScanError, YamlData};

use super::{LocaleEntry, LocaleParser, ParseError};
use crate::position::{LineIndex, Range};

pub struct YamlParser;

impl LocaleParser for YamlParser {
    fn parse(&self, source: &str) -> Result<Vec<LocaleEntry>, ParseError> {
        let docs = MarkedYaml::load_from_str(source).map_err(scan_error)?;
        let line_index = LineIndex::new(source);
        let mut entries = Vec::new();

        for doc in docs {
            let mut path = Vec::new();
            walk_node(&doc, &mut path, None, &line_index, &mut entries);
        }

        Ok(entries)
    }
}

fn scan_error(err: ScanError) -> ParseError {
    ParseError::Syntax {
        offset: err.marker().index(),
        message: err.info().to_string(),
    }
}

fn span_to_range(span: &MarkedYaml<'_>, lines: &LineIndex) -> Range {
    lines.range(span.span.start.index(), span.span.end.index())
}

/// Extract a string leaf value from a scalar node.
fn scalar_string<'a>(node: &'a MarkedYaml<'_>) -> Option<&'a str> {
    match &node.data {
        YamlData::Value(scalar) => scalar.as_str(),
        YamlData::Representation(raw, _, _) => Some(raw.as_ref()),
        YamlData::Tagged(_, inner) => scalar_string(inner),
        _ => None,
    }
}

/// Mapping keys in locale files are expected to be plain string scalars.
fn mapping_key_name(key: &MarkedYaml<'_>) -> Option<String> {
    scalar_string(key).map(str::to_string)
}

fn walk_node(
    node: &MarkedYaml<'_>,
    path: &mut Vec<String>,
    key_range: Option<Range>,
    lines: &LineIndex,
    out: &mut Vec<LocaleEntry>,
) {
    match &node.data {
        YamlData::Mapping(map) => {
            for (key, value) in map.iter() {
                let Some(name) = mapping_key_name(key) else {
                    continue;
                };
                let kr = span_to_range(key, lines);
                path.push(name);
                walk_node(value, path, Some(kr), lines, out);
                path.pop();
            }
        }
        YamlData::Value(_) | YamlData::Representation(_, _, _) => {
            let Some(kr) = key_range else {
                return;
            };
            let Some(value) = scalar_string(node) else {
                return;
            };
            out.push(LocaleEntry {
                key_path: path.clone(),
                value: value.to_string(),
                range: span_to_range(node, lines),
                key_range: kr,
            });
        }
        YamlData::Tagged(_, inner) => walk_node(inner, path, key_range, lines, out),
        // Sequences, aliases, and bad nodes are ignored (pluralisation etc. come later).
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(src: &str) -> Vec<LocaleEntry> {
        YamlParser.parse(src).expect("parse ok")
    }

    #[test]
    fn flat_mapping() {
        let src = "hello: Hi\nbye: Bye\n";
        let e = entries(src);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].key_path, vec!["hello".to_string()]);
        assert_eq!(e[0].value, "Hi");
        assert_eq!(e[1].key_path, vec!["bye".to_string()]);
    }

    #[test]
    fn nested_mapping() {
        let src = r#"
common:
  submit: Submit
  cancel: Cancel
"#;
        let e = entries(src);
        assert_eq!(e.len(), 2);
        assert_eq!(
            e[0].key_path,
            vec!["common".to_string(), "submit".to_string()]
        );
        assert_eq!(
            e[1].key_path,
            vec!["common".to_string(), "cancel".to_string()]
        );
    }

    #[test]
    fn records_source_and_key_ranges() {
        let src = "hello: Hi\n";
        let e = entries(src);
        assert_eq!(&src[e[0].range.start.offset..e[0].range.end.offset], "Hi");
        assert_eq!(
            &src[e[0].key_range.start.offset..e[0].key_range.end.offset],
            "hello"
        );
        assert!(e[0].key_range.end.offset <= e[0].range.start.offset);
    }

    #[test]
    fn key_range_points_at_leaf_in_nested_mapping() {
        let src = "common:\n  submit: Submit\n";
        let e = entries(src);
        assert_eq!(
            &src[e[0].key_range.start.offset..e[0].key_range.end.offset],
            "submit"
        );
    }

    #[test]
    fn ignores_non_string_scalars_without_key() {
        let src = "42\n";
        let e = entries(src);
        assert!(e.is_empty());
    }

    #[test]
    fn ignores_non_string_values() {
        let src = "count: 3\n";
        let e = entries(src);
        assert!(e.is_empty());
    }

    #[test]
    fn quoted_keys_and_values() {
        let src = r#""hello": "Hi""#;
        let e = entries(src);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].value, "Hi");
    }

    #[test]
    fn reports_syntax_errors() {
        let src = "hello: [\n";
        let err = YamlParser.parse(src).unwrap_err();
        assert!(matches!(err, ParseError::Syntax { .. }));
    }
}
