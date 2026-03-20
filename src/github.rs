/// GitHub source fetching for the `gh:owner/repo[@ref]` notation.
///
/// Sources are specified in ai.toml:
///
/// ```toml
/// [ai]
/// skills   = ["gh:gstack/standard-skills@v1.2"]
/// commands = ["gh:jstegeman/my-commands@main"]
/// ```
///
/// Each source is downloaded as a tarball from the GitHub API, extracted into
/// a local directory, and its SHA-256 is returned for lockfile pinning.
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// A parsed `gh:owner/repo` or `gh:owner/repo@ref` source.
#[derive(Debug, Clone, PartialEq)]
pub struct GhSource {
    pub owner: String,
    pub repo: String,
    /// Optional git ref: tag, branch, or commit SHA. Defaults to HEAD when absent.
    pub git_ref: Option<String>,
}

impl GhSource {
    /// Parse a `gh:owner/repo` or `gh:owner/repo@ref` string.
    pub fn parse(s: &str) -> Result<Self> {
        let tail = s
            .strip_prefix("gh:")
            .with_context(|| format!("expected 'gh:' prefix, got: {}", s))?;

        let (repo_part, git_ref) = match tail.split_once('@') {
            Some((left, right)) => (left, Some(right.to_string())),
            None => (tail, None),
        };

        let (owner, repo) = repo_part
            .split_once('/')
            .with_context(|| format!("expected 'owner/repo' after 'gh:', got: {}", tail))?;

        if owner.is_empty() || repo.is_empty() {
            bail!("owner and repo must not be empty in '{}'", s);
        }

        Ok(Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
            git_ref,
        })
    }

    /// Name used as the installation directory (the repo name).
    pub fn name(&self) -> &str {
        &self.repo
    }

    /// The GitHub archive tarball URL for this source.
    pub fn archive_url(&self) -> String {
        let ref_ = self.git_ref.as_deref().unwrap_or("HEAD");
        format!(
            "https://api.github.com/repos/{}/{}/tarball/{}",
            self.owner, self.repo, ref_
        )
    }
}

/// Download a `gh:` source and extract it to `dest_dir/{source.name()}/`.
///
/// Returns the SHA-256 hex digest of the downloaded tarball (for lockfile pinning).
/// Uses the `GITHUB_TOKEN` environment variable as a Bearer token when set.
///
/// GitHub tarballs wrap all files under a single top-level directory
/// (`{owner}-{repo}-{sha}/`). This function strips that prefix.
pub fn fetch_to_dir(source: &GhSource, dest_dir: &Path) -> Result<String> {
    let url = source.archive_url();
    let token = std::env::var("GITHUB_TOKEN").ok();

    let mut request =
        ureq::get(&url).set("User-Agent", "dfiles/0.1 (+https://github.com/dfiles-sh/dfiles)");
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

    // Compute SHA-256 of the raw tarball before extracting.
    let digest = Sha256::digest(&bytes);
    let sha = format!("{:x}", digest);

    // Extract into dest_dir/{name}/.
    let target = dest_dir.join(source.name());
    std::fs::create_dir_all(&target)
        .with_context(|| format!("Cannot create {}", target.display()))?;

    let gz = flate2::read::GzDecoder::new(bytes.as_slice());
    let mut archive = tar::Archive::new(gz);

    for entry in archive.entries().context("Failed to read tar archive")? {
        let mut entry = entry.context("Invalid tar entry")?;
        let raw_path = entry.path().context("Invalid entry path")?.into_owned();

        // Strip the GitHub-generated top-level dir (e.g. `alice-dotfiles-abc123/`).
        let stripped: std::path::PathBuf = raw_path.components().skip(1).collect();
        if stripped.as_os_str().is_empty() {
            continue; // skip the top-level directory entry itself
        }

        let dest = target.join(&stripped);
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            entry
                .unpack(&dest)
                .with_context(|| format!("Failed to extract {}", stripped.display()))?;
        }
    }

    Ok(sha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_owner_repo() {
        let s = GhSource::parse("gh:alice/dotfiles").unwrap();
        assert_eq!(s.owner, "alice");
        assert_eq!(s.repo, "dotfiles");
        assert_eq!(s.git_ref, None);
    }

    #[test]
    fn parses_owner_repo_at_tag() {
        let s = GhSource::parse("gh:alice/dotfiles@v1.2.3").unwrap();
        assert_eq!(s.owner, "alice");
        assert_eq!(s.repo, "dotfiles");
        assert_eq!(s.git_ref, Some("v1.2.3".into()));
    }

    #[test]
    fn parses_owner_repo_at_branch() {
        let s = GhSource::parse("gh:alice/dotfiles@main").unwrap();
        assert_eq!(s.git_ref, Some("main".into()));
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
    fn archive_url_without_ref_uses_head() {
        let s = GhSource::parse("gh:alice/dotfiles").unwrap();
        assert_eq!(
            s.archive_url(),
            "https://api.github.com/repos/alice/dotfiles/tarball/HEAD"
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

    #[test]
    fn name_is_repo_segment() {
        let s = GhSource::parse("gh:alice/my-skills@main").unwrap();
        assert_eq!(s.name(), "my-skills");
    }
}
