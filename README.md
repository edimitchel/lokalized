# LokaliZed — Zed extension for i18n

All-in-one internationalization support for Zed: inline translations, hover previews,
go-to-definition, diagnostics, and more.

Inspired by [i18n-ally](https://github.com/lokalise/i18n-ally) (VSCode).

## Status

Early alpha (Phase 0 scaffold). See [`PLAN.md`](PLAN.md) for the roadmap.

## Architecture

Four crates split across a single Cargo workspace:

| Crate | Target | Role |
|---|---|---|
| `lokalize` (root) | `wasm32-wasip2` | Thin WASM extension — locates and launches the LSP binary |
| `lokalize-lsp` | native binary | Language server: hover, inlay hints, go-to-def, diagnostics, completion |
| `i18n-core` | native lib | Pure logic: locale parsing, key index, framework detection |
| `lokalize-mcp` | native binary | MCP server for the Zed Assistant (planned, v0.3+) |

## Development

Requires [Rust installed via rustup](https://www.rust-lang.org/tools/install) — Homebrew
Rust will not work with Zed dev extensions.

```bash
# 1. Build the language server binary
cargo build -p lokalize-lsp --release

# 2. Make it discoverable
export LOKALIZE_LSP_PATH="$(pwd)/target/release/lokalize-lsp"
# or symlink it into a PATH directory:
# ln -sf "$(pwd)/target/release/lokalize-lsp" /usr/local/bin/lokalize-lsp

# 3. In Zed: open the Extensions page → "Install Dev Extension" → select this directory
```

### Useful commands

```bash
cargo test  --workspace --exclude lokalize        # native tests
cargo clippy --workspace --exclude lokalize       # native lint
cargo build -p lokalize --target wasm32-wasip2    # WASM extension build
```

### Logs

Set `LOKALIZE_LOG=debug` to get verbose language server logs (written to stderr,
captured by Zed in the editor log).

## License

MIT — see [`LICENSE`](LICENSE).
