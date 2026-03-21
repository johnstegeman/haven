/// GitHub source fetching for the `gh:owner/repo[/subpath][@ref]` notation.
///
/// Sources are specified in ai/skills.toml:
///
/// ```toml
/// [[skill]]
/// source = "gh:anthropics/skills/pdf-processing"   # subdir of a monorepo
/// source = "gh:gstack/standard-skills@v1.2"        # whole repo at a tag
/// ```
///
/// For whole-repo sources the tarball is extracted in full.
/// For subpath sources (`gh:owner/repo/subpath`) the same repo tarball is
/// downloaded and only the subtree under `subpath/` is extracted.
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

/// A parsed `gh:owner/repo[/subpath][@ref]` source.
#[derive(Debug, Clone, PartialEq)]
pub struct GhSource {
    pub owner: String,
    pub repo: String,
    /// Optional path within the repo (e.g. `"skills/pdf-processing"`).
    pub subpath: Option<String>,
    /// Optional git ref: tag, branch, or commit SHA. Defaults to HEAD when absent.
    pub git_ref: Option<String>,
}

impl GhSource {
    /// Parse a `gh:owner/repo[/subpath][@ref]` string.
    ///
    /// ```
    /// # use dfiles::github::GhSource;
    /// let s = GhSource::parse("gh:anthropics/skills/pdf-processing").unwrap();
    /// assert_eq!(s.owner, "anthropics");
    /// assert_eq!(s.repo, "skills");
    /// assert_eq!(s.subpath, Some("pdf-processing".into()));
    /// ```
    pub fn parse(s: &str) -> Result<Self> {
        let tail = s
            .strip_prefix("gh:")
            .with_context(|| format!("expected 'gh:' prefix, got: {}", s))?;

        // Split off optional @ref before splitting the path.
        let (path_part, git_ref) = match tail.split_once('@') {
            Some((left, right)) => (left, Some(right.to_string())),
            None => (tail, None),
        };

        // Split into at most 3 components: owner, repo, optional subpath.
        let mut parts = path_part.splitn(3, '/');
        let owner = parts
            .next()
            .filter(|s| !s.is_empty())
            .with_context(|| format!("expected 'owner/repo' after 'gh:', got: {}", tail))?;
        let repo = parts
            .next()
            .filter(|s| !s.is_empty())
            .with_context(|| format!("expected 'owner/repo' after 'gh:', got: {}", tail))?;
        let subpath = parts.next().filter(|s| !s.is_empty()).map(str::to_string);

        Ok(Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
            subpath,
            git_ref,
        })
    }

    /// The name used for the skill directory — the last component of the subpath
    /// if one is present, otherwise the repo name.
    pub fn name(&self) -> &str {
        self.subpath
            .as_deref()
            .and_then(|sp| sp.rsplit('/').next())
            .unwrap_or(&self.repo)
    }

    /// Cache key for `~/.dfiles/skills/`.
    ///
    /// Format: `{owner}--{repo}` or `{owner}--{repo}--{subpath-with-slashes-as-dashes}`.
    pub fn cache_key(&self) -> String {
        match &self.subpath {
            None => format!("{}--{}", self.owner, self.repo),
            Some(sp) => format!("{}--{}--{}", self.owner, self.repo, sp.replace('/', "-")),
        }
    }

    /// Canonical lock-file key — reconstructs the original source string (without @ref
    /// if absent). Used as the key in `dfiles.lock [skill]` entries.
    pub fn source_key(&self) -> String {
        let base = match &self.subpath {
            None => format!("gh:{}/{}", self.owner, self.repo),
            Some(sp) => format!("gh:{}/{}/{}", self.owner, self.repo, sp),
        };
        match &self.git_ref {
            Some(r) => format!("{}@{}", base, r),
            None => base,
        }
    }

    /// The GitHub archive tarball URL for this source.
    ///
    /// Always fetches the entire repo — subpath filtering happens at extraction.
    pub fn archive_url(&self) -> String {
        let ref_ = self.git_ref.as_deref().unwrap_or("HEAD");
        format!(
            "https://api.github.com/repos/{}/{}/tarball/{}",
            self.owner, self.repo, ref_
        )
    }
}

/// Download raw bytes from a URL, using `GITHUB_TOKEN` for auth when set.
pub fn download_bytes(url: &str) -> Result<Vec<u8>> {
    let token = std::env::var("GITHUB_TOKEN").ok();

    let mut request =
        ureq::get(url).set("User-Agent", "dfiles/0.1 (+https://github.com/dfiles-sh/dfiles)");
    if let Some(ref t) = token {
        request = request.set("Authorization", &format!("Bearer {}", t));
    }

    let resp = request
        .call()
        .with_context(|| format!("HTTP request failed: {}", url))?;

    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .context("Failed to read response body")?;

    Ok(bytes)
}

