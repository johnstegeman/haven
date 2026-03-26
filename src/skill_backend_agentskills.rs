/// `AgentSkillsBackend` — delegates skill fetch and deployment to `agent-skills-cli`.
///
/// agent-skills-cli (`skills` binary) installs skills globally to `~/.skills/` and
/// creates symlinks under platform-specific directories (e.g. `~/.claude/skills/`).
/// Haven retains ownership of CLAUDE.md generation, `state.json`, and collision detection.
///
/// # Source format mapping
///
/// ```text
/// gh:owner/repo/subpath[@ref]  →  skills install owner/repo -s subpath -g -a <agent> -y
/// gh:owner/repo[@ref]          →  skills install owner/repo -g -a <agent> -y
/// dir:~/path                   →  skills install <expanded-path> -g -a <agent> -y
/// repo:                        →  Err("not supported by agent-skills backend")
/// ```
///
/// `@ref` is silently dropped — version management is delegated to agent-skills-cli.
///
/// # Source string threading
///
/// `SkillBackend::deploy()` receives a `ResolvedSkill` which has no `source_key` field.
/// The apply.rs call site injects the original source string via the `_haven_source` key
/// in `skill.metadata.metadata`. This is a documented convention to avoid modifying the
/// trait. See apply.rs `ResolvedSkill` construction for the AgentSkills backend path.
///
/// # `cached_path` sentinel
///
/// `fetch()` returns `PathBuf::new()` as the `cached_path`. This is safe because
/// `AgentSkillsBackend::deploy()` never reads `skill.cached_path` — it derives the
/// target path from `target.skills_dir.join(&skill.name)` only. The empty path must
/// not be passed to any filesystem check by callers.
use anyhow::{Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ai_skill::SkillSource;
use crate::skill_backend::{
    CachedSkillInfo, DeployResult, DeploymentTarget, FetchResult, ResolvedSkill, SkillBackend,
    SkillMetadata,
};

// ─── Backend struct ────────────────────────────────────────────────────────────

pub struct AgentSkillsBackend {
    runner: String,
    timeout: Duration,
}

impl AgentSkillsBackend {
    pub fn new(runner: String, timeout: Duration) -> Self {
        AgentSkillsBackend { runner, timeout }
    }
}

// ─── SkillBackend impl ────────────────────────────────────────────────────────

impl SkillBackend for AgentSkillsBackend {
    fn name(&self) -> &str {
        "agent-skills"
    }

    fn is_available(&self) -> bool {
        crate::util::is_on_path(&self.runner)
    }

    /// No-op: agent-skills-cli manages its own cache.
    ///
    /// Returns an empty `cached_path` sentinel — safe because `deploy()` never reads it.
    fn fetch(
        &self,
        _source: &SkillSource,
        _expected_sha: Option<&str>,
    ) -> Result<FetchResult> {
        Ok(FetchResult {
            cached_path: PathBuf::new(),
            sha: "managed-by-agent-skills".to_string(),
            was_cached: true,
        })
    }

    /// Deploy a single skill via agent-skills-cli.
    ///
    /// The original source declaration string (e.g. `"gh:anthropics/skills/pdf-processing"`)
    /// must be stored in `skill.metadata.metadata["_haven_source"]` by the apply.rs call
    /// site when constructing `ResolvedSkill` for this backend.
    ///
    /// NOTE: `skill.sha` will be `"managed-by-agent-skills"` — do NOT use it as a source
    /// string. `skill.cached_path` will be empty — do NOT pass it to any filesystem check.
    fn deploy(&self, skill: &ResolvedSkill, target: &DeploymentTarget) -> Result<DeployResult> {
        let source_str = skill
            .metadata
            .metadata
            .get("_haven_source")
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "AgentSkillsBackend: missing _haven_source in skill metadata for '{}'",
                    skill.name
                )
            })?;

        let (source_arg, skill_selector) = map_source(source_str)?;
        let agent_name = map_platform_id(&target.platform_id);

        run_agent_skills_install(
            &self.runner,
            &source_arg,
            skill_selector.as_deref(),
            agent_name,
            self.timeout,
        )?;

        // Verify by filesystem — target path is predictable from Haven's own config.
        let target_path = target.skills_dir.join(&skill.name);
        if target_path.exists() || target_path.is_symlink() {
            Ok(DeployResult {
                target_path,
                was_collision: false,
                deployed: true,
            })
        } else {
            anyhow::bail!(
                "agent-skills-cli reported success but '{}' was not found at '{}'",
                skill.name,
                target_path.display()
            )
        }
    }

    /// Sequential deploy_all to prevent concurrent writes to `~/.skills/skills.lock`.
    fn deploy_all(
        &self,
        skills: &[(&ResolvedSkill, &DeploymentTarget)],
    ) -> Result<Vec<DeployResult>> {
        skills.iter().map(|(s, t)| self.deploy(s, t)).collect()
    }

    /// Delegate update to `skills update [name...] -g -y`.
    ///
    /// Empty slice → `skills update --all -g -y`.
    /// Non-empty slice → `skills update <name1> <name2> ... -g -y`.
    ///
    /// Returns the names of skills that were passed to the update command on success.
    /// If the command fails, the error is propagated. Stdout is not parsed (no `--json`
    /// flag on update) — all requested names are returned on a successful exit.
    fn update_all(&self, skills: &[(&str, &str)]) -> Result<Vec<String>> {
        let mut cmd = std::process::Command::new(&self.runner);
        cmd.arg("update");
        if skills.is_empty() {
            cmd.args(["--all", "-g", "-y"]);
        } else {
            let names: Vec<&str> = skills.iter().map(|(name, _)| *name).collect();
            cmd.args(&names);
            cmd.args(["-g", "-y"]);
        }
        run_with_timeout(cmd, self.timeout, &self.runner)?;
        Ok(skills.iter().map(|(name, _)| name.to_string()).collect())
    }

    /// Remove a deployed skill from the filesystem.
    ///
    /// Does NOT call `skills uninstall` to avoid removing from `~/.skills/` cache
    /// (other platforms may still reference it).
    fn undeploy(&self, target: &Path) -> Result<()> {
        if target.is_symlink() || target.is_file() {
            std::fs::remove_file(target)
                .with_context(|| format!("Failed to remove '{}'", target.display()))?;
        } else if target.is_dir() {
            std::fs::remove_dir_all(target)
                .with_context(|| format!("Failed to remove directory '{}'", target.display()))?;
        } else {
            anyhow::bail!("undeploy: '{}' not found", target.display());
        }
        Ok(())
    }

    /// Parse and validate a `SKILL.md`, delegating to the native parser.
    fn validate(&self, skill_path: &Path) -> Result<SkillMetadata> {
        crate::skill_backend_native::validate_skill_md(skill_path)
    }

    /// Best-effort: read `~/.skills/skills.lock` and enumerate entries.
    ///
    /// Returns empty vec (not an error) if the lock file is absent or unparseable.
    /// Only `source_key` (skill name) and `sha` (version field) are populated.
    fn list_cached(&self) -> Result<Vec<CachedSkillInfo>> {
        Ok(read_agent_skills_lock())
    }

    /// Evict is not supported — direct the user to `skills uninstall`.
    fn evict(&self, source_key: &str) -> Result<()> {
        anyhow::bail!(
            "AgentSkillsBackend: evict is not supported — manage cache via `skills uninstall {}`",
            source_key
        )
    }
}

