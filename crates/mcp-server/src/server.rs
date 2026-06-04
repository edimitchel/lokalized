//! MCP tool handlers backed by `i18n-core`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use i18n_core::{
    parse_linked_message, resolve_value, set_key_json, Locale, LocaleIndex, ProjectSnapshot,
    ResolvedValue,
};
use rmcp::{
    handler::server::wrapper::Parameters, schemars::JsonSchema, tool, tool_handler, tool_router,
    ServerHandler,
};
use serde::Deserialize;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct LokalizedMcp {
    workspace: PathBuf,
    index: Arc<RwLock<Option<LocaleIndex>>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListKeysParams {
    /// BCP-47 locale id (e.g. `en`, `fr`). Omit to list keys from every locale.
    locale: Option<String>,
    /// Only return keys starting with this prefix (e.g. `common.`).
    prefix: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetValueParams {
    /// Dot-separated translation key.
    key: String,
    /// Target locale id.
    locale: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetValueParams {
    key: String,
    locale: String,
    value: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FindMissingParams {
    /// Locale to check against the source locale.
    locale: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TranslateKeyParams {
    key: String,
    target_locale: String,
    /// `deepl` or `openai` (reserved for a future release).
    engine: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExtractParams {
    /// Source text to turn into a translation key.
    text: String,
    /// Optional file path hint for namespacing (e.g. `src/components/Login.vue`).
    file_context: Option<String>,
}

impl LokalizedMcp {
    pub fn new(workspace: PathBuf) -> anyhow::Result<Self> {
        let index = match build_index(&workspace) {
            Ok(index) => Some(index),
            Err(message) => {
                tracing::warn!(
                    root = %workspace.display(),
                    %message,
                    "starting without a locale index — add locale files or `.zed/lokalized.json`, then retry"
                );
                None
            }
        };
        Ok(Self {
            workspace,
            index: Arc::new(RwLock::new(index)),
        })
    }

    async fn with_index<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&LocaleIndex) -> Result<T, String>,
    {
        let guard = self.index.read().await;
        let idx = guard.as_ref().ok_or_else(|| {
            "no locale index for this workspace yet — add `locales/` (or configure `.zed/lokalized.json`) and call `i18n.refresh_index`".to_string()
        })?;
        f(idx)
    }

    async fn refresh_index(&self) -> Result<(), String> {
        let index = build_index(&self.workspace)?;
        *self.index.write().await = Some(index);
        Ok(())
    }
}

fn build_index(workspace: &Path) -> Result<LocaleIndex, String> {
    ProjectSnapshot::load(workspace)
        .map(|snap| snap.index)
        .map_err(|e| e.to_string())
}

fn slugify_segment(input: &str) -> String {
    let mut out = String::new();
    let mut prev_underscore = false;
    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

#[tool_router]
impl LokalizedMcp {
    #[tool(description = "List translation keys in the workspace")]
    async fn list_keys(
        &self,
        Parameters(params): Parameters<ListKeysParams>,
    ) -> Result<String, String> {
        self.with_index(|idx| {
            let prefix = params.prefix.as_deref().unwrap_or("");
            let locale_filter = params.locale.as_deref().map(Locale::new);

            let mut keys: Vec<String> = idx
                .all_keys()
                .into_iter()
                .filter(|k| prefix.is_empty() || k.starts_with(prefix))
                .filter(|k| {
                    locale_filter
                        .as_ref()
                        .is_none_or(|loc| idx.lookup(k).contains_key(loc))
                })
                .collect();
            keys.sort();
            serde_json::to_string_pretty(&keys).map_err(|e| e.to_string())
        })
        .await
    }

    #[tool(description = "Get the translation value for a key in a locale")]
    async fn get_value(
        &self,
        Parameters(params): Parameters<GetValueParams>,
    ) -> Result<String, String> {
        self.with_index(|idx| {
            let locale = Locale::new(&params.locale);
            let values = idx.lookup(&params.key);
            let Some(value) = values.get(&locale) else {
                return Err(format!(
                    "key `{}` not defined for locale `{}`",
                    params.key, params.locale
                ));
            };
            let mut out = serde_json::json!({
                "key": params.key,
                "locale": params.locale,
                "value": value.value,
                "file": value.file.display().to_string(),
            });
            match resolve_value(idx, &locale, &value.value) {
                ResolvedValue::Literal { text } => {
                    out["resolved"] = serde_json::Value::String(text.to_string());
                }
                ResolvedValue::Linked {
                    display,
                    target_key,
                    chain,
                    ..
                } => {
                    out["resolved"] = serde_json::Value::String(display);
                    out["link"] = serde_json::json!({
                        "target": target_key,
                        "chain": chain,
                    });
                }
                ResolvedValue::Broken { target_key, reason } => {
                    out["resolved"] = serde_json::Value::Null;
                    out["linkError"] = serde_json::json!({
                        "target": target_key,
                        "reason": reason,
                    });
                }
            }
            if let Some(link) = parse_linked_message(&value.value) {
                out["isLinked"] = serde_json::Value::Bool(true);
                if let Some(m) = link.modifier {
                    out["modifier"] = serde_json::Value::String(m.to_string());
                }
            }
            Ok(out.to_string())
        })
        .await
    }

    #[tool(description = "Write a translation value to a locale file (JSON only for now)")]
    async fn set_value(
        &self,
        Parameters(params): Parameters<SetValueParams>,
    ) -> Result<String, String> {
        let result = self.set_value_sync(&params).await;
        if result.is_ok() {
            let _ = self.refresh_index().await;
        }
        result
    }

    #[tool(description = "List keys present in the source locale but missing from a target locale")]
    async fn find_missing(
        &self,
        Parameters(params): Parameters<FindMissingParams>,
    ) -> Result<String, String> {
        self.with_index(|idx| {
            let locale = Locale::new(&params.locale);
            let missing = idx.missing_keys(&locale);
            serde_json::to_string_pretty(&missing).map_err(|e| e.to_string())
        })
        .await
    }

    #[tool(
        description = "Suggest a translation key name for a piece of text (does not write files)"
    )]
    fn extract(&self, Parameters(params): Parameters<ExtractParams>) -> Result<String, String> {
        let slug = slugify_segment(&params.text);
        if slug.is_empty() {
            return Err("text is empty after slugify".to_string());
        }
        let key = if let Some(ctx) = &params.file_context {
            let stem = Path::new(ctx)
                .file_stem()
                .and_then(|s| s.to_str())
                .map(slugify_segment)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "context".to_string());
            format!("{stem}.{slug}")
        } else {
            slug
        };
        Ok(serde_json::json!({ "suggested_key": key }).to_string())
    }

    #[tool(
        description = "Translate a key via DeepL/OpenAI (requires API keys; not yet implemented)"
    )]
    fn translate_key(
        &self,
        Parameters(TranslateKeyParams {
            key,
            target_locale,
            engine,
        }): Parameters<TranslateKeyParams>,
    ) -> Result<String, String> {
        let _ = (key, target_locale, engine);
        let has_deepl = std::env::var("LOKALIZED_DEEPL_KEY")
            .or_else(|_| std::env::var("LOKALIZE_DEEPL_KEY"))
            .or_else(|_| std::env::var("DEEPL_API_KEY"))
            .is_ok();
        let has_openai = std::env::var("OPENAI_API_KEY").is_ok();
        if !has_deepl && !has_openai {
            return Err(
                "no translation API key found (set LOKALIZED_DEEPL_KEY or OPENAI_API_KEY)".into(),
            );
        }
        Err(
            "automatic translation is not implemented yet — use set_value after translating externally"
                .into(),
        )
    }

    #[tool(description = "Reload the locale index from disk")]
    async fn reload_index(&self) -> Result<String, String> {
        self.refresh_index().await?;
        self.with_index(|idx| {
            Ok(format!(
                "indexed {} locale(s), {} key(s)",
                idx.trees.len(),
                idx.all_keys().len()
            ))
        })
        .await
    }
}

impl LokalizedMcp {
    async fn set_value_sync(&self, params: &SetValueParams) -> Result<String, String> {
        let path = {
            let guard = self.index.read().await;
            let idx = guard.as_ref().ok_or("index not loaded")?;
            let locale = Locale::new(&params.locale);
            idx.locale_file_path(&locale)
                .ok_or_else(|| format!("no locale file for `{}`", params.locale))?
        };

        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !matches!(ext, "json" | "jsonc" | "json5") {
            return Err(format!(
                "set_value only supports JSON locale files right now (got .{ext})"
            ));
        }

        let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let segments: Vec<&str> = params.key.split('.').collect();
        let new_content =
            set_key_json(&content, &segments, &params.value).map_err(|e| e.to_string())?;
        std::fs::write(&path, &new_content).map_err(|e| e.to_string())?;

        Ok(serde_json::json!({
            "key": params.key,
            "locale": params.locale,
            "file": path.display().to_string(),
            "written": true,
        })
        .to_string())
    }
}

#[tool_handler]
impl ServerHandler for LokalizedMcp {}
