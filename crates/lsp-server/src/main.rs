//! `lokalize-lsp` — Language server binary.
//!
//! - **Phase 0**: lifecycle (initialize/initialized/shutdown).
//! - **Phase 1**: workspace indexing — discovers locale files, parses them, keeps an
//!   in-memory `LocaleIndex`.
//! - **Phase 3** (current): hover, inlay hints, go-to-definition, completion and
//!   diagnostics for translation-key usages in source files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use i18n_core::{
    escape_md, find_usages, insert_key_json, remove_key_json, truncate_chars, IndexBuilder,
    KeyUsage, LineIndex, LocaleFile, LocaleIndex, LocaleLayout, LocalizedValue, ParsedValue,
    ProjectConfig, UsageIndex,
};
use notify::{Event as NotifyEvent, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{mpsc, RwLock};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams, CodeActionProviderCapability,
    CodeActionResponse, CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams,
    CompletionResponse, Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, ExecuteCommandParams,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, InitializeParams, InitializeResult, InitializedParams, InlayHint,
    InlayHintKind, InlayHintLabel, InlayHintParams, Location, MarkupContent, MarkupKind,
    MessageType, OneOf, Position as LspPosition, Range as LspRange, ServerCapabilities, ServerInfo,
    SymbolInformation, SymbolKind, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit,
    Url, WorkspaceEdit, WorkspaceSymbolParams,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

/// Snapshot of an open document used to re-run usage detection on demand.
#[derive(Clone, Debug)]
struct DocumentState {
    text: String,
    language_id: String,
    version: i32,
}

type DocumentStore = Arc<RwLock<HashMap<Url, DocumentState>>>;
type IndexSlot = Arc<RwLock<Option<LocaleIndex>>>;
type UsageSlot = Arc<RwLock<UsageIndex>>;
type WatcherSlot = Arc<StdMutex<Option<RecommendedWatcher>>>;

struct Backend {
    client: Client,
    /// Locale index. `None` until the first workspace folder is indexed.
    index: IndexSlot,
    /// Reverse usage index: which translation keys are referenced from each
    /// source file. Drives the "unused translation" diagnostic on locale
    /// files. Populated by an initial project walk and kept up to date on
    /// every `did_change`/`did_open` of a source document.
    usages: UsageSlot,
    /// Text + language id of every currently-open document.
    documents: DocumentStore,
    /// Filesystem watcher for hot-reload of locale files. Kept alive for the
    /// lifetime of the server; its background thread holds a Sender to the
    /// rebuild loop, which is dropped (and thus stops the loop) when `Backend`
    /// is dropped.
    _watcher: WatcherSlot,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        info!("-> initialize request received");
        let roots = collect_workspace_roots(&params);
        info!(?roots, "initialize: {} workspace root(s)", roots.len());

        // Index building happens after `initialized` so the handshake stays fast.
        let index_slot = Arc::clone(&self.index);
        let usage_slot = Arc::clone(&self.usages);
        let documents = Arc::clone(&self.documents);
        let watcher_slot = Arc::clone(&self._watcher);
        let client = self.client.clone();
        tokio::spawn(async move {
            build_indexes_for_roots(
                roots,
                index_slot,
                usage_slot,
                documents,
                client,
                watcher_slot,
            )
            .await;
        });

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                definition_provider: Some(OneOf::Left(true)),
                // Provider declared but handler returns None until Phase 4
                // wires up WorkspaceEdit-based actions.
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        "\"".into(),
                        "'".into(),
                        "`".into(),
                        ".".into(),
                    ]),
                    resolve_provider: Some(false),
                    ..Default::default()
                }),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: env!("CARGO_PKG_NAME").into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        info!("-> initialized notification received");
        self.client
            .log_message(MessageType::INFO, "Lokalize LSP ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    // ---------- Document lifecycle ----------

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        info!(uri = %doc.uri, lang = %doc.language_id, "did_open");
        self.documents.write().await.insert(
            doc.uri.clone(),
            DocumentState {
                text: doc.text.clone(),
                language_id: doc.language_id.clone(),
                version: doc.version,
            },
        );

        let usages_changed = self
            .maybe_update_usages(&doc.uri, &doc.language_id, &doc.text)
            .await;
        self.publish_diagnostics(&doc.uri).await;
        if usages_changed {
            self.republish_locale_diagnostics().await;
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let snapshot = if let Some(change) = params.content_changes.into_iter().next() {
            let mut store = self.documents.write().await;
            store.get_mut(&uri).map(|doc| {
                doc.text = change.text;
                doc.version = params.text_document.version;
                (doc.text.clone(), doc.language_id.clone())
            })
        } else {
            None
        };

        let usages_changed = if let Some((text, lang)) = snapshot.as_ref() {
            self.maybe_update_usages(&uri, lang, text).await
        } else {
            false
        };

        self.publish_diagnostics(&uri).await;
        if usages_changed {
            self.republish_locale_diagnostics().await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.write().await.remove(&uri);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    // ---------- Feature handlers ----------

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let Some(doc) = self.documents.read().await.get(&uri).cloned() else {
            return Ok(None);
        };
        let Some(usage) = usage_at_position(&doc, pos) else {
            return Ok(None);
        };

        let index_guard = self.index.read().await;
        let Some(idx) = &*index_guard else {
            return Ok(None);
        };
        let values = idx.lookup(&usage.key);
        if values.is_empty() {
            return Ok(None);
        }

        let md = format_hover_markdown(&usage.key, &values, &idx.source_locale);

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: md,
            }),
            range: Some(to_lsp_range(&usage.range)),
        }))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        let Some(doc) = self.documents.read().await.get(&uri).cloned() else {
            return Ok(None);
        };
        let usages = find_usages(&doc.text, &doc.language_id);

        let index_guard = self.index.read().await;
        let Some(idx) = &*index_guard else {
            return Ok(None);
        };
        let source = &idx.source_locale;

        let hints: Vec<InlayHint> = usages
            .iter()
            .filter_map(|u| {
                let values = idx.lookup(&u.key);
                let value = values.get(source).copied()?;
                Some(build_inlay_hint(u, value))
            })
            .collect();

        info!(
            uri = %uri,
            lang = %doc.language_id,
            usages = usages.len(),
            hints = hints.len(),
            "inlay_hint",
        );

        Ok(Some(hints))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let Some(doc) = self.documents.read().await.get(&uri).cloned() else {
            return Ok(None);
        };
        let Some(usage) = usage_at_position(&doc, pos) else {
            return Ok(None);
        };

        let index_guard = self.index.read().await;
        let Some(idx) = &*index_guard else {
            return Ok(None);
        };

        let locations: Vec<Location> = idx
            .lookup(&usage.key)
            .values()
            .filter_map(|v| localized_value_to_location(v))
            .collect();

        if locations.is_empty() {
            Ok(None)
        } else {
            Ok(Some(GotoDefinitionResponse::Array(locations)))
        }
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri.clone();
        let pos = params.range.start;
        info!(%uri, line = pos.line, character = pos.character, "code_action request");

        // Locale JSON files: only "Remove unused key" is offered. The source-
        // side actions below assume a translation-call usage at the cursor,
        // which never matches inside a JSON document.
        if is_indexed_locale_uri(&self.index, &uri).await {
            let actions = build_locale_code_actions(
                &self.documents,
                &self.index,
                &self.usages,
                &uri,
                pos,
            )
            .await;
            return Ok(actions);
        }

        let Some(doc) = self.documents.read().await.get(&uri).cloned() else {
            info!("code_action: no document state");
            return Ok(None);
        };
        let Some(usage) = usage_at_position(&doc, pos) else {
            info!("code_action: no usage at position");
            return Ok(None);
        };
        info!(key = %usage.key, "code_action: usage found");

        let index_guard = self.index.read().await;
        let Some(idx) = &*index_guard else {
            info!("code_action: no index");
            return Ok(None);
        };

        let actions = build_fill_missing_actions(idx, &usage.key).await;
        info!(count = actions.len(), "code_action: built actions");

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> Result<Option<serde_json::Value>> {
        // No custom commands registered yet. Reserved for Phase 4.
        warn!("unknown command: {}", params.command);
        Ok(None)
    }

    async fn completion(
        &self,
        _params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
        let index_guard = self.index.read().await;
        let Some(idx) = &*index_guard else {
            return Ok(None);
        };

        let source = &idx.source_locale;
        let items: Vec<CompletionItem> = idx
            .all_keys()
            .into_iter()
            .map(|key| {
                let preview = idx
                    .lookup(&key)
                    .get(source)
                    .map(|v| truncate_chars(&v.value, 60));
                CompletionItem {
                    label: key.clone(),
                    kind: Some(CompletionItemKind::TEXT),
                    detail: preview,
                    filter_text: Some(key.clone()),
                    insert_text: Some(key),
                    ..Default::default()
                }
            })
            .collect();

        Ok(Some(CompletionResponse::Array(items)))
    }

    /// Workspace-wide fuzzy search over translation keys (Zed: Cmd+T).
    ///
    /// Each match yields a [`SymbolInformation`] pointing at the key's
    /// definition in the source locale (or any locale that defines it),
    /// so jumping straight to the JSON entry from the picker works.
    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let query = params.query.trim().to_string();
        info!(query = %query, "workspace/symbol request");

        let guard = self.index.read().await;
        let Some(idx) = &*guard else {
            return Ok(None);
        };

        // Case-insensitive substring match. Zed runs its own fuzzy match on
        // the returned list so a permissive server-side filter is enough.
        let needle = query.to_lowercase();
        let source = &idx.source_locale;
        let max_results = 200_usize;

        let mut symbols: Vec<SymbolInformation> = Vec::new();
        for key in idx.all_keys() {
            if !needle.is_empty() && !key.to_lowercase().contains(&needle) {
                continue;
            }
            let values = idx.lookup(&key);
            if values.is_empty() {
                continue;
            }
            // Prefer the source locale so the picker preview shows the
            // canonical translation. Fall back to whichever locale has it.
            let value = values
                .get(source)
                .copied()
                .or_else(|| values.values().next().copied());
            let Some(value) = value else { continue };

            let Ok(uri) = Url::from_file_path(&value.file) else {
                continue;
            };

            #[allow(deprecated)] // SymbolInformation::deprecated is itself deprecated
            symbols.push(SymbolInformation {
                name: key,
                kind: SymbolKind::KEY,
                tags: None,
                deprecated: None,
                location: Location {
                    uri,
                    range: to_lsp_range(&value.key_range),
                },
                container_name: None,
            });
            if symbols.len() >= max_results {
                break;
            }
        }

        info!(
            count = symbols.len(),
            "workspace/symbol responding with {} match(es)",
            symbols.len()
        );
        Ok(Some(symbols))
    }
}

