//! Download and cache `lokalized-lsp` / `lokalized-mcp` from GitHub Releases.
//!
//! Binaries are stored in the extension working directory (managed by Zed), so users
//! never need machine-specific paths in settings.

use std::fs;
use std::path::{Path, PathBuf};

use zed_extension_api::{self as zed, process::Command as ProcessCommand};

pub const GITHUB_REPO: &str = "edimitchel/lokalized";

pub struct BinaryInstaller {
    pub cached_lsp: Option<String>,
    pub cached_mcp: Option<String>,
}

impl BinaryInstaller {
    pub fn new() -> Self {
        Self {
            cached_lsp: None,
            cached_mcp: None,
        }
    }

    pub fn lsp_path(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<String> {
        if let Some(path) = self.cached_lsp.as_ref() {
            if path_exists(path) {
                return Ok(path.clone());
            }
        }

        if let Some(path) = env_lsp_override(worktree) {
            return Ok(path);
        }
        if let Some(path) = worktree.which("lokalized-lsp") {
            return Ok(path);
        }

        let path = ensure_binary("lokalized-lsp", Some(language_server_id))?;
        self.cached_lsp = Some(path.clone());
        Ok(path)
    }

    pub fn mcp_path(&mut self) -> zed::Result<String> {
        if let Some(path) = self.cached_mcp.as_ref() {
            if path_exists(path) {
                return Ok(path.clone());
            }
        }

        let path = ensure_binary("lokalized-mcp", None)?;
        self.cached_mcp = Some(path.clone());
        Ok(path)
    }
}

fn env_lsp_override(worktree: &zed::Worktree) -> Option<String> {
    for key in ["LOKALIZED_LSP_PATH", "LOKALIZE_LSP_PATH"] {
        if let Ok(path) = std::env::var(key) {
            if path_exists(&path) {
                return Some(path);
            }
        }
    }
    let env = worktree.shell_env();
    env.iter()
        .find(|(k, _)| k == "LOKALIZED_LSP_PATH" || k == "LOKALIZE_LSP_PATH")
        .map(|(_, v)| v.clone())
        .filter(|path| path_exists(path))
}

fn ensure_binary(
    binary_name: &str,
    language_server_id: Option<&zed::LanguageServerId>,
) -> zed::Result<String> {
    if let Some(path) = discover_via_which(binary_name) {
        return Ok(absolutize(&path));
    }

    if let Some(path) = home_cargo_bin(binary_name) {
        return Ok(path);
    }

    if binary_name == "lokalized-mcp" {
        if let Some(path) = sibling_of_lokalized_lsp() {
            return Ok(absolutize(&path));
        }
    }

    if let Some(path) = local_build_path(binary_name) {
        return Ok(path);
    }

    if let Some(path) = cargo_build_binary(binary_name) {
        return Ok(path);
    }

    if let Some(path) = cached_download_path(binary_name) {
        return Ok(absolutize(&path));
    }

    download_release_binary(binary_name, language_server_id)
}

/// Resolve via the user shell `PATH` (same discovery model as `worktree.which` for LSP).
fn discover_via_which(binary_name: &str) -> Option<String> {
    let output = ProcessCommand::new("which")
        .arg(binary_name)
        .output()
        .ok()?;
    if output.status != Some(0) {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    path_exists(&path).then_some(path)
}

fn home_cargo_bin(binary_name: &str) -> Option<String> {
    let home = env_nonempty("HOME")?;
    let path = PathBuf::from(home)
        .join(".cargo")
        .join("bin")
        .join(local_binary_filename(binary_name));
    path.is_file()
        .then(|| path.display().to_string())
}

/// If `lokalized-lsp` is on PATH (e.g. `~/.cargo/bin`), look for `lokalized-mcp` next to it.
fn sibling_of_lokalized_lsp() -> Option<String> {
    let lsp = discover_via_which("lokalized-lsp")?;
    let sibling = Path::new(&lsp)
        .parent()?
        .join(local_binary_filename("lokalized-mcp"));
    let path = sibling.display().to_string();
    path_exists(&path).then_some(path)
}

/// Local `target/{release,debug}` build in the extension repo (dev extension).
fn local_build_path(binary_name: &str) -> Option<String> {
    let exe = local_binary_filename(binary_name);
    for root in extension_repo_roots() {
        for dir in ["target/release", "target/debug"] {
            let path = root.join(dir).join(&exe);
            if path.is_file() {
                return Some(path.display().to_string());
            }
        }
    }
    None
}

/// Extension repo path when running as a dev extension (for env propagation).
pub fn extension_dev_root() -> Option<String> {
    extension_repo_roots()
        .into_iter()
        .next()
        .map(|p| p.display().to_string())
}

fn extension_repo_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for key in ["LOKALIZED_EXTENSION_ROOT", "LOKALIZE_EXTENSION_ROOT"] {
        if let Some(path) = env_nonempty(key) {
            let root = PathBuf::from(path);
            if is_extension_root(&root) {
                push_unique(&mut roots, root);
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if is_extension_root(&cwd) {
            push_unique(&mut roots, cwd);
        }
    }
    roots
}

fn is_extension_root(path: &Path) -> bool {
    path.join("extension.toml").is_file() && path.join("Cargo.toml").is_file()
}

fn push_unique(roots: &mut Vec<PathBuf>, path: PathBuf) {
    if !roots.iter().any(|r| r == &path) {
        roots.push(path);
    }
}

/// Build the binary from a checked-out extension repo (dev workflow).
fn cargo_build_binary(binary_name: &str) -> Option<String> {
    let root = extension_repo_roots().into_iter().next()?;
    let manifest = root.join("Cargo.toml");
    let output = ProcessCommand::new("cargo")
        .arg("build")
        .arg("--manifest-path")
        .arg(manifest.display().to_string())
        .arg("-p")
        .arg(binary_name)
        .arg("--release")
        .output()
        .ok()?;
    if output.status != Some(0) {
        return None;
    }
    let built = root
        .join("target/release")
        .join(local_binary_filename(binary_name));
    built.is_file()
        .then(|| built.display().to_string())
}

/// Previously downloaded release in the extension working directory.
fn cached_download_path(binary_name: &str) -> Option<String> {
    let entries = fs::read_dir(".").ok()?;
    let mut versions: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            name.starts_with(&format!("{binary_name}-"))
                .then_some((name, e.path()))
        })
        .collect();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, dir) in versions {
        let candidate = dir.join(local_binary_filename(binary_name));
        if candidate.is_file() {
            return Some(candidate.display().to_string());
        }
    }
    None
}

