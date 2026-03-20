/// `dfiles-manifest.json` — the package manifest written by `dfiles publish`
/// and read by `dfiles bootstrap` when installing a remote environment.
///
/// Example `dfiles-manifest.json`:
///
/// ```json
/// {
///   "name": "my-ai-env",
///   "version": "v1.0",
///   "author": "jstegeman",
///   "profiles": ["default", "work"],
///   "modules": ["shell", "git", "packages", "ai"],
///   "created": "2026-03-20"
/// }
/// ```
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Deserialize, Serialize)]
pub struct DfilesManifest {
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub modules: Vec<String>,
    pub created: Option<String>,
}

impl DfilesManifest {
    /// Load `dfiles-manifest.json` from `repo_root`. Returns an error if the
    /// file is absent or malformed.
    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = repo_root.join("dfiles-manifest.json");
        let text = std::fs::read_to_string(&path)?;
        let manifest: Self = serde_json::from_str(&text)?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_manifest(dir: &Path, json: &str) {
        std::fs::write(dir.join("dfiles-manifest.json"), json).unwrap();
    }

    #[test]
    fn loads_full_manifest() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            tmp.path(),
            r#"{"name":"my-env","version":"v1.0","author":"alice",
               "profiles":["default","work"],"modules":["shell","ai"],
               "created":"2026-03-20"}"#,
        );
        let m = DfilesManifest::load(tmp.path()).unwrap();
        assert_eq!(m.name, "my-env");
        assert_eq!(m.version, "v1.0");
        assert_eq!(m.author.as_deref(), Some("alice"));
        assert_eq!(m.profiles, ["default", "work"]);
        assert_eq!(m.modules, ["shell", "ai"]);
    }

    #[test]
    fn loads_minimal_manifest() {
        let tmp = TempDir::new().unwrap();
        write_manifest(tmp.path(), r#"{"name":"bare","version":"v0.1"}"#);
        let m = DfilesManifest::load(tmp.path()).unwrap();
        assert_eq!(m.name, "bare");
        assert!(m.author.is_none());
        assert!(m.profiles.is_empty());
    }

    #[test]
    fn errors_when_file_absent() {
        let tmp = TempDir::new().unwrap();
        assert!(DfilesManifest::load(tmp.path()).is_err());
    }
}
