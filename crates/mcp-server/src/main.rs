//! `lokalize-mcp` — MCP server stub. Full implementation planned for v0.3.0
//! (see PLAN.md Phase 5). This stub exists only to reserve the crate layout.

fn main() {
    eprintln!(
        "lokalize-mcp v{} is a stub — MCP tools are planned for v0.3.0 (see PLAN.md Phase 5). \
         i18n-core v{} linked OK.",
        env!("CARGO_PKG_VERSION"),
        i18n_core::VERSION,
    );
}
