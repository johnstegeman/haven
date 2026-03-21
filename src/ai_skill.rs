/// AI skill declarations: parsing `ai/skills.toml` and deploying skills
/// to platform skill directories.
///
/// ```toml
/// # ai/skills.toml
/// [[skill]]
/// name     = "pdf-processing"
/// source   = "gh:anthropics/skills/pdf-processing"
/// platforms = ["claude-code"]
/// deploy   = "symlink"
///
/// [[skill]]
/// name     = "find-skills"
/// source   = "gh:vercel-labs/skills/find-skills"
/// platforms = "all"      # all active platforms except cross-client
///
/// [[skill]]
/// name     = "my-local"
/// source   = "dir:~/projects/my-skill"
/// platforms = "cross-client"
/// deploy   = "copy"
/// ```
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::ai_platform::PlatformPlugin;
use crate::config::module::expand_tilde;
use crate::github::GhSource;

// ─── Source ──────────────────────────────────────────────────────────────────

/// A parsed skill source: either a GitHub source or a local directory.
#[derive(Debug, Clone)]
pub enum SkillSource {
    Gh(GhSource),
    Dir(PathBuf),
}

impl SkillSource {
    /// Parse a `gh:owner/repo[/subpath][@ref]` or `dir:~/path` source string.
    pub fn parse(s: &str) -> Result<Self> {
        if let Some(rest) = s.strip_prefix("dir:") {
            let path = expand_tilde(rest)
                .with_context(|| format!("Cannot expand path in '{}'", s))?;
            Ok(Self::Dir(path))
        } else if s.starts_with("gh:") {
            Ok(Self::Gh(GhSource::parse(s)?))
        } else {
            anyhow::bail!(
                "Unknown source prefix in '{}'. Expected 'gh:owner/repo' or 'dir:~/path'.",
                s
            )
        }
    }
}

// ─── Platforms field ─────────────────────────────────────────────────────────

/// The `platforms` field in a skill declaration.
///
/// TOML can hold either an array (`["claude-code"]`) or a string (`"all"`).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SkillPlatforms {
    /// Explicit list of platform IDs.
    List(Vec<String>),
    /// Named target: `"all"` or `"cross-client"`.
    Named(String),
}

// ─── Deploy method ────────────────────────────────────────────────────────────

/// How the skill is installed at the target path.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DeployMethod {
    /// Create a symlink pointing to the cache directory (default).
    #[default]
    Symlink,
    /// Copy the skill directory to the target.
    Copy,
}

impl DeployMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Symlink => "symlink",
            Self::Copy => "copy",
        }
    }
}

// ─── Declaration ─────────────────────────────────────────────────────────────

/// A single `[[skill]]` entry in `ai/skills.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillDeclaration {
    /// Globally unique skill name within this repo.
    pub name: String,
    /// Source string: `gh:owner/repo[/subpath][@ref]` or `dir:~/path`.
    pub source: String,
    /// Target platforms (see [`SkillPlatforms`]).
    pub platforms: SkillPlatforms,
    /// Deploy method. Defaults to `symlink`.
    #[serde(default)]
    pub deploy: DeployMethod,
}

impl SkillDeclaration {
    /// Resolve which active platforms this skill targets.
    ///
    /// - `"all"` → all active platforms **except** `cross-client`
    /// - `"cross-client"` → only the `cross-client` platform (if active)
    /// - `["id1", "id2"]` → the listed platform IDs (filtered to active)
    pub fn resolve_platforms<'a>(&self, active: &'a [PlatformPlugin]) -> Vec<&'a PlatformPlugin> {
        match &self.platforms {
            SkillPlatforms::Named(s) if s == "all" => {
                active.iter().filter(|p| p.id != "cross-client").collect()
            }
            SkillPlatforms::Named(s) if s == "cross-client" => {
                active.iter().filter(|p| p.id == "cross-client").collect()
            }
            SkillPlatforms::Named(_) => vec![],
            SkillPlatforms::List(ids) => ids
                .iter()
                .filter_map(|id| active.iter().find(|p| p.id == id.as_str()))
                .collect(),
        }
    }
}

// ─── Config ───────────────────────────────────────────────────────────────────

/// The full contents of `ai/skills.toml`.
#[derive(Debug, Deserialize, Default)]
pub struct SkillsConfig {
    #[serde(default, rename = "skill")]
    pub skills: Vec<SkillDeclaration>,
}

impl SkillsConfig {
    /// Load `ai/skills.toml` from `repo_root`.
    /// Returns `Ok(None)` if the file doesn't exist.
    /// Returns an error if any two skills share a name.
    pub fn load(repo_root: &Path) -> Result<Option<Self>> {
        let path = repo_root.join("ai").join("skills.toml");
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("Cannot read {}", path.display()))?;
        let config: Self = toml::from_str(&text)
            .with_context(|| format!("Invalid TOML in {}", path.display()))?;

