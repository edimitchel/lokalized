//! `lokalize-lsp` — Language server binary.
//!
//! Phase 0 handled the LSP lifecycle. Phase 1 adds locale indexing: when a workspace
//! opens, the server discovers locale files, parses them and keeps an in-memory
//! `LocaleIndex` ready for hover / inlay-hint / go-to-definition (Phase 3).

use std::path::PathBuf;
use std::sync::Arc;

use i18n_core::{IndexBuilder, LocaleIndex, ProjectConfig};
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    InitializeParams, InitializeResult, InitializedParams, MessageType, ServerCapabilities,
    ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug)]
struct Backend {
    client: Client,
    /// The locale index. `None` until the first workspace folder is indexed.
    /// Populated asynchronously in `initialized` so the handshake stays fast.
    index: Arc<RwLock<Option<LocaleIndex>>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        info!("-> initialize request received");
        let roots = collect_workspace_roots(&params);
        info!(?roots, "initialize: {} workspace root(s)", roots.len());

        // Index building happens after `initialized`, not here, so the handshake
        // returns immediately.
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
    });
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    info!("server listening on stdio");
    Server::new(stdin, stdout, socket).serve(service).await;
    info!("server shutdown");
}
