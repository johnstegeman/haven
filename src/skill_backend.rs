/// Pluggable backend trait for AI skill management.
///
/// The default backend (`NativeBackend`) implements fetch + deploy entirely in
/// Haven.  External backends (e.g. `SkillKitBackend`) shell out to third-party
/// CLIs.  The interface is intentionally rich so Haven retains full observability
/// — state.json and CLAUDE.md are always driven by Haven regardless of backend.
///
/// # Implementor contract
///
/// * `fetch()` — For NativeBackend this downloads from GitHub (or returns a
///   cache hit).  For external backends (SkillKit, akm) this is a **no-op**:
///   return `FetchResult { sha: "managed-by-<backend>", was_cached: true, .. }`.
///   The `expected_sha` parameter is the SHA currently stored in `haven.lock`;
///   NativeBackend uses it for supply-chain verification.  External backends
///   **must ignore** it.
///
/// * `deploy()` — Deploy a single skill.  External backends that require
///   bulk-only operations (SkillKit) **must** return
///   `Err("use deploy_all() — this backend requires bulk deployment")` here
///   and override `deploy_all()` instead.
///
/// * `deploy_all()` — Default implementation loops over `deploy()`.
///   SkillKitBackend overrides this to generate a `.skills` manifest and invoke
///   `skillkit team install` once.
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ai_skill::DeployMethod;

// ─── Trait ────────────────────────────────────────────────────────────────────

pub trait SkillBackend: Send + Sync {
    /// Download a skill from source into local cache.
    ///
    /// Returns the content SHA for lock-file recording. For external backends
    /// this is a no-op — they manage their own cache.
    fn fetch(
        &self,
        source: &crate::ai_skill::SkillSource,
        expected_sha: Option<&str>,
    ) -> Result<FetchResult>;

    /// Deploy a single cached skill to a platform's skills directory.
    ///
    /// External backends that are bulk-only must return an error here and
    /// override `deploy_all()`.
    fn deploy(&self, skill: &ResolvedSkill, target: &DeploymentTarget) -> Result<DeployResult>;

    /// Deploy all skills in a single operation.
    ///
    /// Default implementation loops over `deploy()`.  SkillKitBackend overrides
    /// this to call `skillkit team install` once with a generated manifest.
    fn deploy_all(
        &self,
        skills: &[(&ResolvedSkill, &DeploymentTarget)],
    ) -> Result<Vec<DeployResult>> {
        skills.iter().map(|(s, t)| self.deploy(s, t)).collect()
    }

    /// Remove a deployed skill from the filesystem.
    fn undeploy(&self, target: &Path) -> Result<()>;

    /// Parse and validate a `SKILL.md`, returning its metadata.
    fn validate(&self, skill_path: &Path) -> Result<SkillMetadata>;

    /// List all skills currently in this backend's cache.
    fn list_cached(&self) -> Result<Vec<CachedSkillInfo>>;

    /// Remove a skill from this backend's cache by source key.
    fn evict(&self, source_key: &str) -> Result<()>;

    /// Human-readable backend name (used in error messages).
    fn name(&self) -> &str;

    /// Whether this backend is usable in the current environment.
    fn is_available(&self) -> bool;
}

// ─── Supporting types ─────────────────────────────────────────────────────────

pub struct FetchResult {
    /// Where the skill now lives on disk (cache dir for NativeBackend).
    pub cached_path: PathBuf,
    /// Content SHA: git commit SHA or tarball SHA-256 for NativeBackend;
    /// `"managed-by-<backend>"` for external backends.
    pub sha: String,
    /// True if the cache was already valid; false if a download was needed.
    pub was_cached: bool,
}

pub struct ResolvedSkill {
    /// Skill name (directory name under `ai/skills/`).
    pub name: String,
    /// Local path to the skill's files (cache dir or repo-local path).
    pub cached_path: PathBuf,
    /// SHA as returned by `fetch()`.
    pub sha: String,
    /// Parsed SKILL.md metadata (may be empty if validation was skipped).
    pub metadata: SkillMetadata,
}

pub struct DeploymentTarget {
    /// Platform identifier (e.g. `"claude-code"`).
    pub platform_id: String,
    /// Directory where the platform expects skills to live.
    pub skills_dir: PathBuf,
    /// Whether to symlink or copy the skill.
    pub deploy_method: DeployMethod,
    /// Paths currently owned by Haven (used for collision detection).
    pub owned_targets: HashSet<PathBuf>,
}

pub struct DeployResult {
    /// Absolute path where the skill was (or would be) deployed.
    pub target_path: PathBuf,
    /// True if the target existed and was NOT owned by Haven (skipped).
    /// Always false for external backends that manage deployment themselves.
    pub was_collision: bool,
    /// True if the skill was actually deployed; false if skipped.
    pub deployed: bool,
}

/// Metadata parsed from a `SKILL.md` frontmatter block.
#[derive(Debug, Clone, Default)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    /// Arbitrary key-value pairs from `metadata:` in the frontmatter.
    pub metadata: HashMap<String, String>,
}

pub struct CachedSkillInfo {
    /// Source key (e.g. `"gh:anthropics/skills/pdf-processing"`).
    pub source_key: String,
    /// Absolute path to the cached skill directory.
    pub cached_path: PathBuf,
    /// SHA stored in `.haven-sha`, or `None` if absent.
    pub sha: Option<String>,
}
