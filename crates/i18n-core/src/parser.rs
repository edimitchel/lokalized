//! Parsers for locale file formats (JSON, YAML, ARB, PHP, PO, …) with source positions.

pub mod json;

use std::path::Path;

use crate::position::Range;

#[derive(thiserror::Error, Debug)]
pub enum ParseError {
    #[error("syntax error at byte offset {offset}: {message}")]
    Syntax { offset: usize, message: String },
    #[error("I/O error reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("unsupported locale file format: {0}")]
    Unsupported(String),
}

/// One leaf entry extracted from a locale file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocaleEntry {
    pub key_path: Vec<String>,
    pub value: String,
    pub range: Range,
}

pub trait LocaleParser {
    fn parse(&self, source: &str) -> Result<Vec<LocaleEntry>, ParseError>;
}

/// Parse a locale file on disk, dispatching on the extension.
pub fn parse_file(path: &Path) -> Result<Vec<LocaleEntry>, ParseError> {
    let source = std::fs::read_to_string(path).map_err(|e| ParseError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    parse_with_extension(&source, path)
}

/// Parse a source string, dispatching based on the file extension of `path`.
pub fn parse_with_extension(source: &str, path: &Path) -> Result<Vec<LocaleEntry>, ParseError> {
    match path.extension().and_then(|s| s.to_str()) {
        Some("json" | "jsonc" | "json5" | "arb") => json::JsonParser.parse(source),
        Some(other) => Err(ParseError::Unsupported(other.to_string())),
        None => Err(ParseError::Unsupported(path.display().to_string())),
    }
}