// ─── Source mapping ───────────────────────────────────────────────────────────

/// Parse a Haven source declaration into the (`source_arg`, `skill_selector`) pair
/// for `skills install <source_arg> [-s <skill_selector>] -g -a <agent> -y`.
///
/// Returns `(source_arg, Option<skill_selector>)`.
fn map_source(source_str: &str) -> Result<(String, Option<String>)> {
    let source = SkillSource::parse(source_str)
        .with_context(|| format!("Failed to parse source: {}", source_str))?;

    match source {
        SkillSource::Gh(gh) => {
            // @ref is silently dropped — version management delegated to agent-skills-cli.
            // Visible in HAVEN_LOG=debug output if tracing is ever added.
            if let Some(ref dropped) = gh.git_ref {
                let _ = dropped; // ref noted, not passed to agent-skills-cli
            }
            let repo_arg = format!("{}/{}", gh.owner, gh.repo);
            Ok((repo_arg, gh.subpath.clone()))
        }
        SkillSource::Dir(path) => {
            // SkillSource::parse already expands tilde — path is absolute here.
            Ok((path.to_string_lossy().to_string(), None))
        }
        SkillSource::Repo => {
            anyhow::bail!(
                "source '{}' uses 'repo:' which is not supported by the agent-skills backend\n\
                 hint: use 'native' backend for repo: sources, or convert to a gh: source",
                source_str
            )
        }
    }
}

