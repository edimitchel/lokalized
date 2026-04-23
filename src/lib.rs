//! Zed extension entry point. Locates and launches the `lokalize-lsp` binary.
//!
//! Resolution order:
//! 1. `LOKALIZE_LSP_PATH` env var (read from the worktree shell env)
//! 2. `lokalize-lsp` discovered on the user's `PATH`

use zed_extension_api::{self as zed, LanguageServerId, Result, Worktree};

struct LokalizeExtension;

impl zed::Extension for LokalizeExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<zed::Command> {
        let env = worktree.shell_env();

        let from_env = env
            .iter()
            .find(|(k, _)| k == "LOKALIZE_LSP_PATH")
            .map(|(_, v)| v.clone());
        let from_path = worktree.which("lokalize-lsp");

        let command = from_env.or(from_path).ok_or_else(|| {
            "Could not find the `lokalize-lsp` binary. \
             Build it with `cargo build -p lokalize-lsp --release` and either \
             add it to your PATH or set the `LOKALIZE_LSP_PATH` environment variable."
                .to_string()
        })?;

        Ok(zed::Command {
            command,
            args: vec![],
            env,
        })
    }
}

zed::register_extension!(LokalizeExtension);