/// Extract a GitHub tarball into `dest`.
///
/// GitHub tarballs wrap all files under a single top-level directory
/// (`{owner}-{repo}-{sha}/`). This function strips that prefix.
///
/// When `subpath` is `Some("path/to/skill")`, only files inside that subtree
/// are extracted (and the subpath prefix is also stripped so `dest` contains
/// the skill files directly).
pub fn extract_tarball(bytes: &[u8], subpath: Option<&str>, dest: &Path) -> Result<()> {
    let gz = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(gz);

    for entry in archive.entries().context("Failed to read tar archive")? {
        let mut entry = entry.context("Invalid tar entry")?;
        let raw_path = entry.path().context("Invalid entry path")?.into_owned();

        // Strip the GitHub-generated top-level dir (e.g. `alice-dotfiles-abc123/`).
        let stripped: PathBuf = raw_path.components().skip(1).collect();
        if stripped.as_os_str().is_empty() {
            continue; // top-level directory entry itself
        }

        let dest_path = if let Some(sp) = subpath {
            // Only extract files under the declared subpath.
            let sp_path = Path::new(sp);
            match stripped.strip_prefix(sp_path) {
                Ok(within) if !within.as_os_str().is_empty() => dest.join(within),
                _ => continue, // outside subpath or the subpath dir entry itself
            }
        } else {
            dest.join(&stripped)
        };

        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&dest_path)?;
        } else {
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            entry
                .unpack(&dest_path)
                .with_context(|| format!("Failed to extract {}", dest_path.display()))?;
        }
    }

    Ok(())
}