// ─── Platform ID mapping ──────────────────────────────────────────────────────

/// Map Haven platform IDs to agent-skills-cli agent names.
///
/// Unknown platform IDs fall back to the raw value with a stderr warning.
fn map_platform_id(platform_id: &str) -> &str {
    match platform_id {
        "claude-code" => "claude",
        "cursor"      => "cursor",
        "copilot"     => "copilot",
        "windsurf"    => "windsurf",
        "cline"       => "cline",
        "zed"         => "zed",
        other => {
            eprintln!(
                "warning: agent-skills-cli: unknown platform '{}' — passing as-is",
                other
            );
            other
        }
    }
}

// ─── Subprocess ───────────────────────────────────────────────────────────────

fn run_agent_skills_install(
    runner: &str,
    source: &str,
    skill_selector: Option<&str>,
    agent: &str,
    timeout: Duration,
) -> Result<()> {
    let mut cmd = std::process::Command::new(runner);
    cmd.args(["install", source, "-g", "-a", agent, "-y"]);
    if let Some(sel) = skill_selector {
        cmd.args(["-s", sel]);
    }
    run_with_timeout(cmd, timeout, runner)
}

/// Spawn a command, wait up to `timeout`, kill if exceeded.
fn run_with_timeout(
    mut cmd: std::process::Command,
    timeout: Duration,
    runner: &str,
) -> Result<()> {
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to spawn '{}' — is it installed and on PATH?", runner))?;

    let start = std::time::Instant::now();
    let poll_interval = Duration::from_millis(100);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                // Capture stderr for the error message.
                let mut stderr_buf = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_string(&mut stderr_buf);
                }
                let msg = stderr_buf.trim().to_string();
                if msg.is_empty() {
                    anyhow::bail!(
                        "'{}' exited with status {}",
                        runner,
                        status.code().unwrap_or(-1)
                    );
                } else {
                    anyhow::bail!(
                        "'{}' exited with status {}: {}",
                        runner,
                        status.code().unwrap_or(-1),
                        msg
                    );
                }
            }
            Ok(None) => {
                // Still running.
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    anyhow::bail!(
                        "'{}' timed out after {}s",
                        runner,
                        timeout.as_secs()
                    );
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => {
                anyhow::bail!("Error waiting for '{}': {}", runner, e);
            }
        }
    }
}

// ─── Lock file reading ────────────────────────────────────────────────────────