        // Validate: skill names must be unique within this file.
        let mut seen = HashSet::new();
        for skill in &config.skills {
            if !seen.insert(skill.name.as_str()) {
                anyhow::bail!(
                    "Duplicate skill name '{}' in ai/skills.toml. \
                     Each [[skill]] entry must have a unique name.",
                    skill.name
                );
            }
        }

        Ok(Some(config))
    }
}

// ─── Deploy ───────────────────────────────────────────────────────────────────

/// Deploy a skill from `skill_path` to `target`.
///
/// For `Symlink`: creates an absolute symlink `target → skill_path`.
/// For `Copy`: copies the skill directory tree to `target`.
///
/// If `target` already exists and its path is NOT in `owned_targets`, emits a
/// warning and returns `Ok(false)` — the skill is skipped to avoid clobbering
/// files not managed by dfiles.
///
/// Returns `Ok(true)` on successful deploy, `Ok(false)` on skip.
pub fn deploy_skill(
    skill_path: &Path,
    target: &Path,
    method: &DeployMethod,
    owned_targets: &HashSet<PathBuf>,
) -> Result<bool> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create parent dir {}", parent.display()))?;
    }

    // Collision check: exists but not owned by dfiles → warn + skip.
    if (target.exists() || target.is_symlink()) && !owned_targets.contains(target) {
        eprintln!(
            "warning: skill target '{}' already exists and is not managed by dfiles — skipping.\n\
             Remove it manually or run `dfiles ai remove-skill` to take ownership.",
            target.display()
        );
        return Ok(false);
    }

    // Remove existing managed entry before replacing.
    if target.is_symlink() || target.exists() {
        if target.is_dir() && !target.is_symlink() {
            std::fs::remove_dir_all(target)
                .with_context(|| format!("Cannot remove dir {}", target.display()))?;
        } else {
            std::fs::remove_file(target)
                .with_context(|| format!("Cannot remove {}", target.display()))?;
        }
    }

    match method {
        DeployMethod::Symlink => {
            #[cfg(unix)]
            std::os::unix::fs::symlink(skill_path, target).with_context(|| {
                format!(
                    "Cannot create symlink {} → {}",
                    target.display(),
                    skill_path.display()
                )
            })?;
            #[cfg(not(unix))]
            anyhow::bail!("Symlink deploy is not supported on non-Unix platforms");
        }
        DeployMethod::Copy => {
            copy_dir(skill_path, target)?;
        }
    }

    Ok(true)
}

