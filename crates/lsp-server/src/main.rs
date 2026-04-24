//! `lokalize-lsp` — Language server binary.
//!
//! - **Phase 0**: lifecycle (initialize/initialized/shutdown).
//! - **Phase 1**: workspace indexing — discovers locale files, parses them, keeps an
//!   in-memory `LocaleIndex`.
//! - **Phase 3** (current): hover, inlay hints, go-to-definition, completion and
//!   diagnostics for translation-key usages in source files.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use i18n_core::{
    find_usages, IndexBuilder, KeyUsage, LineIndex, LocaleIndex, LocalizedValue, ProjectConfig,
};
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, GotoDefinitionParams, GotoDefinitionResponse, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
    InitializedParams, InlayHint, InlayHintKind, InlayHintLabel, InlayHintParams, Location,
    MarkupContent, MarkupKind, MessageType, OneOf, Position as LspPosition, Range as LspRange,
    ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
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

struct Backend {
    client: Client,
    /// Locale index. `None` until the first workspace folder is indexed.
    index: Arc<RwLock<Option<LocaleIndex>>>,
    /// Text + language id of every currently-open document (from LSP notifications).
    documents: DocumentStore,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        info!("-> initialize request received");
        let roots = collect_workspace_roots(&params);
        info!(?roots, "initialize: {} workspace root(s)", roots.len());

        // Index building happens after `initialized` so the handshake stays fast.
        let index_slot = Arc::clone(&self.index);
        let client = self.client.clone();
        tokio::spawn(async move {
            build_indexes_for_roots(roots, index_slot, client).await;
        });

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                definition_provider: Some(OneOf::Left(true)),
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
                text: doc.text,
                language_id: doc.language_id,
                version: doc.version,
            },
        );
        self.publish_diagnostics(&doc.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        if let Some(change) = params.content_changes.into_iter().next() {
            let mut store = self.documents.write().await;
            if let Some(doc) = store.get_mut(&uri) {
                doc.text = change.text;
                doc.version = params.text_document.version;
            }
        }
        self.publish_diagnostics(&uri).await;
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

        let mut md = format!("**`{}`**\n\n", usage.key);
        for (locale, val) in &values {
            md.push_str(&format!("- **{locale}** — {}\n", escape_md(&val.value)));
        }

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
                    .map(|v| truncate(&v.value, 60));
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
}

impl Backend {
    /// Recompute diagnostics (missing keys) for a single document and push them.
    async fn publish_diagnostics(&self, uri: &Url) {
        let Some(doc) = self.documents.read().await.get(uri).cloned() else {
            return;
        };
        let usages = find_usages(&doc.text, &doc.language_id);

        let diagnostics = {
            let index_guard = self.index.read().await;
            let Some(idx) = &*index_guard else {
                return;
            };
            let source = &idx.source_locale;
            usages
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
                .collect::<Vec<_>>()
        };

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, Some(doc.version))
            .await;
    }
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
    InlayHint {
        position: to_lsp_position(&usage.range.end),
        label: InlayHintLabel::String(format!(" = {}", truncate(&value.value, 40))),
        kind: Some(InlayHintKind::PARAMETER),
        text_edits: None,
        tooltip: None,
        padding_left: Some(true),
        padding_right: Some(false),
        data: None,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let prefix: String = s.chars().take(max).collect();
        format!("{prefix}…")
    }
}

fn escape_md(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace('*', "\\*")
        .replace('_', "\\_")
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

/// Build the locale index for each workspace root, storing the first successful one.
async fn build_indexes_for_roots(
    roots: Vec<PathBuf>,
    index_slot: Arc<RwLock<Option<LocaleIndex>>>,
    client: Client,
) {
    for root in roots {
        let root_for_log = root.clone();
        let result = tokio::task::spawn_blocking(move || {
            let config = ProjectConfig::load(&root);
            IndexBuilder::new(&root, &config).build()
        })
        .await;

        match result {
            Ok(Ok(index)) => {
                let summary = format!(
                    "indexed {} locale(s), {} file(s), {} unique key(s)",
                    index.trees.len(),
                    index.files.len(),
                    index.all_keys().len(),
                );
                info!(root = %root_for_log.display(), "{summary}");
                client
                    .log_message(MessageType::INFO, format!("Lokalize: {summary}"))
                    .await;
                *index_slot.write().await = Some(index);
                // Phase 1: stop at the first workspace root that yields an index.
                return;
            }
            Ok(Err(e)) => {
                warn!(root = %root_for_log.display(), "no index built: {e}");
            }
            Err(e) => {
                error!(root = %root_for_log.display(), "indexer task panicked: {e}");
            }
        }
    }

    warn!("no usable workspace root found — no locale index built");
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
        documents: Arc::new(RwLock::new(HashMap::new())),
    });
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    info!("server listening on stdio");
    Server::new(stdin, stdout, socket).serve(service).await;
    info!("server shutdown");
}
