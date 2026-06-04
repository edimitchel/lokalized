//! Zed extension entry point — locates or downloads `lokalized-lsp` and `lokalized-mcp`.

mod binary;

use zed_extension_api::{
    self as zed, ContextServerId, LanguageServerId, Project, Result, Worktree,
};

use binary::BinaryInstaller;

struct LokalizedExtension {
    binaries: BinaryInstaller,
}

impl zed::Extension for LokalizedExtension {
    fn new() -> Self {
        Self {
            binaries: BinaryInstaller::new(),
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<zed::Command> {
        let command = self.binaries.lsp_path(language_server_id, worktree)?;
        let mut env = worktree.shell_env();
        if let Some(root) = binary::extension_dev_root() {
            env.push(("LOKALIZED_EXTENSION_ROOT".into(), root));
        }
        Ok(zed::Command {
            command,
            args: vec![],
            env,
        })
    }

    fn context_server_command(
        &mut self,
        _id: &ContextServerId,
        _project: &Project,
    ) -> Result<zed::Command> {
        let command = self.binaries.mcp_path().map_err(|e| {
            format!(
                "{e} From the extension repo run: \
                 `cargo build -p lokalized-mcp --release && cp target/release/lokalized-mcp ~/.cargo/bin/`"
            )
        })?;

        // Zed sets the extension working directory to the open project root when
        // starting a context server (same pattern as other MCP extensions).
        let workspace = std::env::current_dir()
            .map_err(|e| e.to_string())?
            .display()
            .to_string();

        Ok(zed::Command {
            command,
            args: vec![workspace.clone()],
            env: vec![
                ("LOKALIZED_WORKSPACE".into(), workspace.clone()),
                ("LOKALIZE_WORKSPACE".into(), workspace),
            ],
        })
    }

    fn context_server_configuration(
        &mut self,
        _id: &ContextServerId,
        _project: &Project,
    ) -> Result<Option<zed::ContextServerConfiguration>> {
        Ok(Some(zed::ContextServerConfiguration {
            installation_instructions: format!(
                "The **Lokalized** MCP server is installed automatically by the extension \
                 (binary downloaded from [GitHub Releases](https://github.com/{repo}/releases)).\n\n\
                 Enable **Lokalized** in Agent → Settings. No manual path configuration is required.",
                repo = binary::GITHUB_REPO
            ),
            settings_schema: r#"{"type":"object","properties":{}}"#.into(),
            default_settings: "{}".into(),
        }))
    }
}

zed::register_extension!(LokalizedExtension);