impl Backend {
    /// Recompute diagnostics for a single document (source or locale) and
    /// push them to the client.
    async fn publish_diagnostics(&self, uri: &Url) {
        publish_diagnostics_for(&self.documents, &self.index, &self.usages, &self.client, uri)
            .await;
    }

    /// Refresh the reverse [`UsageIndex`] for `uri` if it points at a
    /// supported source-code file (Vue/TS/JS/...). Returns `true` when the
    /// index was actually modified, so callers can decide whether to
    /// republish the locale-side diagnostics that depend on it.
    async fn maybe_update_usages(&self, uri: &Url, language_id: &str, text: &str) -> bool {
        if !is_supported_source_lang(language_id) {
            return false;
        }
        // Defensive: a Vue file shouldn't ever land in `idx.files`, but
        // if a project is mis-configured we'd rather skip than corrupt
        // the usage index with locale-file content.
        if is_indexed_locale_uri(&self.index, uri).await {
            return false;
        }
        let Ok(path) = uri.to_file_path() else {
            return false;
        };
        self.usages.write().await.update_file(path, text, language_id);
        true
    }

    /// Re-emit unused-key diagnostics for every open locale file. Triggered
    /// after the [`UsageIndex`] changes so the editor's faded-key markers
    /// stay in sync with what the source code actually references.
    async fn republish_locale_diagnostics(&self) {
        let uris: Vec<Url> = self.documents.read().await.keys().cloned().collect();
        for uri in uris {
            if is_indexed_locale_uri(&self.index, &uri).await {
                self.publish_diagnostics(&uri).await;
            }
        }
    }
}

