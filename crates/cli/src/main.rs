mod ops;
mod output;

#[cfg(feature = "gui")]
mod gui;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use output::OutputFormat;

#[derive(Parser)]
#[command(
    name = "lokalized",
    version,
    about = "Lokalized — i18n CLI and translation management UI"
)]
struct Cli {
    /// Project workspace root (defaults to current directory).
    #[arg(long, global = true, value_name = "DIR")]
    workspace: Option<PathBuf>,

    /// Output format for machine-readable CI integration.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run validate + missing + unused; exit 1 if any issue (CI-friendly).
    Check {
        #[arg(long, help = "Only check missing keys for this locale")]
        locale: Option<String>,
    },
    /// Verify every locale file parses successfully.
    Validate,
    /// List keys missing from target locale(s) compared to the source locale.
    Missing {
        #[arg(long)]
        locale: Option<String>,
    },
    /// List translation keys not referenced in scanned source files.
    Unused,
    /// Translation key operations.
    Keys {
        #[command(subcommand)]
        command: KeysCommands,
    },
    /// Read a translation value (with @: link resolution).
    Get {
        key: String,
        #[arg(long)]
        locale: String,
    },
    /// Write a translation value (JSON locale files only).
    Set {
        key: String,
        #[arg(long)]
        locale: String,
        #[arg(long)]
        value: String,
    },
    /// Per-locale key counts and coverage vs source locale.
    Stats,
    /// Open the translation management window (requires `gui` feature).
    #[cfg(feature = "gui")]
    Gui,
}

#[derive(Subcommand)]
enum KeysCommands {
    /// List all keys (optional prefix / locale filter).
    List {
        #[arg(long)]
        prefix: Option<String>,
        #[arg(long)]
        locale: Option<String>,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code as u8),
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<i32> {
    let cli = Cli::parse();
    let workspace = ops::workspace_path(cli.workspace);

    match cli.command {
        Commands::Check { locale } => {
            let ctx = ops::Context::load(workspace, cli.format)?;
            ops::run_check(&ctx, locale.as_deref())
        }
        Commands::Validate => {
            let ctx = ops::Context::load(workspace, cli.format)?;
            ops::run_validate(&ctx)
        }
        Commands::Missing { locale } => {
            let ctx = ops::Context::load(workspace, cli.format)?;
            ops::run_missing(&ctx, locale.as_deref())
        }
        Commands::Unused => {
            let ctx = ops::Context::load(workspace, cli.format)?;
            ops::run_unused(&ctx)
        }
        Commands::Keys { command } => match command {
            KeysCommands::List { prefix, locale } => {
                let ctx = ops::Context::load(workspace, cli.format)?;
                ops::run_keys_list(&ctx, prefix.as_deref(), locale.as_deref())
            }
        },
        Commands::Get { key, locale } => {
            let ctx = ops::Context::load(workspace, cli.format)?;
            ops::run_get(&ctx, &key, &locale)
        }
        Commands::Set { key, locale, value } => {
            let ctx = ops::Context::load(workspace, cli.format)?;
            ops::run_set(&ctx, &key, &locale, &value)
        }
        Commands::Stats => {
            let ctx = ops::Context::load(workspace, cli.format)?;
            ops::run_stats(&ctx)
        }
        #[cfg(feature = "gui")]
        Commands::Gui => gui::run(workspace),
    }
}