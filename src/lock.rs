/// dfiles.lock — pins GitHub sources by commit SHA for reproducible installs.
///
/// ```toml
/// # Dotfile / command sources (existing section — SHA-256 of tarball).
/// [sources."gh:alice/dotfiles@v1.0"]
/// sha256     = "3a9f2..."
/// fetched_at = "2026-03-20T12:00:00Z"
///
/// # AI skill sources (new section — git commit SHA or tarball SHA-256).
/// [skill."gh:anthropics/skills/pdf-processing"]
/// sha        = "abc123def456..."
/// fetched_at = "2026-03-21T10:00:00Z"
/// ```
///
/// The lockfile is written after each successful fetch. On a cache miss, the
/// freshly-fetched SHA is compared against the recorded lock entry — a mismatch
/// is a hard error (see `SkillCache::ensure`).
///
/// Skills use their full source key (e.g. `"gh:owner/repo/subpath"`) as the TOML
/// table key, not the skill name, so renames don't invalidate the lock.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A locked dotfile/command source entry (existing behavior).
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LockEntry {
    /// SHA-256 hex digest of the downloaded tarball.
    pub sha256: String,
    /// RFC-3339 timestamp when this source was last fetched.
    pub fetched_at: String,
}

/// A locked AI skill entry.
///
/// `sha` holds either a git commit SHA (sparse checkout path) or a tarball
/// SHA-256 (tarball fallback). It is an opaque string used only for cache
/// validation — both forms are 40–64 hex chars and never compared across types.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SkillLockEntry {
    /// Opaque SHA: git commit SHA when fetched via sparse checkout,
    /// or tarball SHA-256 when fetched via tarball fallback.
    pub sha: String,
    /// RFC-3339 timestamp when this skill was last fetched.
    pub fetched_at: String,
}

/// The full contents of `dfiles.lock`.
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct LockFile {
    /// Map from source key (e.g. `"gh:alice/dotfiles@v1.0"`) to its lock entry.
    #[serde(default)]
    pub sources: HashMap<String, LockEntry>,

    /// Map from full skill source key (e.g. `"gh:anthropics/skills/pdf-processing"`)
    /// to its lock entry.  Absent from old lock files — `#[serde(default)]` ensures
    /// backward compatibility.
    #[serde(default)]
    pub skill: HashMap<String, SkillLockEntry>,
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

    /// Return the pinned SHA for a dotfile/command source key, if present.
    #[allow(dead_code)]
    pub fn pinned_sha(&self, key: &str) -> Option<&str> {
        self.sources.get(key).map(|e| e.sha256.as_str())
    }

    /// Record a newly fetched AI skill with its SHA.
    pub fn pin_skill(&mut self, key: &str, sha: &str) {
        self.skill.insert(
            key.to_string(),
            SkillLockEntry {
                sha: sha.to_string(),
                fetched_at: chrono::Utc::now().to_rfc3339(),
            },
        );
    }

    /// Return the locked SHA for a skill source key, if present.
    pub fn skill_sha(&self, key: &str) -> Option<&str> {
        self.skill.get(key).map(|e| e.sha.as_str())
    }

    /// Remove the lock entry for a skill source key.
    ///
    /// Used by `dfiles ai update` to force a re-fetch: clearing the lock SHA
    /// causes `SkillCache::ensure()` to treat the cache as a miss and re-fetch.
    pub fn remove_skill(&mut self, key: &str) {
        self.skill.remove(key);
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

    #[test]
    fn pin_skill_and_retrieve() {
        let mut lock = LockFile::default();
        lock.pin_skill("gh:anthropics/skills/pdf-processing", "abc123git");
        assert_eq!(
            lock.skill_sha("gh:anthropics/skills/pdf-processing"),
            Some("abc123git")
        );
        assert_eq!(lock.skill_sha("gh:nobody/missing"), None);
    }

    #[test]
    fn skill_section_round_trips() {
        let dir = TempDir::new().unwrap();
        let mut lock = LockFile::load(dir.path()).unwrap();
        lock.pin_skill("gh:anthropics/skills/pdf-processing", "deadbeef");
        lock.save(dir.path()).unwrap();

        let loaded = LockFile::load(dir.path()).unwrap();
        assert_eq!(
            loaded.skill_sha("gh:anthropics/skills/pdf-processing"),
            Some("deadbeef")
        );
        // Existing sources section is untouched.
        assert!(loaded.sources.is_empty());
    }

    #[test]
    fn old_lock_without_skill_section_loads_cleanly() {
        let dir = TempDir::new().unwrap();
        // Write a lock file that has only the [sources] section (old format).
        std::fs::write(
            dir.path().join("dfiles.lock"),
            r#"[sources."gh:alice/dotfiles@v1.0"]
sha256 = "abc123"
fetched_at = "2026-03-20T12:00:00Z"
"#,
        )
        .unwrap();

        let lock = LockFile::load(dir.path()).unwrap();
        assert_eq!(lock.pinned_sha("gh:alice/dotfiles@v1.0"), Some("abc123"));
        // skill section defaults to empty — no error.
        assert!(lock.skill.is_empty());
    }
}