/// Language ids the project-wide source scan understands. Kept in sync with
/// the file-extension matcher in [`i18n_core::scan`].
fn is_supported_source_lang(language_id: &str) -> bool {
    matches!(
        language_id.to_ascii_lowercase().as_str(),
        "vue.js" | "typescript" | "tsx" | "javascript" | "jsx" | "html"
    )
}

/// Free-function version of [`Backend::publish_diagnostics`] used by the
/// watcher rebuild loop and by other helpers that don't carry a `Backend`.
///
/// Dispatches based on whether `uri` is a locale file known to the index:
///
/// - **Locale file** → "unused translation" diagnostics computed from the
///   reverse [`UsageIndex`].
/// - **Anything else** → the existing source-side diagnostics for missing
///   translations of the keys referenced in the document.
async fn publish_diagnostics_for(
    documents: &DocumentStore,
    index: &IndexSlot,
    usages: &UsageSlot,
    client: &Client,
    uri: &Url,
) {
    let Some(doc) = documents.read().await.get(uri).cloned() else {
        return;
    };

    let is_locale = is_indexed_locale_uri(index, uri).await;
    let diagnostics = if is_locale {
        // Locale path always returns a vec (possibly empty) so we can
        // clear stale markers even when the buffer is mid-edit.
        compute_locale_diagnostics(documents, index, usages, uri).await
    } else {
        match compute_source_diagnostics(index, &doc).await {
            Some(d) => d,
            None => return,
        }
    };

    client
        .publish_diagnostics(uri.clone(), diagnostics, Some(doc.version))
        .await;
}

/// Build the source-side missing-key diagnostics for an open document.
/// Returns `None` only if the locale index is not ready yet.
async fn compute_source_diagnostics(
    index: &IndexSlot,
    doc: &DocumentState,
) -> Option<Vec<Diagnostic>> {
    let key_usages = find_usages(&doc.text, &doc.language_id);

    let index_guard = index.read().await;
    let idx = index_guard.as_ref()?;
    let source = &idx.source_locale;
    let out = key_usages
        .into_iter()
        .filter_map(|u| {
            let values = idx.lookup(&u.key);
            if values.is_empty() {
                Some(Diagnostic {
                    range: to_lsp_range(&u.range),
                    severity: Some(DiagnosticSeverity::WARNING),
                    code: Some(tower_lsp::lsp_types::NumberOrString::String(
                        "missing-key".into(),
                    )),
                    source: Some("lokalize".into()),
                    message: format!("Missing translation for key `{}`", u.key),
                    ..Default::default()
                })
            } else if !values.contains_key(source) {
                Some(Diagnostic {
                    range: to_lsp_range(&u.range),
                    severity: Some(DiagnosticSeverity::INFORMATION),
                    code: Some(tower_lsp::lsp_types::NumberOrString::String(
                        "missing-source".into(),
                    )),
                    source: Some("lokalize".into()),
                    message: format!(
                        "Key `{}` is missing from source locale `{}`",
                        u.key, source
                    ),
                    ..Default::default()
                })
            } else {
                None
            }
        })
        .collect();
    Some(out)
}