/// Download a `gh:` source and extract it to `dest_dir/{source.name()}/`.
///
/// Returns the SHA-256 hex digest of the downloaded tarball (for lockfile pinning).
/// Uses the `GITHUB_TOKEN` environment variable as a Bearer token when set.
pub fn fetch_to_dir(source: &GhSource, dest_dir: &Path) -> Result<String> {
    let bytes = download_bytes(&source.archive_url())?;

    // Compute SHA-256 of the raw tarball before extracting.
    let sha = format!("{:x}", Sha256::digest(&bytes));

    let target = dest_dir.join(source.name());
    std::fs::create_dir_all(&target)
        .with_context(|| format!("Cannot create {}", target.display()))?;

    extract_tarball(&bytes, None, &target)?;

    Ok(sha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tempfile::TempDir;

    // ── parse tests ──────────────────────────────────────────────────────────

    #[test]
    fn parses_owner_repo() {
        let s = GhSource::parse("gh:alice/dotfiles").unwrap();
        assert_eq!(s.owner, "alice");
        assert_eq!(s.repo, "dotfiles");
        assert_eq!(s.subpath, None);
        assert_eq!(s.git_ref, None);
    }

    #[test]
    fn parses_owner_repo_at_tag() {
        let s = GhSource::parse("gh:alice/dotfiles@v1.2.3").unwrap();
        assert_eq!(s.owner, "alice");
        assert_eq!(s.repo, "dotfiles");
        assert_eq!(s.subpath, None);
        assert_eq!(s.git_ref, Some("v1.2.3".into()));
    }

    #[test]
    fn parses_owner_repo_at_branch() {
        let s = GhSource::parse("gh:alice/dotfiles@main").unwrap();
        assert_eq!(s.git_ref, Some("main".into()));
    }

    #[test]
    fn parses_owner_repo_subpath() {
        let s = GhSource::parse("gh:anthropics/skills/pdf-processing").unwrap();
        assert_eq!(s.owner, "anthropics");
        assert_eq!(s.repo, "skills");
        assert_eq!(s.subpath, Some("pdf-processing".into()));
        assert_eq!(s.git_ref, None);
    }

    #[test]
    fn parses_owner_repo_subpath_at_ref() {
        let s = GhSource::parse("gh:anthropics/skills/pdf-processing@main").unwrap();
        assert_eq!(s.owner, "anthropics");
        assert_eq!(s.repo, "skills");
        assert_eq!(s.subpath, Some("pdf-processing".into()));
        assert_eq!(s.git_ref, Some("main".into()));
    }

    #[test]
    fn parses_deep_subpath() {
        let s = GhSource::parse("gh:owner/monorepo/path/to/skill").unwrap();
        assert_eq!(s.subpath, Some("path/to/skill".into()));
    }

    #[test]
    fn rejects_missing_gh_prefix() {
        assert!(GhSource::parse("alice/dotfiles").is_err());
    }

    #[test]
    fn rejects_no_slash() {
        assert!(GhSource::parse("gh:alicedotfiles").is_err());
    }

    #[test]
    fn rejects_empty_owner() {
        assert!(GhSource::parse("gh:/repo").is_err());
    }

    #[test]
    fn rejects_empty_repo() {
        assert!(GhSource::parse("gh:alice/").is_err());
    }

    // ── name tests ───────────────────────────────────────────────────────────

    #[test]
    fn name_is_repo_when_no_subpath() {
        let s = GhSource::parse("gh:alice/my-skills@main").unwrap();
        assert_eq!(s.name(), "my-skills");
    }

    #[test]
    fn name_is_last_subpath_component() {
        let s = GhSource::parse("gh:anthropics/skills/pdf-processing").unwrap();
        assert_eq!(s.name(), "pdf-processing");
    }

    #[test]
    fn name_is_last_component_of_deep_subpath() {
        let s = GhSource::parse("gh:owner/repo/path/to/skill-name").unwrap();
        assert_eq!(s.name(), "skill-name");
    }

    // ── cache_key tests ──────────────────────────────────────────────────────

    #[test]
    fn cache_key_no_subpath() {
        let s = GhSource::parse("gh:alice/dotfiles").unwrap();
        assert_eq!(s.cache_key(), "alice--dotfiles");
    }

    #[test]
    fn cache_key_with_subpath() {
        let s = GhSource::parse("gh:anthropics/skills/pdf-processing").unwrap();
        assert_eq!(s.cache_key(), "anthropics--skills--pdf-processing");
    }

    #[test]
    fn cache_key_deep_subpath_slashes_become_dashes() {
        let s = GhSource::parse("gh:owner/repo/path/to/skill").unwrap();
        assert_eq!(s.cache_key(), "owner--repo--path-to-skill");
    }

    // ── source_key tests ─────────────────────────────────────────────────────

    #[test]
    fn source_key_round_trips() {
        let cases = [
            "gh:alice/dotfiles",
            "gh:alice/dotfiles@v1.0",
            "gh:anthropics/skills/pdf-processing",
            "gh:anthropics/skills/pdf-processing@main",
        ];
        for case in cases {
            let s = GhSource::parse(case).unwrap();
            assert_eq!(s.source_key(), case, "source_key round-trip failed for {}", case);
        }
    }

    // ── archive_url tests ────────────────────────────────────────────────────

    #[test]
    fn archive_url_without_ref_uses_head() {
        let s = GhSource::parse("gh:alice/dotfiles").unwrap();
        assert_eq!(
            s.archive_url(),
            "https://api.github.com/repos/alice/dotfiles/tarball/HEAD"
        );
    }

    #[test]
    fn archive_url_uses_repo_not_subpath() {
        // Subpath sources still fetch the whole repo tarball.
        let s = GhSource::parse("gh:anthropics/skills/pdf-processing").unwrap();
        assert_eq!(
            s.archive_url(),
            "https://api.github.com/repos/anthropics/skills/tarball/HEAD"
        );
    }

    #[test]
    fn archive_url_with_ref() {
        let s = GhSource::parse("gh:alice/dotfiles@v2.0").unwrap();
        assert_eq!(
            s.archive_url(),
            "https://api.github.com/repos/alice/dotfiles/tarball/v2.0"
        );
    }

    // ── extract_tarball tests ────────────────────────────────────────────────

    /// Build a minimal `.tar.gz` in memory for testing extraction.
    ///
    /// Layout inside the archive (mimicking GitHub's wrapping dir):
    /// ```
    /// fake-owner-fake-repo-abc123/
    ///   SKILL.md
    ///   main.sh
    ///   subskill/
    ///     nested.md
    /// ```
    fn make_test_tarball() -> Vec<u8> {
        let buf = Vec::new();
        let gz = GzEncoder::new(buf, Compression::default());
        let mut ar = tar::Builder::new(gz);

        let add_file = |ar: &mut tar::Builder<_>, path: &str, content: &[u8]| {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            ar.append(&header, content).unwrap();
        };

        let add_dir = |ar: &mut tar::Builder<_>, path: &str| {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_size(0);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Directory);
            header.set_cksum();
            ar.append(&header, &[][..]).unwrap();
        };

        add_dir(&mut ar, "fake-owner-fake-repo-abc123/");
        add_file(&mut ar, "fake-owner-fake-repo-abc123/SKILL.md", b"---\nname: test\n---\n");
        add_file(&mut ar, "fake-owner-fake-repo-abc123/main.sh", b"#!/bin/sh\necho hi\n");
        add_dir(&mut ar, "fake-owner-fake-repo-abc123/subskill/");
        add_file(&mut ar, "fake-owner-fake-repo-abc123/subskill/nested.md", b"# Nested\n");

        let gz = ar.into_inner().unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn extract_tarball_whole_repo() {
        let bytes = make_test_tarball();
        let dir = TempDir::new().unwrap();

        extract_tarball(&bytes, None, dir.path()).unwrap();

        assert!(dir.path().join("SKILL.md").exists());
        assert!(dir.path().join("main.sh").exists());
        assert!(dir.path().join("subskill/nested.md").exists());
    }

    #[test]
    fn extract_tarball_subpath_only() {
        let bytes = make_test_tarball();
        let dir = TempDir::new().unwrap();

        extract_tarball(&bytes, Some("subskill"), dir.path()).unwrap();

        // Only files from subskill/ should be present.
        assert!(dir.path().join("nested.md").exists());
        // Root files should NOT be extracted.
        assert!(!dir.path().join("SKILL.md").exists());
        assert!(!dir.path().join("main.sh").exists());
    }

    #[test]
    fn extract_tarball_subpath_strips_prefix() {
        let bytes = make_test_tarball();
        let dir = TempDir::new().unwrap();

        // nested.md is at subskill/nested.md inside the tarball.
        // After extraction with subpath="subskill", it should be at dest/nested.md.
        extract_tarball(&bytes, Some("subskill"), dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("nested.md")).unwrap();
        assert_eq!(content, "# Nested\n");
    }
}
