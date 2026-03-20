/// dfiles.lock — pins GitHub sources by commit SHA for reproducible installs.
///
/// Example lock file (`dfiles.lock` in the repo root):
///
/// ```toml
/// [sources."gh:alice/dotfiles@v1.0"]
/// sha256     = "3a9f2..."
/// fetched_at = "2026-03-20T12:00:00Z"
/// ```
///
/// The lockfile is written after each successful fetch so subsequent runs can
/// detect whether a source has changed. SHA verification (P1 TODO) will compare
/// the stored SHA against re-downloaded content before overwriting.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A single locked source entry.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LockEntry {
    /// SHA-256 hex digest of the downloaded tarball.
    pub sha256: String,
    /// RFC-3339 timestamp when this source was last fetched.
    pub fetched_at: String,
}

/// The full contents of `dfiles.lock`.
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct LockFile {
    /// Map from source key (e.g. `"gh:alice/dotfiles@v1.0"`) to its lock entry.
    #[serde(default)]
    pub sources: HashMap<String, LockEntry>,
}

impl LockFile {
    /// Load `dfiles.lock` from `repo_root`. Returns an empty lock if the file
    /// doesn't exist yet (first run).
    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = lock_path(repo_root);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("Cannot read {}", path.display()))?;
        toml::from_str(&text)
            .with_context(|| format!("Invalid TOML in {}", path.display()))
    }

    /// Write the lock file to `repo_root/dfiles.lock`.
    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let path = lock_path(repo_root);
        let text = toml::to_string_pretty(self)?;
        std::fs::write(&path, text)
            .with_context(|| format!("Cannot write {}", path.display()))
    }

    /// Record a newly fetched source with its SHA-256.
    pub fn pin(&mut self, key: &str, sha256: &str) {
        self.sources.insert(
            key.to_string(),
            LockEntry {
                sha256: sha256.to_string(),
                fetched_at: chrono::Utc::now().to_rfc3339(),
            },
        );
    }

    /// Return the pinned SHA for a source key, if present.
    ///
    /// Reserved for P1 SHA verification: compare the stored SHA against a
    /// freshly-downloaded tarball before overwriting an installed source.
    #[allow(dead_code)]
    pub fn pinned_sha(&self, key: &str) -> Option<&str> {
        self.sources.get(key).map(|e| e.sha256.as_str())
    }
}

fn lock_path(repo_root: &Path) -> PathBuf {
    repo_root.join("dfiles.lock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_returns_empty_when_file_absent() {
        let dir = TempDir::new().unwrap();
        let lock = LockFile::load(dir.path()).unwrap();
        assert!(lock.sources.is_empty());
    }

    #[test]
    fn pin_and_save_round_trips() {
        let dir = TempDir::new().unwrap();
        let mut lock = LockFile::load(dir.path()).unwrap();
        lock.pin("gh:alice/dotfiles@v1.0", "abc123");
        lock.save(dir.path()).unwrap();

        let loaded = LockFile::load(dir.path()).unwrap();
        assert_eq!(
            loaded.pinned_sha("gh:alice/dotfiles@v1.0"),
            Some("abc123")
        );
    }

    #[test]
    fn pinned_sha_returns_none_for_unknown_key() {
        let lock = LockFile::default();
        assert_eq!(lock.pinned_sha("gh:nobody/missing"), None);
    }
}
