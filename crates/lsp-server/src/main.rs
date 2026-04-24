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
    find_usages, IndexBuilder, KeyUsage, LineIndex, LocaleIndex, LocalizedValue, ProjectConfig,
};
use notify::{Event as NotifyEvent, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{mpsc, RwLock};
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
type IndexSlot = Arc<RwLock<Option<LocaleIndex>>>;
type WatcherSlot = Arc<StdMutex<Option<RecommendedWatcher>>>;

struct Backend {
    client: Client,
    /// Locale index. `None` until the first workspace folder is indexed.
    index: IndexSlot,
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
        let documents = Arc::clone(&self.documents);
        let watcher_slot = Arc::clone(&self._watcher);
        let client = self.client.clone();
        tokio::spawn(async move {
            build_indexes_for_roots(roots, index_slot, documents, client, watcher_slot).await;
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
        publish_diagnostics_for(&self.documents, &self.index, &self.client, uri).await;
    }
}

/// Free-function version of `publish_diagnostics` used both by the `Backend`
/// handler and by the file-watcher rebuild loop.
async fn publish_diagnostics_for(
    documents: &DocumentStore,
    index: &IndexSlot,
    client: &Client,
    uri: &Url,
) {
    let Some(doc) = documents.read().await.get(uri).cloned() else {
        return;
    };
    let usages = find_usages(&doc.text, &doc.language_id);

    let diagnostics = {
        let index_guard = index.read().await;
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

    client
        .publish_diagnostics(uri.clone(), diagnostics, Some(doc.version))
        .await;
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

/// Build the locale index for each workspace root, storing the first successful
/// one and starting a filesystem watcher on it.
async fn build_indexes_for_roots(
    roots: Vec<PathBuf>,
    index_slot: IndexSlot,
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
                // Phase 1: stop at the first workspace root that yields an index,
                // and start watching it for hot reload.
                tokio::spawn(start_watcher(
                    root,
                    Arc::clone(&index_slot),
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
                republish_all_diagnostics(&documents, &index_slot, &client).await;
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
    client: &Client,
) {
    let uris: Vec<Url> = documents.read().await.keys().cloned().collect();
    for uri in uris {
        publish_diagnostics_for(documents, index, client, &uri).await;
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
        documents: Arc::new(RwLock::new(HashMap::new())),
        _watcher: Arc::new(StdMutex::new(None)),
    });
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    info!("server listening on stdio");
    Server::new(stdin, stdout, socket).serve(service).await;
    info!("server shutdown");
}