/// Build "unused translation" diagnostics for the locale file at `uri`.
///
/// Critically, this *re-parses the live buffer* (falling back to disk when
/// the file isn't open) instead of reusing the cached [`LocaleIndex`].
/// Without this, deleting a line — by hand or via the "Remove unused key"
/// quickfix — leaves the HINT decoration glued to the old line until the
/// watcher rebuilds the index from disk on save.
///
/// Always returns a vector (possibly empty) so the LSP can publish it
/// unconditionally and clear stale markers, even on a parse error caused
/// by an in-flight edit.
async fn compute_locale_diagnostics(
    documents: &DocumentStore,
    index: &IndexSlot,
    usages: &UsageSlot,
    uri: &Url,
) -> Vec<Diagnostic> {
    let Ok(path) = uri.to_file_path() else {
        return Vec::new();
    };

    // Live buffer wins over disk so ranges follow the user's edits.
    let source = match documents.read().await.get(uri).map(|d| d.text.clone()) {
        Some(t) => t,
        None => match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return Vec::new(),
        },
    };

    // Mid-typing JSON is often invalid; rather than freezing the previous
    // diagnostic positions, return an empty list so the editor clears.
    let entries = match i18n_core::parser::parse_with_extension(&source, &path) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let usages_guard = usages.read().await;
    // Until the project-wide source scan has at least one file recorded,
    // every key would look unused. Suppress the diagnostic in that
    // transient state to avoid a flash of "all keys unused" on cold start.
    if usages_guard.file_count() == 0 {
        return Vec::new();
    }

    let index_guard = index.read().await;
    let Some(idx) = index_guard.as_ref() else {
        return Vec::new();
    };
    let Some(locale_file) = idx.files.iter().find(|f| f.path == path) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries {
        let key = idx.compose_full_key(locale_file, &entry.key_path);
        if usages_guard.is_key_used(&key) {
            continue;
        }
        out.push(Diagnostic {
            range: to_lsp_range(&entry.key_range),
            severity: Some(DiagnosticSeverity::HINT),
            tags: Some(vec![tower_lsp::lsp_types::DiagnosticTag::UNNECESSARY]),
            code: Some(tower_lsp::lsp_types::NumberOrString::String(
                "unused-key".into(),
            )),
            source: Some("lokalize".into()),
            message: format!(
                "Translation key `{key}` is not referenced by any scanned source file."
            ),
            ..Default::default()
        });
    }
    out
}

async fn is_indexed_locale_uri(index: &IndexSlot, uri: &Url) -> bool {
    let Ok(path) = uri.to_file_path() else {
        return false;
    };
    let guard = index.read().await;
    guard
        .as_ref()
        .map(|idx| idx.files.iter().any(|f| f.path == path))
        .unwrap_or(false)
}

// ---------- Helpers ----------

fn usage_at_position(doc: &DocumentState, pos: LspPosition) -> Option<KeyUsage> {
    let lines = LineIndex::new(&doc.text);
    let offset = lines.offset_at(pos.line, pos.character)?;
    find_usages(&doc.text, &doc.language_id)
        .into_iter()
        .find(|u| u.range.start.offset <= offset && offset < u.range.end.offset)
}

fn to_lsp_position(p: &i18n_core::Position) -> LspPosition {
    LspPosition {
        line: p.line,
        character: p.character,
    }
}

fn to_lsp_range(r: &i18n_core::Range) -> LspRange {
    LspRange {
        start: to_lsp_position(&r.start),
        end: to_lsp_position(&r.end),
    }
}

fn localized_value_to_location(v: &LocalizedValue) -> Option<Location> {
    Url::from_file_path(&v.file).ok().map(|uri| Location {
        uri,
        range: to_lsp_range(&v.range),
    })
}

fn build_inlay_hint(usage: &KeyUsage, value: &LocalizedValue) -> InlayHint {
    let parsed = ParsedValue::parse(&value.value);
    let preview = truncate_chars(parsed.primary_form(), 60);
    // Surface plurality with a `…` hint so users know multiple forms exist.
    let label = if parsed.is_plural() {
        format!(" = {preview} …")
    } else {
        format!(" = {preview}")
    };
    InlayHint {
        position: to_lsp_position(&usage.range.end),
        label: InlayHintLabel::String(label),
        kind: Some(InlayHintKind::PARAMETER),
        text_edits: None,
        tooltip: Some(tower_lsp::lsp_types::InlayHintTooltip::String(
            value.value.clone(),
        )),
        padding_left: Some(true),
        padding_right: Some(false),
        data: None,
    }
}

