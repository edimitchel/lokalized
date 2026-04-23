# Plan — Extension Zed façon i18n-ally

Réplique des fonctionnalités de [lokalise/i18n-ally](https://github.com/lokalise/i18n-ally)
sous forme d'extension Zed, avec positionnement **Vue/Nuxt-first** puis élargissement
multi-framework.

---

## 0. Décisions structurantes (validées)

- [x] **Nouvelle extension** (pas un fork de `intl-lens`)
- [x] **Format de config framework custom** : `.zed/i18n-ally-custom-framework.yml`
      à l'identique d'i18n-ally (+ alias `.zed/lokalize.yml`)
- [x] **MCP + slash commands** : crate scaffoldée dès v0.1, features IA implémentées en v0.3
- [x] **MVP étroit** : Vue + TS/JS, frameworks `vue-i18n` + `i18next`, formats JSON + YAML,
      features LSP = hover / inlay hints / go-to-def / completion / diagnostics

---

## 1. Architecture cible

```
lokalize-vue/
├── extension.toml            # métadonnées Zed
├── Cargo.toml                # workspace
├── crates/
│   ├── zed-extension/        # WASM (cdylib) — installe + lance le LSP
│   ├── lsp-server/           # binaire LSP (tower-lsp + tokio)
│   ├── i18n-core/            # lib pure : parsing, index, détection, refactor
│   └── mcp-server/           # binaire MCP (rmcp) — features IA
├── assets/
│   └── frameworks/           # YAML importés d'i18n-ally (MIT)
└── .github/workflows/        # CI cross-build + release
```

**Principes**
- `i18n-core` = logique pure, pas de dépendance LSP ni réseau ; 100% testable.
- `zed-extension` = ~150 lignes, délègue tout au LSP.
- Binaire LSP téléchargé depuis GitHub Releases selon `(os, arch)` via `latest_github_release` + `download_file`.
- Clés API (DeepL/OpenAI) jamais dans le LSP ni le WASM → uniquement dans le process MCP.

---

## Phase 0 — Scaffolding (≈ 1-2 j)

- [x] `extension.toml` (schema_version 1, id `lokalize`, language_server rattaché)
- [x] Workspace `Cargo.toml` (resolver 2, 4 membres, `default-members = ["."]`)
- [x] WASM extension au root : `crate-type = ["cdylib"]`, `zed_extension_api = "0.7"`,
      impl de `Extension` avec `language_server_command` (résolution `LOKALIZE_LSP_PATH` → `which`)
- [x] `crates/i18n-core` : lib avec modules `parser`, `index`, `framework`, `locale`
- [x] `crates/lsp-server` : tower-lsp répondant à `initialize/initialized/shutdown`, tracing `LOKALIZE_LOG`
- [x] `crates/mcp-server` : binaire stub (implémentation Phase 5)
- [x] CI GitHub Actions : `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` + build WASM
- [x] Workflow `release.yml` déclenché par tag `v*`, matrix 5 cibles (linux x64/arm64, macos x64/arm64, windows x64)
- [x] README initial (install dev, architecture, commandes utiles)
- [x] Scaffold buildable : `cargo check` natif ✅, `cargo build --target wasm32-wasip2` ✅ (lokalize.wasm 166 KB)
- [x] Premier commit installable via `zed: install dev extension` (✅ extension chargée dans Zed)

---

## Phase 1 — Parsing & indexation des locales (≈ 3-5 j)

### Détection du projet

- [ ] Scan du worktree à l'ouverture, lecture `.zed/lokalize.json`
- [ ] Heuristiques fallback : `locales/`, `src/locales/`, `i18n/`, `public/locales/`, `lib/l10n/`
- [ ] Détection structure : **flat** (`en.json`) vs **nested** (`en/common.json`)
- [ ] Détection de la `sourceLocale` (défaut `en`, override par config)

### Parsers avec positions source

- [ ] JSON / JSON5 (crate `jsonc-parser` ou équivalent avec spans)
- [ ] YAML (crate `yaml-rust2` ou `serde_yaml` + extraction positions)
- [ ] ARB (Flutter) — JSON + métadonnées `@key`
- [ ] PHP arrays — parser regex/AST minimaliste
- [ ] Parsers différés v0.2+ : PO/gettext, TOML, INI, Properties, Strings, XLIFF

### Index

- [ ] `LocaleIndex` : `HashMap<Locale, KeyTree>` avec feuilles `{ value, file, range }`
- [ ] Support clés à plat (`a.b.c`) et imbriquées
- [ ] Support `linked messages` vue-i18n (`@:other.key`)
- [ ] Index secondaire : `HashMap<Key, Vec<Location>>` pour go-to-def rapide

### Watcher

- [ ] `notify` crate, debounce 100 ms
- [ ] Réindexation incrémentale par fichier modifié
- [ ] Invalidation propre du cache

### Tests

- [ ] Fixtures multi-framework (vue-i18n nested, i18next flat, Flutter ARB)
- [ ] Tests unitaires parsers (valid + invalid + positions)
- [ ] Test d'intégration : ouverture d'un projet type → index attendu

---

## Phase 2 — Détection des usages dans le code (≈ 2-3 j)

- [ ] Port des définitions de frameworks d'i18n-ally (YAML → `include_str!`)
- [ ] Frameworks built-in v0.1 : `vue-i18n`, `i18next`, `react-intl`, `nuxt-i18n`
- [ ] `FrameworkRegistry` chargeant built-in + custom `.zed/i18n-ally-custom-framework.yml`
- [ ] `KeyFinder` : match `usage_match_regex` sur le source, capture group 1 = clé
- [ ] Résolution du `scope_range_regex` (namespace inféré depuis `useTranslation("ns")`)
- [ ] `Vec<KeyUsage { key, range, scope, framework }>` par document
- [ ] **Option robustesse** : tree-sitter pour Vue SFC et TSX (limiter faux positifs
      dans commentaires/strings multi-lignes)
- [ ] Tests : fixtures de fichiers `.vue`, `.tsx`, `.php`, `.dart` avec usages attendus

---

## Phase 3 — Fonctions LSP utilisateur (≈ 4-6 j)

### Capabilities dans `initialize`

- [ ] hoverProvider
- [ ] inlayHintProvider
- [ ] definitionProvider (avec `LocationLink[]`)
- [ ] referencesProvider
- [ ] completionProvider (trigger chars : `"`, `'`, `` ` ``, `.`)
- [ ] codeActionProvider
- [ ] renameProvider (avec prepare)
- [ ] workspaceSymbolProvider
- [ ] documentSymbolProvider

### Handlers

- [ ] **Hover** : markdown listant toutes les locales + liens `file://`
- [ ] **Inlay Hint** : texte de la traduction source après `t(…)`, label cliquable
- [ ] **Definition** : liste de `LocationLink` vers chaque fichier de locale
- [ ] **Completion** : items filtrés par scope, doc = traduction source
- [ ] **Diagnostics** (sur `didOpen` / `didChange` / watcher) :
  - [ ] clé utilisée mais absente de la source locale
  - [ ] clé utilisée mais absente d'une locale cible
  - [ ] valeur vide dans une locale
  - [ ] clé inutilisée (scan global, diagnostic sur le fichier de locale)
- [ ] **Workspace Symbol** : fuzzy match sur toutes les clés

### Config workspace

- [ ] Réception de `workspace/configuration` avec :
      `locale_paths`, `source_locale`, `enabled_frameworks`, `diagnostics`, `key_style`
- [ ] Rechargement à chaud sur `workspace/didChangeConfiguration`

### Perf

- [ ] Index partagé `Arc<RwLock<Index>>`
- [ ] Parsing des locales en parallèle avec `rayon`
- [ ] Cache disque dans `$XDG_CACHE_HOME/lokalize/<hash>.bin` (bincode)

---

## Phase 4 — Refactoring & code actions (≈ 2-3 j)

- [ ] **Extract to i18n key** : string littérale → `$t("auto.key")` + ajout dans toutes les locales
- [ ] Génération automatique de nom de clé (slugify + chemin du fichier)
- [ ] Utilisation des `refactor_templates` du framework détecté
- [ ] **Fill missing translation** : remplit une clé manquante (valeur source ou TODO)
- [ ] **Rename key** (via `textDocument/rename`) : WorkspaceEdit couvrant
      code source + tous les fichiers de locale
- [ ] **Open in editor (Assistant)** : code action qui émet un lien `zed://` vers un
      slash command `/i18n-edit <key>` (pont vers phase 5)
- [ ] Tests : fixtures avant/après pour chaque action

---

## Phase 5 — Intégration Assistant (MCP + slash commands) (≈ 3-4 j)

### Slash commands (dans `zed-extension`)

- [ ] `/i18n-missing` : liste des clés manquantes par locale
- [ ] `/i18n-extract <text>` : suggestion de noms de clés
- [ ] `/i18n-translate <key> <lang>` : traduction via engine configuré
- [ ] `/i18n-stats` : progression par locale
- [ ] `complete_slash_command_argument` pour autocomplétion des clés existantes

### MCP server (`mcp-server`)

- [ ] Outil `i18n.list_keys(locale?, prefix?)`
- [ ] Outil `i18n.get_value(key, locale)`
- [ ] Outil `i18n.set_value(key, locale, value)` (écrit le fichier)
- [ ] Outil `i18n.find_missing(locale)`
- [ ] Outil `i18n.translate_key(key, target_locale, engine)` — appel DeepL/OpenAI
- [ ] Outil `i18n.extract(text, file_context)` — suggestion + création
- [ ] Config des engines via env (`LOKALIZE_DEEPL_KEY`, `OPENAI_API_KEY`)
      ou section `ai` de `.zed/lokalize.json`
- [ ] Déclaration dans `extension.toml` + `context_server_command`

---

## Phase 6 — Qualité, perf, robustesse (continu)

- [ ] `cargo clippy -D warnings` + `cargo fmt --check` en CI
- [ ] Couverture via `cargo-llvm-cov` ≥ 80% sur `i18n-core`
- [ ] Fuzzing `cargo-fuzz` sur parsers JSON/YAML/PHP
- [ ] Tests d'intégration LSP (jsonrpc over stdio, fixtures snapshot)
- [ ] Bench : projet 10k clés doit s'indexer < 500 ms à froid, < 50 ms à chaud
- [ ] Logs structurés `tracing` écrits dans fichier, pas de stdout pollué
- [ ] Documentation `docs/` : architecture, contribution, custom framework
- [ ] Changelog `CHANGELOG.md` (Keep a Changelog)

---

## Phase 7 — Expansion frameworks & formats (itératif)

### Frameworks (ordre de priorité)

- [ ] vue-i18n (v0.1)
- [ ] i18next + react-i18next (v0.1)
- [ ] nuxt-i18n (v0.1)
- [ ] react-intl / FormatJS (v0.2)
- [ ] Angular (ngx-translate, Transloco) (v0.2)
- [ ] Laravel (`__`, `trans`, `@lang`, Blade) (v0.2)
- [ ] Flutter (ARB + GetX + EasyLocalization) (v0.3)
- [ ] Django, Rails, Go i18n (v0.4+)

### Formats (ordre de priorité)

- [ ] JSON (v0.1)
- [ ] YAML (v0.1)
- [ ] ARB (v0.2)
- [ ] PHP arrays (v0.2)
- [ ] PO / gettext (v0.3)
- [ ] TOML (v0.3)
- [ ] JSON5 (v0.3)
- [ ] Properties, Strings, XLIFF, INI (v0.4+)

---

## Jalons de release

- [ ] **v0.1.0** — MVP : Vue + i18next, JSON/YAML, hover/inlay/def/completion/diagnostics
- [ ] **v0.2.0** — Code actions (extract, rename), +ARB/PHP, +React-Intl/Angular/Laravel
- [ ] **v0.3.0** — MCP + slash commands IA (DeepL/OpenAI), +Flutter, +PO/TOML
- [ ] **v0.4.0** — Formats exotiques, Django/Rails/Go, custom framework enrichi
- [ ] Publication sur `zed-industries/extensions`

---

## Risques & mitigations (rappel)

| Risque | Mitigation |
|---|---|
| Pas de Webview pour éditeur graphique | Slash commands + MCP dans Assistant Zed |
| Pas de Code Lens côté Zed | Inlay hints + code actions |
| Parsers fragiles | Crates éprouvées + fuzzing + tree-sitter en fallback |
| Binaire LSP multi-plateforme | CI matrix + GitHub Releases + téléchargement conditionnel |
| Perf sur gros monorepo | Indexation parallèle, cache disque, watcher incrémental |
| Concurrence `intl-lens` | Positionnement Vue/Nuxt-first + features avancées (MCP, refactor) |

---

## Notes de session

- Inspiration MIT : on peut réutiliser les YAML de frameworks d'i18n-ally
  (`src/frameworks/*.yml` du repo `lokalise/i18n-ally`).
- Tester localement via `zed: install dev extension` + `zed --foreground` pour les logs.
- Rust doit être installé via rustup (pas via Homebrew) sinon les dev extensions Zed
  ne buildent pas.
