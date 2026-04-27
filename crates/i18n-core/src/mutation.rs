//! In-place mutations of locale files (insert / rename / remove keys).
//!
//! The mutations go through `serde_json::Value` because it is the only parser
//! that (with the `preserve_order` feature) keeps insertion order stable.
//! Comments in JSONC/JSON5 and YAML are not yet supported — contributions
//! would be welcome.

use std::error::Error;
use std::fmt;

use serde::Serialize;
use serde_json::Value;

/// Errors raised while mutating a locale document.
#[derive(Debug)]
pub enum MutationError {
    /// Input was not parseable JSON.
    Parse(serde_json::Error),
    /// A segment on the way to the leaf was not an object, so the key can't
    /// be created without overwriting something.
    PathCollision(String),
    /// Leaf key already exists at the requested path.
    KeyAlreadyExists(String),
    /// Empty key path provided.
    EmptyPath,
    /// Re-serialisation of the mutated tree failed.
    Serialize(serde_json::Error),
}

impl fmt::Display for MutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "JSON parse error: {e}"),
            Self::PathCollision(p) => {
                write!(f, "path collision at `{p}` (non-object in the way)")
            }
            Self::KeyAlreadyExists(p) => write!(f, "key `{p}` already exists"),
            Self::EmptyPath => write!(f, "empty key path"),
            Self::Serialize(e) => write!(f, "JSON serialize error: {e}"),
        }
    }
}

impl Error for MutationError {}

