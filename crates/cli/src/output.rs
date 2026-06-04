use std::collections::BTreeMap;


use i18n_core::ParseIssue;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Serialize)]
pub struct CheckReport {
    pub ok: bool,
    pub missing: BTreeMap<String, Vec<String>>,
    pub unused: Vec<String>,
    pub parse_errors: Vec<ParseIssueJson>,
}

#[derive(Serialize)]
pub struct ParseIssueJson {
    pub path: String,
    pub message: String,
}

impl From<ParseIssue> for ParseIssueJson {
    fn from(p: ParseIssue) -> Self {
        Self {
            path: p.path.display().to_string(),
            message: p.message,
        }
    }
}

pub fn emit_check(
    format: OutputFormat,
    missing: BTreeMap<String, Vec<String>>,
    unused: Vec<String>,
    parse_errors: Vec<ParseIssue>,
) -> anyhow::Result<()> {
    let ok = missing.values().all(|v| v.is_empty()) && unused.is_empty() && parse_errors.is_empty();
    let report = CheckReport {
        ok,
        missing,
        unused,
        parse_errors: parse_errors.into_iter().map(Into::into).collect(),
    };
    emit(format, &report)
}

pub fn emit<T: Serialize>(format: OutputFormat, value: &T) -> anyhow::Result<()> {
    match format {
        OutputFormat::Text => emit_text(value),
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(value)?);
            Ok(())
        }
    }
}

fn emit_text<T: Serialize>(value: &T) -> anyhow::Result<()> {
    let v = serde_json::to_value(value)?;
    print_json_value(&v, 0);
    Ok(())
}

fn print_json_value(v: &serde_json::Value, indent: usize) {
    let pad = "  ".repeat(indent);
    match v {
        serde_json::Value::Null => println!("{pad}null"),
        serde_json::Value::Bool(b) => println!("{pad}{b}"),
        serde_json::Value::Number(n) => println!("{pad}{n}"),
        serde_json::Value::String(s) => println!("{pad}{s}"),
        serde_json::Value::Array(arr) if arr.is_empty() => println!("{pad}(empty)"),
        serde_json::Value::Array(arr) => {
            for item in arr {
                print_json_value(item, indent);
            }
        }
        serde_json::Value::Object(map) if map.is_empty() => println!("{pad}(empty)"),
        serde_json::Value::Object(map) => {
            for (k, val) in map {
                if val.is_array() || val.is_object() {
                    println!("{pad}{k}:");
                    print_json_value(val, indent + 1);
                } else {
                    print_json_value(val, indent);
                }
            }
        }
    }
}