/// Render the hover popup markdown for a key and all its known translations.
///
/// Structure:
/// 1. Key name as title
/// 2. Source-locale value in a blockquote (pluralised form listed individually)
/// 3. Other locales with compact previews
/// 4. Footer with *clickable* links to every defining file + line (Zed supports
///    `file://…` URIs in markdown links natively — they act as navigation
///    buttons since LSP hover does not permit real interactive controls).
fn format_hover_markdown(
    key: &str,
    values: &std::collections::BTreeMap<&i18n_core::Locale, &LocalizedValue>,
    source_locale: &i18n_core::Locale,
) -> String {
    let mut md = String::with_capacity(384);

    // Title: the key itself.
    md.push_str(&format!("**`{}`**\n\n", key));

    // Prominent source-locale value, rendered as a blockquote.
    if let Some(src) = values.get(source_locale) {
        let parsed = ParsedValue::parse(&src.value);
        if parsed.is_plural() {
            md.push_str(&format!("> **{source_locale}** (pluralised)\n>\n"));
            for (i, form) in parsed.forms.iter().enumerate() {
                md.push_str(&format!(
                    "> - `{}` — {}\n",
                    parsed.form_label(i),
                    escape_md(form),
                ));
            }
        } else {
            md.push_str(&format!(
                "> **{source_locale}** — {}\n",
                escape_md(&src.value),
            ));
        }
        md.push('\n');
    }

    // Other locales (sorted alphabetically for stable output).
    let mut others: Vec<(&i18n_core::Locale, &LocalizedValue)> = values
        .iter()
        .filter(|(locale, _)| locale != &&source_locale)
        .map(|(l, v)| (*l, *v))
        .collect();
    others.sort_by_key(|(l, _)| l.to_string());

    if !others.is_empty() {
        md.push_str("**Other locales**\n\n");
        for (locale, val) in others {
            let parsed = ParsedValue::parse(&val.value);
            let preview = truncate_chars(parsed.primary_form(), 80);
            md.push_str(&format!("- **{locale}** — {}\n", escape_md(&preview)));
        }
        md.push('\n');
    }

    // Footer: clickable links to every locale file. Sorted by locale.
    let mut files: Vec<(&i18n_core::Locale, &LocalizedValue)> =
        values.iter().map(|(l, v)| (*l, *v)).collect();
    files.sort_by_key(|(l, _)| l.to_string());

    md.push_str("---\n**Open translation file:**\n\n");
    for (locale, val) in files {
        let name = val
            .file
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?");
        let line = val.range.start.line + 1;
        match Url::from_file_path(&val.file) {
            Ok(url) => md.push_str(&format!(
                "- **{locale}** — [`{name}:{line}`]({url})\n",
            )),
            Err(_) => md.push_str(&format!("- **{locale}** — `{name}:{line}`\n")),
        }
    }

    md.push_str("\n*Press `F12` to jump to a translation in a picker.*");

    md
}

// ---------- Code-action builders (Phase 4) ----------

/// Build one "Create key in <locale>.json" action per locale missing `key`.
///
/// Strategy:
/// 1. Look at every locale the project knows about. Skip the ones that
///    already have the key.
/// 2. For each missing locale, find the file that mirrors the one where
///    the key is currently defined (same namespace, different locale).
///    If we can't figure out where to insert, skip that locale silently.
/// 3. Apply `insert_key_json` to its current contents and wrap the result
///    in a `WorkspaceEdit` that replaces the whole file.
async fn build_fill_missing_actions(
    idx: &LocaleIndex,
    key: &str,
) -> Vec<CodeActionOrCommand> {
    let values = idx.lookup(key);
    // Pick a reference value (source locale preferred, else any).
    let reference_value: String = values
        .get(&idx.source_locale)
        .copied()
        .or_else(|| values.values().next().copied())
        .map(|v| v.value.clone())
        .unwrap_or_else(|| humanize_key(key));

    // Identify the source `LocaleFile` so we can look up the sibling file in
    // each missing locale. Falls back to a sibling-prefix lookup for keys
    // that don't exist anywhere yet (first-time authoring).
    let source_value = values
        .values()
        .next()
        .copied()
        .or_else(|| find_reference_value(idx, key));
    let source_file: Option<&LocaleFile> =
        source_value.and_then(|v| idx.files.iter().find(|f| f.path == v.file));

    // Every locale present in the project.
    let all_locales: Vec<&i18n_core::Locale> = idx.trees.keys().collect();
    let mut missing: Vec<&i18n_core::Locale> = all_locales
        .iter()
        .copied()
        .filter(|l| !values.contains_key(l))
        .collect();
    missing.sort_by_key(|l| l.to_string());

    let layout = idx.layout.unwrap_or(LocaleLayout::Nested);

    let mut actions = Vec::new();
    for locale in missing {
        let Some(source) = source_file else {
            continue;
        };
        let Some(target) = find_target_file(idx, source, locale) else {
            continue;
        };

        let Ok(content) = std::fs::read_to_string(&target.path) else {
            warn!(path = %target.path.display(), "could not read target locale file");
            continue;
        };

        // Key path relative to the JSON root of the target file.
        let path_segments =
            key_path_in_file(key, target.namespace.as_deref(), layout, idx);
        if path_segments.is_empty() {
            continue;
        }
        let path_refs: Vec<&str> = path_segments.iter().map(|s| s.as_str()).collect();

        let new_content = match insert_key_json(&content, &path_refs, &reference_value) {
            Ok(c) => c,
            Err(e) => {
                warn!(path = %target.path.display(), "insert_key_json failed: {e}");
                continue;
            }
        };

        let Ok(target_uri) = Url::from_file_path(&target.path) else {
            continue;
        };
        let range = whole_file_range(&content);
        let mut changes = HashMap::new();
        changes.insert(
            target_uri,
            vec![TextEdit {
                range,
                new_text: new_content,
            }],
        );

        let filename = target
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?");
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: format!("Lokalize: Create `{key}` in {locale} (`{filename}`)"),
            kind: Some(CodeActionKind::QUICKFIX),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            ..Default::default()
        }));
    }

    actions
}

