//! Core i18n logic: locale parsing, key indexing, framework detection.
//!
//! Kept free of LSP/MCP/async dependencies so it can be reused by the
//! language server, MCP server, and future CLI tools.

pub mod config;
pub mod display;
pub mod framework;
pub mod index;
pub mod locale;
pub mod mutation;
pub mod parser;
pub mod position;

pub use config::{KeyStyle, ProjectConfig};
pub use display::{escape_md, truncate_chars, ParsedValue};
pub use framework::{find_usages, Framework, KeyUsage, BUILTIN_FRAMEWORKS};
pub use index::{IndexBuilder, IndexError, KeyNode, KeyTree, LocaleIndex, LocalizedValue};
pub use locale::{Locale, LocaleFile, LocaleLayout};
pub use mutation::{detect_indent, insert_key_json, MutationError};
pub use parser::{parse_file, LocaleEntry, LocaleParser, ParseError};
pub use position::{LineIndex, Position, Range};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
