//! `lokalized-mcp` — MCP server for the Zed Assistant (stdio transport).
//!
//! Workspace root resolution (first match wins):
//! 1. `LOKALIZED_WORKSPACE` (legacy: `LOKALIZE_WORKSPACE`)
//! 2. First CLI argument
//! 3. Current working directory

mod server;

use std::path::PathBuf;

use rmcp::transport::stdio;
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

use server::LokalizedMcp;

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
}

fn resolve_workspace() -> PathBuf {
    if let Some(root) = env_nonempty("LOKALIZED_WORKSPACE").or_else(|| env_nonempty("LOKALIZE_WORKSPACE")) {
        return PathBuf::from(root);
    }
    let mut args = std::env::args().skip(1);
    if let Some(arg) = args.next() {
        return PathBuf::from(arg);
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("lokalized_mcp=info".parse()?))
        .with_writer(std::io::stderr)
        .init();

    let workspace = resolve_workspace();
    tracing::info!(root = %workspace.display(), "lokalized-mcp starting");

    let service = LokalizedMcp::new(workspace)?;
    let running = service.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