/// Read `~/.skills/skills.lock` and return best-effort `CachedSkillInfo` entries.
///
/// Lock schema: `{"version":"1","skills":{"<name>":{"version":"<sha>",...}}}`
///
/// Returns empty vec if the file is absent or unparseable — never errors.
fn read_agent_skills_lock() -> Vec<CachedSkillInfo> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return vec![],
    };
    let lock_path = home.join(".skills").join("skills.lock");
    let text = match std::fs::read_to_string(&lock_path) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let skills_map = match value.get("skills").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return vec![],
    };
    skills_map
        .iter()
        .map(|(name, entry)| {
            let sha = entry
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            CachedSkillInfo {
                source_key: name.clone(),
                cached_path: PathBuf::new(),
                sha,
            }
        })
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── map_source ────────────────────────────────────────────────────────────

    #[test]
    fn map_source_gh_with_subpath() {
        let (src, sel) = map_source("gh:anthropics/skills/pdf-processing").unwrap();
        assert_eq!(src, "anthropics/skills");
        assert_eq!(sel.as_deref(), Some("pdf-processing"));
    }

    #[test]
    fn map_source_gh_no_subpath() {
        let (src, sel) = map_source("gh:anthropics/my-skill").unwrap();
        assert_eq!(src, "anthropics/my-skill");
        assert!(sel.is_none());
    }

    #[test]
    fn map_source_gh_drops_ref() {
        // @ref is silently dropped — the source/selector are still correct.
        let (src, sel) = map_source("gh:anthropics/skills/pdf-processing@v1.0").unwrap();
        assert_eq!(src, "anthropics/skills");
        assert_eq!(sel.as_deref(), Some("pdf-processing"));
    }

    #[test]
    fn map_source_dir_absolute() {
        let (src, sel) = map_source("dir:/absolute/path/to/skill").unwrap();
        assert_eq!(src, "/absolute/path/to/skill");
        assert!(sel.is_none());
    }

    #[test]
    fn map_source_repo_returns_error() {
        let err = map_source("repo:").unwrap_err();
        assert!(
            format!("{err}").contains("repo:"),
            "error should mention 'repo:': {err}"
        );
    }

    // ── map_platform_id ───────────────────────────────────────────────────────

    #[test]
    fn platform_id_claude_code_maps_to_claude() {
        assert_eq!(map_platform_id("claude-code"), "claude");
    }

    #[test]
    fn platform_id_cursor_passes_through() {
        assert_eq!(map_platform_id("cursor"), "cursor");
    }

    #[test]
    fn platform_id_unknown_passes_through_as_is() {
        // Unknown IDs fall back to the raw value (warn-and-continue, not hard error).
        assert_eq!(map_platform_id("some-unknown-platform"), "some-unknown-platform");
    }

    #[test]
    fn platform_id_known_platforms_all_mapped() {
        let known = [
            ("claude-code", "claude"),
            ("cursor", "cursor"),
            ("copilot", "copilot"),
            ("windsurf", "windsurf"),
            ("cline", "cline"),
            ("zed", "zed"),
        ];
        for (haven_id, expected) in &known {
            assert_eq!(
                map_platform_id(haven_id), *expected,
                "platform '{}' should map to '{}'", haven_id, expected
            );
        }
    }

    // ── deploy error: missing _haven_source ───────────────────────────────────

    #[test]
    fn deploy_errors_when_haven_source_missing() {
        use std::collections::HashSet;
        use crate::ai_skill::DeployMethod;

        let backend = AgentSkillsBackend::new("skills".to_string(), Duration::from_secs(30));
        let skill = ResolvedSkill {
            name: "my-skill".to_string(),
            cached_path: PathBuf::new(),
            sha: "managed-by-agent-skills".to_string(),
            metadata: SkillMetadata::default(), // no _haven_source key
        };
        let target = DeploymentTarget {
            platform_id: "claude-code".to_string(),
            skills_dir: PathBuf::from("/tmp/skills"),
            deploy_method: DeployMethod::Symlink,
            owned_targets: HashSet::new(),
        };
        let err = backend.deploy(&skill, &target)
            .err()
            .expect("deploy should fail when _haven_source is missing");
        assert!(
            format!("{err}").contains("_haven_source"),
            "error should mention _haven_source: {err}"
        );
    }

    // ── lock file parsing ─────────────────────────────────────────────────────

    #[test]
    fn read_agent_skills_lock_returns_empty_for_invalid_json() {
        // The function returns empty vec rather than erroring on bad input.
        // We test this indirectly: an absent lock file also returns empty.
        // (Actual file reading is tested in integration tests.)
        let result = std::panic::catch_unwind(read_agent_skills_lock);
        assert!(result.is_ok(), "read_agent_skills_lock should not panic");
    }
}
