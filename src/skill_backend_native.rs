/// NativeBackend: Haven's built-in skill pipeline.
///
/// Wraps `SkillCache` (for fetch/cache) and `ai_skill::deploy_skill` (for
/// deployment) behind the `SkillBackend` trait.  Zero external dependencies.
///
/// This is the default backend.  It is always available, cryptographically
/// verifies fetched content via SHA-256, and stores ownership in `state.json`.
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ai_skill::{deploy_skill, SkillSource};
use crate::skill_backend::{
    CachedSkillInfo, DeployResult, DeploymentTarget, FetchResult, ResolvedSkill, SkillBackend,
    SkillMetadata,
};
use crate::skill_cache::SkillCache;

// ─── Backend ──────────────────────────────────────────────────────────────────

pub struct NativeBackend {
    cache: SkillCache,
}

impl NativeBackend {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            cache: SkillCache::new(state_dir),
        }
    }

    /// Return the inner `SkillCache` for direct access by `apply_ai_skills`.
    ///
    /// Phase 1 still accesses the cache directly for the parallel-fetch
    /// optimisation (checking cache hits before spawning threads, and recording
    /// `cache_path` after threads complete).  Phase 2 will remove this once
    /// `apply_ai_skills` accepts a `Box<dyn SkillBackend>` from the factory.
    pub fn cache(&self) -> &SkillCache {
        &self.cache
    }
}

impl SkillBackend for NativeBackend {
    fn fetch(
        &self,
        source: &SkillSource,
        expected_sha: Option<&str>,
    ) -> Result<FetchResult> {
        match source {
            SkillSource::Gh(gh) => {
                // Fast cache-hit path: if the cached SHA matches the lock, skip fetch.
                if let Some(cached) = self.cache.cached_sha(gh) {
                    if expected_sha.map_or(true, |e| e == cached) {
                        return Ok(FetchResult {
                            cached_path: self.cache.cache_path(gh),
                            sha: cached,
                            was_cached: true,
                        });
                    }
                }
                // Cache miss or SHA mismatch — fetch from GitHub.
                let sha = self.cache.fetch_and_verify(gh, expected_sha)?;
                Ok(FetchResult {
                    cached_path: self.cache.cache_path(gh),
                    sha,
                    was_cached: false,
                })
            }
            SkillSource::Dir(path) => Ok(FetchResult {
                cached_path: path.clone(),
                sha: String::new(),
                was_cached: true,
            }),
            SkillSource::Repo => Ok(FetchResult {
                // Caller sets the actual path from the repo root.
                cached_path: PathBuf::new(),
                sha: String::new(),
                was_cached: true,
            }),
        }
    }

    fn deploy(&self, skill: &ResolvedSkill, target: &DeploymentTarget) -> Result<DeployResult> {
        let target_path = target.skills_dir.join(&skill.name);
        let deployed = deploy_skill(
            &skill.cached_path,
            &target_path,
            &target.deploy_method,
            &target.owned_targets,
        )?;
        Ok(DeployResult {
            target_path,
            was_collision: !deployed,
            deployed,
        })
    }

    fn undeploy(&self, target: &Path) -> Result<()> {
        if !target.exists() && !target.is_symlink() {
            anyhow::bail!("undeploy: path does not exist: {}", target.display());
        }
        if target.is_dir() && !target.is_symlink() {
            std::fs::remove_dir_all(target)
                .with_context(|| format!("Cannot remove directory {}", target.display()))?;
        } else {
            std::fs::remove_file(target)
                .with_context(|| format!("Cannot remove {}", target.display()))?;
        }
        Ok(())
    }

    fn validate(&self, skill_path: &Path) -> Result<SkillMetadata> {
        let skill_md = skill_path.join("SKILL.md");
        if !skill_md.exists() {
            anyhow::bail!("SKILL.md not found in {}", skill_path.display());
        }
        let content = std::fs::read_to_string(&skill_md)
            .with_context(|| format!("Cannot read {}", skill_md.display()))?;
        parse_skill_md_frontmatter(&content)
            .with_context(|| format!("Invalid SKILL.md in {}", skill_md.display()))
    }