/// Insert `value` at the dot-separated key `path` inside a JSON document.
///
/// - Missing intermediate objects are created.
/// - If the sibling keys are already in ascending lexicographic order, the
///   new key is inserted at the position that keeps them sorted. Otherwise
///   the key is appended at the end so any manual ordering stays intact.
/// - The original indent style (2/4 spaces, tabs) is detected and reapplied.
/// - A trailing newline in the input is kept in the output.
///
/// Returns the new file contents. The caller is responsible for turning that
/// into an LSP `WorkspaceEdit`.
pub fn insert_key_json(
    content: &str,
    path: &[&str],
    value: &str,
) -> Result<String, MutationError> {
    if path.is_empty() {
        return Err(MutationError::EmptyPath);
    }

    let mut root: Value = serde_json::from_str(content).map_err(MutationError::Parse)?;
    let indent = detect_indent(content);
    let trailing_newline = content.ends_with('\n');

    insert_into_value(&mut root, path, value)?;

    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    root.serialize(&mut ser).map_err(MutationError::Serialize)?;

    // Safe: `serde_json` always produces valid UTF-8.
    let mut out = String::from_utf8(buf).expect("serde_json emits UTF-8");
    if trailing_newline && !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn insert_into_value(
    root: &mut Value,
    path: &[&str],
    value: &str,
) -> Result<(), MutationError> {
    let mut current = root;
    for (i, part) in path.iter().enumerate() {
        let is_leaf = i == path.len() - 1;
        let map = current
            .as_object_mut()
            .ok_or_else(|| MutationError::PathCollision(path[..=i].join(".")))?;

        if is_leaf {
            if map.contains_key(*part) {
                return Err(MutationError::KeyAlreadyExists(path.join(".")));
            }
            sorted_or_append_insert(map, part, Value::String(value.to_string()));
            return Ok(());
        }

        if !map.contains_key(*part) {
            sorted_or_append_insert(map, part, Value::Object(serde_json::Map::new()));
        }
        current = map
            .get_mut(*part)
            .expect("branch was just inserted or already present");
    }
    Ok(())
}

/// Insert `key -> value` into `map`, respecting the existing ordering:
///
/// - If the existing sibling keys are already in ascending lexicographic
///   order (or the map is empty), the new key lands at the position that
///   keeps the order sorted. This matches the convention used by most JSON
///   linters and i18n tooling.
/// - Otherwise the entry is appended at the end so any manual ordering the
///   user has set up stays intact.
///
/// The caller must ensure `key` is absent from `map`.
fn sorted_or_append_insert(map: &mut serde_json::Map<String, Value>, key: &str, value: Value) {
    let keys: Vec<&str> = map.keys().map(String::as_str).collect();
    let already_sorted = keys.windows(2).all(|w| w[0] <= w[1]);
    if already_sorted {
        let idx = keys.partition_point(|k| *k < key);
        map.shift_insert(idx, key.to_string(), value);
    } else {
        map.insert(key.to_string(), value);
    }
}

/// Best-effort detection of the indent unit used in a JSON document.
///
/// Scans lines until it finds one with leading whitespace and returns that
/// exact prefix. Falls back to two spaces if nothing can be inferred (single
/// line or all top-level keys).
pub fn detect_indent(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        // Skip braces-only lines as they may have different indent than leaves.
        if trimmed.starts_with('}') || trimmed.starts_with(']') {
            continue;
        }
        let leading: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if !leading.is_empty() {
            return leading;
        }
    }
    "  ".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_into_existing_nested_path_preserves_order() {
        let input = r#"{
  "slots": {
    "a": "1",
    "b": "2"
  }
}
"#;
        let out = insert_key_json(input, &["slots", "c"], "3").unwrap();
        let expected = r#"{
  "slots": {
    "a": "1",
    "b": "2",
    "c": "3"
  }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn insert_into_sorted_map_places_key_in_alphabetical_position() {
        let input = r#"{
  "a": "1",
  "c": "3"
}
"#;
        let out = insert_key_json(input, &["b"], "2").unwrap();
        let expected = r#"{
  "a": "1",
  "b": "2",
  "c": "3"
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn insert_into_unsorted_map_appends_at_end_to_preserve_manual_order() {
        let input = r#"{
  "z": "26",
  "a": "1"
}
"#;
        let out = insert_key_json(input, &["m"], "13").unwrap();
        let expected = r#"{
  "z": "26",
  "a": "1",
  "m": "13"
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn new_intermediate_branch_is_inserted_alphabetically_at_top_level() {
        let input = r#"{
  "auth": {
    "login": "Log in"
  },
  "slots": {
    "table": "Table"
  }
}
"#;
        let out = insert_key_json(input, &["menu", "home"], "Home").unwrap();
        let expected = r#"{
  "auth": {
    "login": "Log in"
  },
  "menu": {
    "home": "Home"
  },
  "slots": {
    "table": "Table"
  }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn creates_missing_branches() {
        let input = "{}\n";
        let out = insert_key_json(input, &["a", "b", "c"], "hello").unwrap();
        let expected = r#"{
  "a": {
    "b": {
      "c": "hello"
    }
  }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn preserves_4_space_indent() {
        let input = "{\n    \"a\": \"1\"\n}\n";
        let out = insert_key_json(input, &["b"], "2").unwrap();
        assert_eq!(out, "{\n    \"a\": \"1\",\n    \"b\": \"2\"\n}\n");
    }

    #[test]
    fn preserves_tab_indent() {
        let input = "{\n\t\"a\": \"1\"\n}\n";
        let out = insert_key_json(input, &["b"], "2").unwrap();
        assert_eq!(out, "{\n\t\"a\": \"1\",\n\t\"b\": \"2\"\n}\n");
    }

    #[test]
    fn preserves_no_trailing_newline() {
        let input = "{\n  \"a\": \"1\"\n}";
        let out = insert_key_json(input, &["b"], "2").unwrap();
        assert_eq!(out, "{\n  \"a\": \"1\",\n  \"b\": \"2\"\n}");
    }

    #[test]
    fn errors_on_key_already_exists() {
        let input = "{\"a\": \"1\"}";
        let err = insert_key_json(input, &["a"], "2").unwrap_err();
        assert!(matches!(err, MutationError::KeyAlreadyExists(_)));
    }

    #[test]
    fn errors_on_path_collision_with_string() {
        let input = "{\"a\": \"1\"}";
        let err = insert_key_json(input, &["a", "b"], "2").unwrap_err();
        assert!(matches!(err, MutationError::PathCollision(_)));
    }

    #[test]
    fn errors_on_empty_path() {
        let err = insert_key_json("{}", &[], "x").unwrap_err();
        assert!(matches!(err, MutationError::EmptyPath));
    }

    #[test]
    fn errors_on_invalid_json() {
        let err = insert_key_json("not json", &["a"], "x").unwrap_err();
        assert!(matches!(err, MutationError::Parse(_)));
    }

    #[test]
    fn non_ascii_values_are_not_escaped() {
        let input = "{\n  \"a\": \"1\"\n}\n";
        let out = insert_key_json(input, &["b"], "éléphant 🐘").unwrap();
        assert!(out.contains("éléphant 🐘"));
    }

    #[test]
    fn detect_indent_two_spaces() {
        let s = "{\n  \"a\": 1\n}";
        assert_eq!(detect_indent(s), "  ");
    }

    #[test]
    fn detect_indent_four_spaces() {
        let s = "{\n    \"a\": 1\n}";
        assert_eq!(detect_indent(s), "    ");
    }

    #[test]
    fn detect_indent_tab() {
        let s = "{\n\t\"a\": 1\n}";
        assert_eq!(detect_indent(s), "\t");
    }

    #[test]
    fn detect_indent_falls_back_to_two_spaces() {
        assert_eq!(detect_indent("{}"), "  ");
        assert_eq!(detect_indent("{\"a\":1}"), "  ");
    }
}
