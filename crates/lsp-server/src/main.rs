//! `lokalize-lsp` — Language server binary. Phase 0 handles only the LSP lifecycle
//! (initialize / initialized / shutdown) so the extension is installable end-to-end
//! before richer features land in Phase 1+.

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    InitializeParams, InitializeResult, InitializedParams, MessageType, ServerCapabilities,
    ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug)]
struct Backend {
    client: Client,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
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
        info!("lokalize-lsp initialized");
        self.client
            .log_message(MessageType::INFO, "Lokalize LSP ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_env("LOKALIZE_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    init_tracing();
    info!(
        "starting {} v{} (i18n-core v{})",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        i18n_core::VERSION,
    );

    let (service, socket) = LspService::new(|client| Backend { client });
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    Server::new(stdin, stdout, socket).serve(service).await;
}
