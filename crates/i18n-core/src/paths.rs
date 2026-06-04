//! Path normalization for stable comparisons (symlinks, `/private` on macOS).

use std::path::{Path, PathBuf};

/// Canonicalize when possible; otherwise return `path` unchanged.
pub fn normalize_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Compare two filesystem paths, resolving symlinks when possible.
pub fn paths_equal(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    normalize_path(a) == normalize_path(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn paths_equal_matches_canonical_symlink() {
        let dir = TempDir::new().unwrap();
        let real = dir.path().join("global.json");
        fs::write(&real, "{}").unwrap();
        #[cfg(unix)]
        {
            let link = dir.path().join("link.json");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert!(paths_equal(&real, &link));
        }
    }
}