/// Recursively copy `src` directory to `dest`.
fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in walkdir::WalkDir::new(src).min_depth(1) {
        let entry = entry.context("Error walking skill directory")?;
        let rel = entry.path().strip_prefix(src)?;
        let dest_path = dest.join(rel);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest_path)?;
        } else {
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &dest_path).with_context(|| {
                format!(
                    "Cannot copy {} → {}",
                    entry.path().display(),
                    dest_path.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_skills_toml(dir: &TempDir, content: &str) {
        let ai_dir = dir.path().join("ai");
        std::fs::create_dir_all(&ai_dir).unwrap();
        std::fs::write(ai_dir.join("skills.toml"), content).unwrap();
    }

    // ── SkillSource::parse ───────────────────────────────────────────────────

    #[test]
    fn parses_gh_source() {
        let src = SkillSource::parse("gh:anthropics/skills/pdf-processing").unwrap();
        assert!(matches!(src, SkillSource::Gh(_)));
    }

    #[test]
    fn parses_dir_source() {
        let src = SkillSource::parse("dir:/tmp/my-skill").unwrap();
        match src {
            SkillSource::Dir(p) => assert_eq!(p, PathBuf::from("/tmp/my-skill")),
            _ => panic!("expected Dir"),
        }
    }

    #[test]
    fn rejects_unknown_prefix() {
        assert!(SkillSource::parse("http://example.com/skill").is_err());
        assert!(SkillSource::parse("my-skill").is_err());
    }

    // ── SkillsConfig::load ───────────────────────────────────────────────────

    #[test]
    fn returns_none_when_file_absent() {
        let dir = TempDir::new().unwrap();
        assert!(SkillsConfig::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn parses_skill_declarations() {
        let dir = TempDir::new().unwrap();
        write_skills_toml(
            &dir,
            r#"
[[skill]]
name     = "pdf-processing"
source   = "gh:anthropics/skills/pdf-processing"
platforms = ["claude-code"]

[[skill]]
name     = "find-skills"
source   = "gh:vercel-labs/skills/find-skills"
platforms = "all"
deploy   = "copy"
"#,
        );

        let cfg = SkillsConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(cfg.skills.len(), 2);

        let s0 = &cfg.skills[0];
        assert_eq!(s0.name, "pdf-processing");
        assert_eq!(s0.deploy, DeployMethod::Symlink); // default

        let s1 = &cfg.skills[1];
        assert_eq!(s1.name, "find-skills");
        assert_eq!(s1.deploy, DeployMethod::Copy);
    }

    #[test]
    fn duplicate_skill_name_is_error() {
        let dir = TempDir::new().unwrap();
        write_skills_toml(
            &dir,
            r#"
[[skill]]
name     = "my-skill"
source   = "gh:owner/repo"
platforms = "all"

[[skill]]
name     = "my-skill"
source   = "gh:owner/other-repo"
platforms = "all"
"#,
        );

        let err = SkillsConfig::load(dir.path()).unwrap_err();
        let msg = format!("{:#}", err);
        assert!(msg.contains("Duplicate skill name 'my-skill'"), "error was: {}", msg);
    }

    // ── resolve_platforms ────────────────────────────────────────────────────

    fn make_active(ids: &[&str]) -> Vec<PlatformPlugin> {
        ids.iter()
            .map(|id| PlatformPlugin {
                id: id.to_string(),
                name: id.to_string(),
                config_dir: None,
                skills_dir: PathBuf::from(format!("/fake/{}/skills", id)),
                config_file: None,
                binary: None,
                agentskills_compliant: false,
            })
            .collect()
    }

    #[test]
    fn all_excludes_cross_client() {
        let active = make_active(&["claude-code", "codex", "cross-client"]);
        let decl = SkillDeclaration {
            name: "s".into(),
            source: "gh:a/b".into(),
            platforms: SkillPlatforms::Named("all".into()),
            deploy: DeployMethod::Symlink,
        };
        let targets = decl.resolve_platforms(&active);
        let ids: Vec<&str> = targets.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"claude-code"));
        assert!(ids.contains(&"codex"));
        assert!(!ids.contains(&"cross-client"), "cross-client must be excluded from 'all'");
    }

    #[test]
    fn cross_client_targets_only_cross_client() {
        let active = make_active(&["claude-code", "cross-client"]);
        let decl = SkillDeclaration {
            name: "s".into(),
            source: "gh:a/b".into(),
            platforms: SkillPlatforms::Named("cross-client".into()),
            deploy: DeployMethod::Symlink,
        };
        let targets = decl.resolve_platforms(&active);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "cross-client");
    }

    #[test]
    fn list_filters_to_active_platforms() {
        let active = make_active(&["claude-code", "codex"]);
        let decl = SkillDeclaration {
            name: "s".into(),
            source: "gh:a/b".into(),
            // "cursor" is not active — should be excluded.
            platforms: SkillPlatforms::List(vec![
                "claude-code".into(),
                "cursor".into(),
            ]),
            deploy: DeployMethod::Symlink,
        };
        let targets = decl.resolve_platforms(&active);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "claude-code");
    }

    // ── deploy_skill ─────────────────────────────────────────────────────────

    fn make_skill_dir(dir: &TempDir) -> PathBuf {
        let skill = dir.path().join("cached-skill");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "---\nname: test\n---\n").unwrap();
        skill
    }

    #[test]
    fn deploy_symlink_creates_link() {
        let dir = TempDir::new().unwrap();
        let skill_path = make_skill_dir(&dir);
        let target = dir.path().join("skills").join("test-skill");
        let owned: HashSet<PathBuf> = HashSet::new();

        let deployed = deploy_skill(&skill_path, &target, &DeployMethod::Symlink, &owned).unwrap();
        assert!(deployed);
        assert!(target.is_symlink());

        let link_target = std::fs::read_link(&target).unwrap();
        assert_eq!(link_target, skill_path);
    }

    #[test]
    fn deploy_copy_creates_directory() {
        let dir = TempDir::new().unwrap();
        let skill_path = make_skill_dir(&dir);
        let target = dir.path().join("skills").join("test-skill");
        let owned: HashSet<PathBuf> = HashSet::new();

        let deployed = deploy_skill(&skill_path, &target, &DeployMethod::Copy, &owned).unwrap();
        assert!(deployed);
        assert!(target.is_dir());
        assert!(target.join("SKILL.md").exists());
    }

    #[test]
    fn deploy_warns_and_skips_unmanaged_collision() {
        let dir = TempDir::new().unwrap();
        let skill_path = make_skill_dir(&dir);
        let target = dir.path().join("skills").join("test-skill");

        // Create an unmanaged directory at the target.
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("README.md"), "# pre-existing").unwrap();

        let owned: HashSet<PathBuf> = HashSet::new(); // target NOT in owned

        let deployed =
            deploy_skill(&skill_path, &target, &DeployMethod::Symlink, &owned).unwrap();
        assert!(!deployed, "should skip unmanaged collision");
        // Pre-existing file must not be touched.
        assert!(target.join("README.md").exists());
    }

    #[test]
    fn deploy_replaces_managed_entry() {
        let dir = TempDir::new().unwrap();
        let skill_path = make_skill_dir(&dir);
        let target = dir.path().join("skills").join("test-skill");
        std::fs::create_dir_all(&target).unwrap();

        // Mark target as owned by dfiles.
        let mut owned = HashSet::new();
        owned.insert(target.clone());

        let deployed = deploy_skill(&skill_path, &target, &DeployMethod::Symlink, &owned).unwrap();
        assert!(deployed);
        assert!(target.is_symlink());
    }
}