/// Build code actions for a cursor position inside a locale JSON file.
///
/// Currently the only locale-side action is "Remove unused key": when the
/// cursor sits on a key whose dotted name is absent from the
/// [`UsageIndex`], we propose a [`WorkspaceEdit`] that removes the entry
/// and prunes any namespace that becomes empty as a result.
///
/// The buffer is re-parsed on demand so the cursor-to-key match always
/// uses ranges that line up with what the user currently sees, including
/// unsaved edits.
async fn build_locale_code_actions(
    documents: &DocumentStore,
    index: &IndexSlot,
    usages: &UsageSlot,
    uri: &Url,
    pos: LspPosition,
) -> Option<CodeActionResponse> {
    let path = uri.to_file_path().ok()?;

    let buffer = documents.read().await.get(uri).map(|d| d.text.clone())?;
    let entries = i18n_core::parser::parse_with_extension(&buffer, &path).ok()?;
    let entry = entries
        .into_iter()
        .find(|e| range_contains_position(&e.key_range, pos))?;

    let full_key = {
        let guard = index.read().await;
        let idx = guard.as_ref()?;
        let locale_file = idx.files.iter().find(|f| f.path == path)?;
        idx.compose_full_key(locale_file, &entry.key_path)
    };

    if usages.read().await.is_key_used(&full_key) {
        return None;
    }

    let path_refs: Vec<&str> = entry.key_path.iter().map(String::as_str).collect();
    let new_content = match remove_key_json(&buffer, &path_refs) {
        Ok(c) => c,
        Err(e) => {
            warn!(path = %path.display(), "remove_key_json failed: {e}");
            return None;
        }
    };

    let range = whole_file_range(&buffer);
    let mut changes = HashMap::new();
    changes.insert(
        uri.clone(),
        vec![TextEdit {
            range,
            new_text: new_content,
        }],
    );

    let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
    Some(vec![CodeActionOrCommand::CodeAction(CodeAction {
        title: format!("Lokalize: Remove unused key `{full_key}` from `{filename}`"),
        kind: Some(CodeActionKind::QUICKFIX),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        ..Default::default()
    })])
}

/// True iff `pos` lies inside `range`. Uses the LSP convention of half-open
/// ranges (end is exclusive in LSP, but for cursor-on-token UX we accept
/// equality on either side — the user's caret often lands at the boundary
/// of the property name they meant to interact with).
fn range_contains_position(range: &i18n_core::Range, pos: LspPosition) -> bool {
    let start = (range.start.line, range.start.character);
    let end = (range.end.line, range.end.character);
    let p = (pos.line, pos.character);
    start <= p && p <= end
}

/// Walk progressively shorter prefixes of `key` and return a `LocalizedValue`
/// for any leaf that shares a prefix. Used to locate the target file when
/// the key itself doesn't exist in any locale yet.
///
/// Example: looking up `global.zzzNewKey` will find any existing key under
/// `global.*` (e.g. `global.numberOfDisplayedElements`) and return it so the
/// caller can derive the target file from its path.
fn find_reference_value<'a>(
    idx: &'a LocaleIndex,
    key: &str,
) -> Option<&'a LocalizedValue> {
    let segments: Vec<&str> = key.split('.').collect();
    for n in (1..segments.len()).rev() {
        for tree in idx.trees.values() {
            if let Some(v) = first_leaf_under(tree, &segments[..n]) {
                return Some(v);
            }
        }
    }
    None
}

/// Navigate `tree` down `prefix` and return the first `Leaf` found under the
/// matching subtree (DFS). Returns `None` if the prefix isn't reachable.
fn first_leaf_under<'a>(
    tree: &'a i18n_core::KeyTree,
    prefix: &[&str],
) -> Option<&'a LocalizedValue> {
    let mut current = tree;
    for seg in prefix {
        match current.children.get(*seg)? {
            i18n_core::KeyNode::Branch(sub) => current = sub,
            i18n_core::KeyNode::Leaf(v) => return Some(v),
        }
    }
    first_leaf_in(current)
}