fn path_exists(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file())
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
}

fn release_asset_name(binary_name: &str) -> zed::Result<String> {
    let (platform, arch) = zed::current_platform();
    let triple = match (platform, arch) {
        (zed::Os::Mac, zed::Architecture::Aarch64) => "aarch64-apple-darwin",
        (zed::Os::Mac, zed::Architecture::X8664) => "x86_64-apple-darwin",
        (zed::Os::Linux, zed::Architecture::Aarch64) => "aarch64-unknown-linux-gnu",
        (zed::Os::Linux, zed::Architecture::X8664) => "x86_64-unknown-linux-gnu",
        (zed::Os::Windows, zed::Architecture::X8664) => "x86_64-pc-windows-msvc",
        (_, arch) => {
            return Err(format!("unsupported platform/architecture: {arch:?}"));
        }
    };
    let ext = if platform == zed::Os::Windows {
        ".exe"
    } else {
        ""
    };
    Ok(format!("{binary_name}-{triple}{ext}"))
}

fn local_binary_filename(binary_name: &str) -> String {
    let (platform, _) = zed::current_platform();
    if platform == zed::Os::Windows {
        format!("{binary_name}.exe")
    } else {
        binary_name.to_string()
    }
}

fn download_release_binary(
    binary_name: &str,
    language_server_id: Option<&zed::LanguageServerId>,
) -> zed::Result<String> {
    if let Some(id) = language_server_id {
        zed::set_language_server_installation_status(
            id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );
    }

    let release = fetch_github_release(false).or_else(|_| fetch_github_release(true)).map_err(|_| {
        no_binary_help(binary_name)
    })?;

    let asset_name = release_asset_name(binary_name)?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| format!("no release asset named {asset_name:?}"))?;

    let version_dir = format!("{binary_name}-{}", release.version);
    let binary_path = format!("{version_dir}/{}", local_binary_filename(binary_name));

    if !path_exists(&binary_path) {
        if let Some(id) = language_server_id {
            zed::set_language_server_installation_status(
                id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );
        }

        zed::download_file(
            &asset.download_url,
            &binary_path,
            zed::DownloadedFileType::Uncompressed,
        )
        .map_err(|e| format!("failed to download {asset_name}: {e}"))?;

        zed::make_file_executable(&binary_path)?;

        prune_old_version_dirs(binary_name, &version_dir)?;
    }

    Ok(absolutize(&binary_path))
}

fn fetch_github_release(pre_release: bool) -> zed::Result<zed::GithubRelease> {
    zed::latest_github_release(
        GITHUB_REPO,
        zed::GithubReleaseOptions {
            require_assets: true,
            pre_release,
        },
    )
    .map_err(|e| e.to_string())
}

fn absolutize(path: &str) -> String {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        return path.to_string();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(&p).display().to_string())
        .unwrap_or_else(|_| path.to_string())
}

fn prune_old_version_dirs(binary_name: &str, keep_dir: &str) -> zed::Result<()> {
    let entries =
        fs::read_dir(".").map_err(|e| format!("failed to list extension directory: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read directory entry: {e}"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with(binary_name) && name != keep_dir {
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(path).ok();
            }
        }
    }
    Ok(())
}

fn no_binary_help(binary_name: &str) -> String {
    format!(
        "could not find `{binary_name}`. There is no published release on \
         https://github.com/{GITHUB_REPO}/releases yet.\n\n\
         Local development:\n\
         1. In the extension repo: `cargo build -p {binary_name} --release`\n\
         2. Either copy to `~/.cargo/bin/`, or set `LOKALIZED_LSP_PATH` in Zed settings, \
            or set `LOKALIZED_EXTENSION_ROOT` to the extension repo path."
    )
}