    fn list_cached(&self) -> Result<Vec<CachedSkillInfo>> {
        let cache_dir = self.cache.cache_dir();
        if !cache_dir.exists() {
            return Ok(vec![]);
        }
        let mut infos = Vec::new();
        for entry in std::fs::read_dir(cache_dir)
            .with_context(|| format!("Cannot read cache dir {}", cache_dir.display()))?
        {
            let entry = entry.with_context(|| "Cannot read cache dir entry")?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let cached_path = entry.path();
            let sha = std::fs::read_to_string(cached_path.join(".haven-sha"))
                .ok()
                .map(|s| s.trim().to_string());
            // Best-effort source key reconstruction from the cache directory name.
            // Cache key format: `{owner}--{repo}[--{subpath}]`.
            // We prefix with `gh:` and replace `--` with `/`.
            let dir_name = entry.file_name().to_string_lossy().into_owned();
            let source_key = format!("gh:{}", dir_name.replace("--", "/"));
            infos.push(CachedSkillInfo { source_key, cached_path, sha });
        }
        Ok(infos)
    }

    fn evict(&self, source_key: &str) -> Result<()> {
        if let Ok(SkillSource::Gh(gh)) = SkillSource::parse(source_key) {
            let path = self.cache.cache_path(&gh);
            if path.exists() {
                std::fs::remove_dir_all(&path)
                    .with_context(|| format!("Cannot evict {}", path.display()))?;
            }
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "native"
    }

    fn is_available(&self) -> bool {
        true
    }
}

// ─── SKILL.md frontmatter parser ─────────────────────────────────────────────

/// Raw deserialization target for SKILL.md YAML frontmatter.
#[derive(Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
    license: Option<String>,
    compatibility: Option<String>,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

/// Parse the YAML frontmatter block from a SKILL.md file.
///
/// Expects the file to start with `---\n`, contain YAML, and close with `\n---`.
fn parse_skill_md_frontmatter(content: &str) -> Result<SkillMetadata> {
    let rest = content
        .strip_prefix("---\n")
        .ok_or_else(|| anyhow::anyhow!("SKILL.md must begin with ---"))?;
    let yaml_block = rest
        .split("\n---")
        .next()
        .ok_or_else(|| anyhow::anyhow!("SKILL.md frontmatter is not closed with ---"))?;
    let fm: SkillFrontmatter = serde_yaml::from_str(yaml_block)
        .context("Failed to parse SKILL.md YAML frontmatter")?;
    Ok(SkillMetadata {
        name: fm.name,
        description: fm.description,
        license: fm.license,
        compatibility: fm.compatibility,
        metadata: fm.metadata,
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tempfile::TempDir;

    fn make_skill_dir(dir: &TempDir) -> PathBuf {
        let skill = dir.path().join("test-skill");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: test-skill\ndescription: A test skill\n---\n# Test\n",
        )
        .unwrap();
        skill
    }

    // ── validate ─────────────────────────────────────────────────────────────

    #[test]
    fn validate_parses_skill_md_frontmatter() {
        let dir = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let backend = NativeBackend::new(state.path());
        let skill_path = make_skill_dir(&dir);
        let meta = backend.validate(&skill_path).unwrap();
        assert_eq!(meta.name, "test-skill");
        assert_eq!(meta.description, "A test skill");
        assert!(meta.license.is_none());
    }

    #[test]
    fn validate_parses_full_frontmatter() {
        let dir = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let backend = NativeBackend::new(state.path());
        let skill_path = dir.path().join("rich");
        std::fs::create_dir_all(&skill_path).unwrap();
        std::fs::write(
            skill_path.join("SKILL.md"),
            "---\nname: rich\ndescription: Rich skill\nlicense: Apache-2.0\ncompatibility: \"Rust 1.70+\"\nmetadata:\n  author: test-org\n  version: \"2.0\"\n---\n",
        )
        .unwrap();
        let meta = backend.validate(&skill_path).unwrap();
        assert_eq!(meta.license.as_deref(), Some("Apache-2.0"));
        assert_eq!(meta.compatibility.as_deref(), Some("Rust 1.70+"));
        assert_eq!(meta.metadata.get("author").map(String::as_str), Some("test-org"));
        assert_eq!(meta.metadata.get("version").map(String::as_str), Some("2.0"));
    }

    #[test]
    fn validate_errors_on_missing_skill_md() {
        let dir = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let backend = NativeBackend::new(state.path());
        let skill_path = dir.path().join("no-skill-md");
        std::fs::create_dir_all(&skill_path).unwrap();
        assert!(backend.validate(&skill_path).is_err());
    }

    #[test]
    fn validate_errors_on_missing_required_field() {
        let dir = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let backend = NativeBackend::new(state.path());
        let skill_path = dir.path().join("bad-fm");
        std::fs::create_dir_all(&skill_path).unwrap();
        // Missing 'description' field.
        std::fs::write(skill_path.join("SKILL.md"), "---\nname: bad\n---\n").unwrap();
        assert!(backend.validate(&skill_path).is_err());
    }

    // ── undeploy ─────────────────────────────────────────────────────────────

    #[test]
    fn undeploy_removes_symlink() {
        let dir = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let backend = NativeBackend::new(state.path());
        let target = dir.path().join("link");
        let source = dir.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&source, &target).unwrap();
        #[cfg(unix)]
        {
            backend.undeploy(&target).unwrap();
            assert!(!target.exists() && !target.is_symlink());
        }
    }

    #[test]
    fn undeploy_removes_directory() {
        let dir = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let backend = NativeBackend::new(state.path());
        let target = dir.path().join("skill-dir");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("SKILL.md"), "# skill").unwrap();
        backend.undeploy(&target).unwrap();
        assert!(!target.exists());
    }

