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

- [x] Scan du worktree à l'ouverture, lecture `.zed/lokalize.json` (`config::ProjectConfig::load`)
- [x] Heuristiques fallback : `locales/`, `src/locales/`, `i18n/`, `public/locales/`, `lib/l10n/`, `app/locales/`, `assets/locales/`
- [x] Détection structure : **flat** (`en.json`) vs **nested** (`en/common.json`) (`IndexBuilder::detect_layout`)
- [x] Détection de la `sourceLocale` (défaut `en`, override par config)

### Parsers avec positions source

- [x] JSON / JSONC / JSON5 / ARB (`jsonc-parser 0.32`, positions byte/line/UTF-16)
- [x] ARB (Flutter) — JSON + métadonnées `@key` ignorées
- [ ] YAML (crate `yaml-rust2` ou `saphyr` + extraction positions)
- [ ] PHP arrays — parser regex/AST minimaliste
- [ ] Parsers différés v0.2+ : PO/gettext, TOML, INI, Properties, Strings, XLIFF

### Index

- [x] `LocaleIndex` : `BTreeMap<Locale, KeyTree>` avec feuilles `{ value, file, range }`
- [x] Support clés à plat (`a.b.c`) et imbriquées
- [x] API de lookup multi-locale + `missing_keys` + `all_keys`
- [ ] Support `linked messages` vue-i18n (`@:other.key`)
- [ ] Index secondaire : `HashMap<Key, Vec<Location>>` pour go-to-def rapide

### Watcher (Phase 1.5)

- [ ] `notify` crate, debounce 100 ms
- [ ] Réindexation incrémentale par fichier modifié
- [ ] Invalidation propre du cache

### Tests

- [x] Tests unitaires : 14 tests (positions, Locale, JSON parser, KeyTree, diff)
- [x] Tests d'intégration avec fixtures : `nested_project` + `flat_project` + erreur "no locales"
- [x] **17 tests verts** sur `cargo test -p i18n-core`
- [ ] Fixtures multi-framework supplémentaires (vue-i18n, Flutter ARB réel)

### Intégration LSP

- [x] Le LSP charge `ProjectConfig` + construit `LocaleIndex` au `initialize` (async, non bloquant)
- [x] Log structuré du résultat dans Zed log : `Lokalize: indexed N locales, M files, K keys`
- [x] `Arc<RwLock<Option<LocaleIndex>>>` partagé, prêt pour les handlers hover/inlay/def (Phase 3)

---

## Phase 2 — Détection des usages dans le code (≈ 2-3 j)

- [x] Frameworks built-in hard-codés : `vue-i18n`, `nuxt-i18n`, `i18next`, `react-intl`
- [x] `Framework` struct + `KeyUsage { key, range, scope, framework_id }`
- [x] `KeyFinder` : regex avec placeholder `{key}`, capture group 1 = clé
- [x] Patterns reconnus : `$t/$tc/$rt/t/tc/i18n.t/keypath=/useTranslation`, `<Trans i18nKey>`, `formatMessage({id})`, `<FormattedMessage id>`
- [x] Résolution du scope (ex. `useTranslation("forms") + t("submit")` → `forms.submit`)
- [x] Dédup multi-framework (priorité au match avec scope résolu)
- [x] 10 tests unitaires + **27 tests verts** (lib + intégration)
- [ ] Support custom framework via `.zed/i18n-ally-custom-framework.yml` (Phase 2.5)
- [ ] Option robustesse : tree-sitter pour réduire les faux positifs dans commentaires/strings multi-lignes (Phase 2.5)

---

## Phase 3 — Fonctions LSP utilisateur (≈ 4-6 j)

### Capabilities dans `initialize`

- [x] hoverProvider
- [x] inlayHintProvider
- [x] definitionProvider
- [x] completionProvider (trigger chars : `"`, `'`, `` ` ``, `.`)
- [ ] referencesProvider (Phase 3.5)
- [ ] codeActionProvider (Phase 4)
- [ ] renameProvider (Phase 4)
- [ ] workspaceSymbolProvider (Phase 3.5)
- [ ] documentSymbolProvider (Phase 3.5)

### Handlers

- [x] **Hover** : markdown listant toutes les locales (via `idx.lookup(key)`)
- [x] **Inlay Hint** : ` = <traduction>` après chaque `t(…)`, tronquée à 40 chars
- [x] **Definition** : liste de `Location` pointant chaque fichier de locale
- [x] **Completion** : toutes les clés du projet avec preview source en `detail`
- [x] **Diagnostics** push sur `didOpen` / `didChange` :
  - [x] clé utilisée mais absente de toutes les locales → WARNING `missing-key`
  - [x] clé utilisée mais absente de la source locale → INFO `missing-source`
  - [ ] valeur vide dans une locale (Phase 3.5)
  - [ ] clé inutilisée (scan global, sur le fichier de locale) (Phase 3.5)
- [ ] **Workspace Symbol** : fuzzy match sur toutes les clés (Phase 3.5)

### Config workspace

- [x] Lecture de `.zed/lokalize.json` côté LSP (`ProjectConfig::load`)
- [ ] Réception de `workspace/configuration` via LSP standard (Phase 3.5)
- [ ] Rechargement à chaud sur `workspace/didChangeConfiguration` (Phase 3.5)

### Document store

- [x] `Arc<RwLock<HashMap<Url, DocumentState>>>` — text + language_id + version
- [x] `did_open` / `did_change` (TextDocumentSyncKind::FULL) / `did_close`
- [x] Helper `usage_at_position(doc, pos)` pour hover/definition
- [x] Helper `LineIndex::offset_at(line, char)` pour position → byte offset

### Perf

- [x] Index partagé `Arc<RwLock<Option<LocaleIndex>>>`
- [x] Construction de l'index hors-main-thread via `spawn_blocking` (handshake instant)
- [ ] Parsing des locales en parallèle avec `rayon` (Phase 1.5)
- [ ] Cache disque dans `$XDG_CACHE_HOME/lokalize/<hash>.bin` (bincode) (Phase 6)

### Hot reload (file watcher)

- [x] Crate `notify` sur les `localePaths` résolus (récursif)
- [x] Filtre : seuls create/modify/remove sur `.json/.jsonc/.json5/.arb/.yml/.yaml`
- [x] Bridge sync→async via `tokio::sync::mpsc`
- [x] Debounce 300ms pour agréger les events en rafale
- [x] Rebuild complet de l'index + swap atomique
- [x] `republish_diagnostics` pour chaque doc ouvert
- [x] `client.inlay_hint_refresh()` pour invalider le cache inlay côté Zed
- [x] Config `namespace: false` (parité i18n-ally pour JSON self-wrappés)

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
