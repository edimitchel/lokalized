# Lokalized — Zed extension for i18n

All-in-one internationalization support for Zed: inline translations, hover previews,
go-to-definition, diagnostics, code actions, and an MCP server for the Assistant.

Inspired by [i18n-ally](https://github.com/lokalise/i18n-ally) (VSCode).

## Status

Alpha — LSP (hover, inlay hints, go-to-def, references, completion, diagnostics, code
actions) for Vue / TypeScript / JavaScript with `vue-i18n` / `i18next` and JSON/YAML
locale files. MCP tools for the Zed Agent. See [`PLAN.md`](PLAN.md) for the roadmap.

## Quick start

### 1. Install the extension

**From the Zed catalog** (once published): Extensions → search **Lokalized** → Install.

**Dev extension** (this repo):

1. Install [Rust via rustup](https://www.rust-lang.org/tools/install) (Homebrew Rust does not work with Zed dev extensions).
2. Zed → **Extensions** → **Install Dev Extension** → select this directory.
3. Reinstall after each `extension.wasm` rebuild.

### 2. Install native binaries

Until [GitHub Releases](https://github.com/edimitchel/lokalized/releases) ship platform
assets, build locally:

```bash
cargo build -p lokalized-lsp -p lokalized-mcp --release
cp target/release/lokalized-lsp target/release/lokalized-mcp ~/.cargo/bin/
```

After the first release, the extension downloads `lokalized-lsp` and `lokalized-mcp`
automatically (no paths in settings).

### 3. Open the right folder in Zed

Open the **workspace root** (e.g. monorepo root), not only `front/`. Locale folders
such as `front/i18n/locales` are discovered automatically.

### 4. Enable the MCP server (optional)

In `~/.config/zed/settings.json` (or project `.zed/settings.json`):

```json
"context_servers": {
  "lokalized": {
    "enabled": true,
    "settings": {}
  }
}
```

Also enable **Lokalized** under **Agent → Settings**. The key must be `lokalized`
(see `extension.toml`). Do **not** set `command` / `args` alone — Zed would expect a
custom stdio server.

---

## Configuration

### Zed user settings (`settings.json`)

| Setting | Purpose |
|--------|---------|
| `context_servers.lokalized` | Enable the MCP context server (extension-managed binary). |
| `languages.*.language_servers` | Optionally prioritize `"lokalized"` among servers for Vue/TS/JS. |
| `lsp.lokalized.binary.env` | Override env vars passed to `lokalized-lsp` (see below). |

Example — force a local LSP binary (dev without `~/.cargo/bin`):

```json
"lsp": {
  "lokalized": {
    "binary": {
      "path": "/absolute/path/to/lokalize-vue/target/release/lokalized-lsp",
      "env": {
        "LOKALIZED_LOG": "debug"
      }
    }
  }
}
```

Example — prefer Lokalized for Vue:

```json
"languages": {
  "Vue.js": {
    "language_servers": ["lokalized", "..."]
  }
}
```

### Project config (`.zed/lokalized.json`)

Placed at the **workspace root**. All fields are optional; missing values use
filesystem auto-detection. Legacy filename: `.zed/lokalize.json`.

```json
{
  "localePaths": ["front/i18n/locales"],
  "sourceLocale": "fr",
  "namespace": true,
  "keyStyle": "nested",
  "enabledFrameworks": []
}
```

| Field | Description |
|-------|-------------|
| `localePaths` | Directories (relative to workspace root) containing locale files. If empty, auto-detection runs. |
| `sourceLocale` | Reference locale for “missing key” checks (default: `en`). Use the locale you author in (e.g. `fr` if you have no `en`). |
| `namespace` | If `true` (default), keys in `en/common.json` are indexed as `common.*` when the JSON is flat (`{ "actions": … }`). Set `false` when each file already wraps content (`{ "slots": { … } }` in `slots.json` → keys stay `slots.*`). Self-wrapped files are detected automatically and never double-prefixed. |
| `keyStyle` | `nested`, `flat`, or `auto`. |
| `enabledFrameworks` | Framework ids to enable; empty = auto-detect (`vue-i18n`, `i18next`, etc.). |

**Monorepo / Nuxt example** (e.g. MG_SHOP with `front/i18n/locales/fr/*.json`):

```json
{
  "localePaths": ["front/i18n/locales"],
  "sourceLocale": "fr",
  "namespace": false
}
```

Point at the `locales` root, not `locales/fr`. Each file is a namespace (`global.json` → keys `global.*`). Use `"namespace": false` when the JSON already has a top-level `"global": { … }` wrapper (typical for `@nuxtjs/i18n` split files).

### Auto-detected locale directories

Without config, Lokalized scans common paths, including:

- `locales`, `src/locales`, `i18n/locales`, `public/locales`, …
- Monorepo: `front/i18n/locales`, `frontend/locales`, `client/i18n/locales`, `apps/web/locales`, …

It also walks the tree (depth 8, skipping `node_modules`, `dist`, `target`, `.git`, etc.) for folders named `locales` or `l10n` that contain translation files.

### Environment variables

| Variable | Used by | Description |
|----------|---------|-------------|
| `LOKALIZED_LSP_PATH` | Extension / LSP | Absolute path to `lokalized-lsp` (skips download). Legacy: `LOKALIZE_LSP_PATH`. |
| `LOKALIZED_EXTENSION_ROOT` | Extension | Path to this repo (dev builds in `target/release`). Legacy: `LOKALIZE_EXTENSION_ROOT`. |
| `LOKALIZED_WORKSPACE` | MCP | Workspace root (set automatically by the extension). Legacy: `LOKALIZE_WORKSPACE`. |
| `LOKALIZED_LOG` | LSP | Log level, e.g. `debug`. Legacy: `LOKALIZE_LOG`. |
| `LOKALIZED_LOG_FILE` | LSP | Log file path (default: `/tmp/lokalized-lsp.log`). Legacy: `LOKALIZE_LOG_FILE`. |
| `LOKALIZED_DEEPL_KEY` | MCP | DeepL API key (translation tool, not implemented yet). Legacy: `LOKALIZE_DEEPL_KEY`. |

### How binaries are resolved

For `lokalized-lsp` and `lokalized-mcp`, the extension tries in order:

1. `LOKALIZED_LSP_PATH` / shell env / `which lokalized-lsp`
2. `~/.cargo/bin/`
3. `lokalized-mcp` next to `lokalized-lsp` on PATH
4. `target/release` or `target/debug` in the extension repo (`LOKALIZED_EXTENSION_ROOT` or dev extension cwd)
5. `cargo build -p <binary> --release` in the extension repo
6. Previously downloaded copy in the extension cache directory
7. GitHub Release asset for your OS/arch (`lokalized-lsp-<triple>`, `lokalized-mcp-<triple>`)

### Languages & features (LSP)

Registered for: Vue.js, TypeScript, TSX, JavaScript, JSX, HTML, JSON, JSONC, JSON5, YAML.

- Inlay hints, hover, go-to-definition, references, completion
- Diagnostics (missing keys, etc.)
- Code actions (create key, remove unused key)

Enable inlay hints in Zed if needed:

```json
"inlay_hints": {
  "enabled": true,
  "show_other_hints": true
}
```

### MCP tools (Agent)

| Tool | Description |
|------|-------------|
| `i18n.list_keys` | List keys (optional locale, prefix filter). |
| `i18n.get_value` | Read a key in a locale. |
| `i18n.set_value` | Write a value (JSON locales). |
| `i18n.find_missing` | Keys in source locale missing from a target. |
| `i18n.extract` | Propose a key from source text (planned behaviour). |
| `i18n.translate_key` | DeepL/OpenAI (requires API key; not implemented yet). |
| `i18n.refresh_index` | Reload locale index from disk. |

The MCP server receives the open project root as its first argument (and via `LOKALIZED_WORKSPACE`).

---

## Troubleshooting

### `failed to fetch GitHub release for edimitchel/lokalized`

No release is published yet. Build and install binaries (see [Quick start](#2-install-native-binaries)), then reinstall the dev extension and restart Zed.

### No inlay hints / empty index

- Open the **monorepo root**, not a subfolder only.
- Add `.zed/lokalized.json` with `localePaths` and `sourceLocale` if you have no `en` locale.
- Check LSP log: `LOKALIZED_LOG=debug`, file default `/tmp/lokalized-lsp.log`.
- Zed log: command palette → `zed: open log`.

### MCP “server is stopped”

- Enable `context_servers.lokalized` in settings (no manual `command`).
- Ensure `lokalized-mcp` is on PATH or next to `lokalized-lsp`.
- Open the workspace at the project root so locales are indexed.

### Migration from `lokalize` / `lokalize-vue`

| Old | New |
|-----|-----|
| Extension id `lokalize` | `lokalized` |
| `.zed/lokalize.json` | `.zed/lokalized.json` (old name still read) |
| `LOKALIZE_*` env vars | `LOKALIZED_*` (old names still honored) |
| Binaries `lokalize-lsp` / `lokalize-mcp` | `lokalized-lsp` / `lokalized-mcp` |

---

## Architecture

| Crate | Target | Role |
|-------|--------|------|
| `lokalized` (root) | `wasm32-wasip2` | Zed extension — locates/downloads and launches LSP + MCP |
| `lokalized-lsp` | native binary | Language server |
| `i18n-core` | native lib | Locale index, parsers, framework detection |
| `lokalized-mcp` | native binary | MCP server for the Zed Assistant |

## Development

```bash
# WASM extension (required after changing src/)
cargo build -p lokalized --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/lokalized.wasm extension.wasm

# Native binaries
cargo build -p lokalized-lsp -p lokalized-mcp --release

# Tests & lint
cargo test  --workspace --exclude lokalized
cargo clippy --workspace --exclude lokalized
```

Reinstall the dev extension in Zed after rebuilding `extension.wasm`.

## License

MIT — see [`LICENSE`](LICENSE).