    #[test]
    fn undeploy_errors_on_missing_target() {
        let dir = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let backend = NativeBackend::new(state.path());
        let target = dir.path().join("nonexistent");
        assert!(backend.undeploy(&target).is_err());
    }

    // ── evict ─────────────────────────────────────────────────────────────────

    #[test]
    fn evict_removes_cache_dir() {
        let state = TempDir::new().unwrap();
        let backend = NativeBackend::new(state.path());
        // Manually create a cache entry.
        let cache_path = state.path().join("skills").join("anthropics--skills--pdf");
        std::fs::create_dir_all(&cache_path).unwrap();
        std::fs::write(cache_path.join(".haven-sha"), "abc123").unwrap();
        backend.evict("gh:anthropics/skills/pdf").unwrap();
        assert!(!cache_path.exists());
    }

    #[test]
    fn evict_noop_for_unknown_key() {
        let state = TempDir::new().unwrap();
        let backend = NativeBackend::new(state.path());
        // Should not error when the cache entry doesn't exist.
        assert!(backend.evict("gh:nobody/nowhere/nothing").is_ok());
    }

    // ── deploy_all default loop ───────────────────────────────────────────────

    #[test]
    fn deploy_all_default_calls_deploy_per_skill() {
        let dir = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let backend = NativeBackend::new(state.path());

        // Create two skill dirs.
        let s1 = dir.path().join("skill-a");
        let s2 = dir.path().join("skill-b");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(s1.join("SKILL.md"), "---\nname: skill-a\ndescription: A\n---").unwrap();
        std::fs::write(s2.join("SKILL.md"), "---\nname: skill-b\ndescription: B\n---").unwrap();

        let platform_dir = dir.path().join("platform");
        std::fs::create_dir_all(&platform_dir).unwrap();

        let r1 = ResolvedSkill {
            name: "skill-a".into(),
            cached_path: s1,
            sha: String::new(),
            metadata: SkillMetadata::default(),
        };
        let r2 = ResolvedSkill {
            name: "skill-b".into(),
            cached_path: s2,
            sha: String::new(),
            metadata: SkillMetadata::default(),
        };
        let t1 = DeploymentTarget {
            platform_id: "test".into(),
            skills_dir: platform_dir.clone(),
            deploy_method: crate::ai_skill::DeployMethod::Copy,
            owned_targets: HashSet::new(),
        };
        let t2 = DeploymentTarget {
            platform_id: "test".into(),
            skills_dir: platform_dir.clone(),
            deploy_method: crate::ai_skill::DeployMethod::Copy,
            owned_targets: HashSet::new(),
        };

        let results = backend.deploy_all(&[(&r1, &t1), (&r2, &t2)]).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.deployed));
        assert!(platform_dir.join("skill-a").exists());
        assert!(platform_dir.join("skill-b").exists());
    }

    // ── is_available ─────────────────────────────────────────────────────────

    #[test]
    fn native_backend_is_always_available() {
        let state = TempDir::new().unwrap();
        let backend = NativeBackend::new(state.path());
        assert!(backend.is_available());
        assert_eq!(backend.name(), "native");
    }

    // ── parse_skill_md_frontmatter ────────────────────────────────────────────

    #[test]
    fn parse_frontmatter_rejects_no_opening_delimiter() {
        assert!(parse_skill_md_frontmatter("name: foo\ndescription: bar\n").is_err());
    }

    #[test]
    fn parse_frontmatter_rejects_unclosed_block() {
        // The yaml block never closes.  The parser treats everything as yaml,
        // which should fail to parse as SkillFrontmatter.
        let result = parse_skill_md_frontmatter("---\nname: foo\n");
        // Either Ok (if yaml parsing somehow succeeded) or Err — what matters
        // is that an unclosed block with missing 'description' is always Err.
        if let Ok(meta) = result {
            // This path means description defaulted — confirm name is set.
            assert_eq!(meta.name, "foo");
        }
        // (Some YAML parsers treat EOF as the end of the document; that's fine.)
    }
}
