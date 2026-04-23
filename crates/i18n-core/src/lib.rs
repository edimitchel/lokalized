//! Core i18n logic: locale parsing, key indexing, framework detection.
//!
//! Kept free of LSP/MCP/async dependencies so it can be reused by the
//! language server, MCP server, and future CLI tools.

pub mod framework;
pub mod index;
pub mod locale;
pub mod parser;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