fn first_leaf_in(tree: &i18n_core::KeyTree) -> Option<&LocalizedValue> {
    for node in tree.children.values() {
        match node {
            i18n_core::KeyNode::Leaf(v) => return Some(v),
            i18n_core::KeyNode::Branch(sub) => {
                if let Some(v) = first_leaf_in(sub) {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Find the file in `target_locale` whose namespace matches `source`'s.
/// Falls back to the sole file if the target locale only has one.
fn find_target_file<'a>(
    idx: &'a LocaleIndex,
    source: &LocaleFile,
    target_locale: &i18n_core::Locale,
) -> Option<&'a LocaleFile> {
    if let Some(sibling) = idx
        .files
        .iter()
        .find(|f| f.locale == *target_locale && f.namespace == source.namespace)
    {
        return Some(sibling);
    }
    // Fallback: flat layout without namespaces.
    let mut in_locale = idx.files.iter().filter(|f| f.locale == *target_locale);
    let first = in_locale.next();
    match in_locale.next() {
        Some(_) => None, // ambiguous
        None => first,
    }
}

/// Compute the dot-path of `key` *relative to the JSON root of the target file*.
///
/// Two semantics, controlled by `config.namespace`:
///
/// - **Nested + `namespace: true`** (default, i18n-ally style) — the indexer
///   prepended the filename stem to every key, so the stored path already
///   includes the namespace. The JSON file itself, however, does *not* start
///   with a `{ stem: {...} }` wrapper, so we must strip the first segment
///   before writing.
/// - **Nested + `namespace: false`** — the JSON is self-wrapped
///   (`{"slots": {...}}`) and the indexer stored keys exactly as they appear.
///   No stripping: the full key navigates into the JSON as-is.
/// - **Flat layout** — the JSON holds every top-level key; no stripping.
fn key_path_in_file(
    key: &str,
    file_namespace: Option<&str>,
    layout: LocaleLayout,
    idx: &LocaleIndex,
) -> Vec<String> {
    let segments: Vec<String> = key.split('.').map(str::to_string).collect();
    if layout != LocaleLayout::Nested {
        return segments;
    }
    let Some(ns) = file_namespace else {
        return segments;
    };
    // Strip only when the indexer explicitly prepended the namespace.
    // `config.namespace` is the source of truth here — the tree structure
    // alone cannot distinguish `namespace: true` from `namespace: false`
    // because both end up with `{ns: {...}}` at the top.
    if idx.config.use_file_namespace() && segments.first().map(String::as_str) == Some(ns) {
        return segments.into_iter().skip(1).collect();
    }
    segments
}

fn whole_file_range(content: &str) -> LspRange {
    let lines: Vec<&str> = content.split('\n').collect();
    let last_line_len = lines.last().map(|l| l.chars().count()).unwrap_or(0);
    LspRange {
        start: LspPosition {
            line: 0,
            character: 0,
        },
        end: LspPosition {
            line: (lines.len().saturating_sub(1)) as u32,
            character: last_line_len as u32,
        },
    }
}

/// Turn `some.nested.myKey` into a human-friendly placeholder: `my key`.
fn humanize_key(key: &str) -> String {
    let last = key.rsplit('.').next().unwrap_or(key);
    // camelCase / PascalCase → space-separated.
    let mut out = String::with_capacity(last.len());
    for (i, ch) in last.chars().enumerate() {
        if i > 0 && ch.is_uppercase() {
            out.push(' ');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

/// Extract filesystem paths for every `workspace_folder` advertised by the client,
/// falling back to the deprecated `root_uri`/`root_path` fields if needed.
fn collect_workspace_roots(params: &InitializeParams) -> Vec<PathBuf> {
    if let Some(folders) = &params.workspace_folders {
        return folders
            .iter()
            .filter_map(|f| f.uri.to_file_path().ok())
            .collect();
    }

    #[allow(deprecated)]
    if let Some(uri) = &params.root_uri {
        if let Ok(p) = uri.to_file_path() {
            return vec![p];
        }
    }

    #[allow(deprecated)]
    if let Some(path) = &params.root_path {
        return vec![PathBuf::from(path)];
    }

    Vec::new()
}

/// Build the locale index for each workspace root, storing the first successful
/// one and starting a filesystem watcher on it.
async fn build_indexes_for_roots(
    roots: Vec<PathBuf>,
    index_slot: IndexSlot,
    usage_slot: UsageSlot,
    documents: DocumentStore,
    client: Client,
    watcher_slot: WatcherSlot,
) {
    for root in roots {
        match build_index(&root).await {
            Ok(index) => {
                let summary = index_summary(&index);
                info!(root = %root.display(), "{summary}");
                client
                    .log_message(MessageType::INFO, format!("Lokalize: {summary}"))
                    .await;
                *index_slot.write().await = Some(index);

                // Walk the project for source-code translation usages. Done
                // after the locale index is in place so the first published
                // diagnostic round can already see both sides.
                let scan_root = root.clone();
                let scan_slot = Arc::clone(&usage_slot);
                let scan_index = Arc::clone(&index_slot);
                let scan_documents = Arc::clone(&documents);
                let scan_client = client.clone();
                tokio::spawn(async move {
                    let usages = tokio::task::spawn_blocking(move || {
                        UsageIndex::build_from_project(&scan_root)
                    })
                    .await
                    .unwrap_or_default();
                    let summary = format!(
                        "scanned {} source file(s) for {} key reference(s)",
                        usages.file_count(),
                        usages.total_usages(),
                    );
                    info!("{summary}");
                    scan_client
                        .log_message(MessageType::INFO, format!("Lokalize: {summary}"))
                        .await;
                    *scan_slot.write().await = usages;
                    // Now that the reverse index is populated, push fresh
                    // diagnostics to every open document so unused-key
                    // markers appear without waiting for the next edit.
                    republish_all_diagnostics(
                        &scan_documents,
                        &scan_index,
                        &scan_slot,
                        &scan_client,
                    )
                    .await;
                });

                // Phase 1: stop at the first workspace root that yields an index,
                // and start watching it for hot reload.
                tokio::spawn(start_watcher(
                    root,
                    Arc::clone(&index_slot),
                    Arc::clone(&usage_slot),
                    documents,
                    client,
                    watcher_slot,
                ));
                return;
            }
            Err(e) => {
                warn!(root = %root.display(), "no index built: {e}");
            }
        }
    }

    warn!("no usable workspace root found — no locale index built");
}

/// Build a single index for a single root, blocking filesystem work moved off
/// the tokio reactor.
async fn build_index(root: &Path) -> std::result::Result<LocaleIndex, String> {
    let root = root.to_path_buf();
    match tokio::task::spawn_blocking(move || {
        let config = ProjectConfig::load(&root);
        IndexBuilder::new(&root, &config).build()
    })
    .await
    {
        Ok(Ok(index)) => Ok(index),
        Ok(Err(e)) => Err(e.to_string()),
        Err(e) => Err(format!("indexer task panicked: {e}")),
    }
}

fn index_summary(index: &LocaleIndex) -> String {
    format!(
        "indexed {} locale(s), {} file(s), {} unique key(s)",
        index.trees.len(),
        index.files.len(),
        index.all_keys().len(),
    )
}

// ---------- File watcher ----------

/// Events buffered during this window are coalesced into a single rebuild.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(300);

/// Start a filesystem watcher on every resolved locale directory under `root`.
///
/// The `notify` callback runs on its own OS thread; we bridge into async-land
/// through an mpsc channel. The rebuild loop ends only when the watcher is
/// dropped (i.e. when `Backend` is dropped at LSP shutdown).
async fn start_watcher(
    root: PathBuf,
    index_slot: IndexSlot,
    usage_slot: UsageSlot,
    documents: DocumentStore,
    client: Client,
    watcher_slot: WatcherSlot,
) {
    let config = ProjectConfig::load(&root);
    let locale_dirs = config.resolved_locale_dirs(&root);
    if locale_dirs.is_empty() {
        warn!(root = %root.display(), "watcher: no locale dirs to watch");
        return;
    }

    let (tx, mut rx) = mpsc::channel::<()>(128);
    let cb_tx = tx.clone();
    let result = notify::recommended_watcher(move |res: notify::Result<NotifyEvent>| {
        if let Ok(event) = res {
            if is_locale_event(&event) {
                // `try_send` is fine — on overflow we just drop the event, the
                // debouncer will pick up later notifications anyway.
                let _ = cb_tx.try_send(());
            }
        }
    });
    let mut watcher = match result {
        Ok(w) => w,
        Err(e) => {
            error!("failed to create watcher: {e}");
            return;
        }
    };

    for p in &locale_dirs {
        match watcher.watch(p, RecursiveMode::Recursive) {
            Ok(()) => info!(path = %p.display(), "watching locale dir"),
            Err(e) => warn!(path = %p.display(), "watch failed: {e}"),
        }
    }
    // Store the watcher so its background thread stays alive. Dropping it would
    // close the channel and kill the rebuild loop.
    if let Ok(mut slot) = watcher_slot.lock() {
        *slot = Some(watcher);
    }

    // Drop our extra Sender so the loop exits naturally when the watcher is
    // dropped (only the callback's Sender remains).
    drop(tx);

    while rx.recv().await.is_some() {
        // Debounce: consume every additional event queued during the window.
        tokio::time::sleep(WATCH_DEBOUNCE).await;
        while rx.try_recv().is_ok() {}

        info!(root = %root.display(), "locale change detected, rebuilding index");
        match build_index(&root).await {
            Ok(index) => {
                let summary = index_summary(&index);
                info!(root = %root.display(), "{summary} (reload)");
                client
                    .log_message(
                        MessageType::INFO,
                        format!("Lokalize: {summary} (reload)"),
                    )
                    .await;
                *index_slot.write().await = Some(index);
                republish_all_diagnostics(&documents, &index_slot, &usage_slot, &client).await;
                // Inlay hints follow a pull model: the editor only re-fetches
                // them when we ask it to. Without this, inlay hints keep
                // showing stale translations until the user edits the buffer.
                if let Err(e) = client.inlay_hint_refresh().await {
                    warn!("inlay_hint_refresh failed: {e}");
                }
            }
            Err(e) => warn!("index rebuild failed: {e}"),
        }
    }

    info!("watcher loop ended");
}

fn is_locale_event(event: &NotifyEvent) -> bool {
    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) && event.paths.iter().any(|p| is_locale_file(p))
}

fn is_locale_file(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e, "json" | "jsonc" | "json5" | "arb" | "yml" | "yaml"))
        .unwrap_or(false)
}

async fn republish_all_diagnostics(
    documents: &DocumentStore,
    index: &IndexSlot,
    usages: &UsageSlot,
    client: &Client,
) {
    let uris: Vec<Url> = documents.read().await.keys().cloned().collect();
    for uri in uris {
        publish_diagnostics_for(documents, index, usages, client, &uri).await;
    }
}

fn init_tracing() {
    use std::sync::Mutex;

    let filter = EnvFilter::try_from_env("LOKALIZE_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    // Also log to a file we can inspect outside Zed. Zed captures LSP stdout
    // (JSON-RPC) and ignores stderr, so a dedicated file is the most reliable
    // way to observe the server's internal state.
    let log_path = std::env::var("LOKALIZE_LOG_FILE")
        .unwrap_or_else(|_| "/tmp/lokalize-lsp.log".to_string());

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path);

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(false);

    match file {
        Ok(f) => builder.with_writer(Mutex::new(f)).init(),
        Err(_) => builder.with_writer(std::io::stderr).init(),
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    init_tracing();
    info!(
        pid = std::process::id(),
        "starting {} v{} (i18n-core v{})",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        i18n_core::VERSION,
    );

    let (service, socket) = LspService::new(|client| Backend {
        client,
        index: Arc::new(RwLock::new(None)),
        usages: Arc::new(RwLock::new(UsageIndex::new())),
        documents: Arc::new(RwLock::new(HashMap::new())),
        _watcher: Arc::new(StdMutex::new(None)),
    });
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    info!("server listening on stdio");
    Server::new(stdin, stdout, socket).serve(service).await;
    info!("server shutdown");
}
