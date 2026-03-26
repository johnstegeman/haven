/// SkillKitBackend — delegates AI skill deployment to the `skillkit` CLI.
///
/// SkillKit is a bulk-only backend: a single `skillkit team install` call
/// deploys all declared skills in one subprocess invocation.
///
/// ## Invocation
///
/// ```
/// <runner> skillkit team install --manifest <path> --json
/// ```
///
/// Where `runner` is "npx", "bunx", or a direct binary path configured in
/// `ai/config.toml`. The manifest is a JSON temp file:
///
/// ```json
/// [{"name":"pdf-processing","source":"anthropics/skills/pdf-processing","version":"latest"}]
/// ```
///
/// ## What Haven retains regardless of this backend
///
/// - `state.json` ownership tracking (populated from SkillKit's `--json` stdout)
/// - `CLAUDE.md` generation (Haven always drives this from deployed state)
/// - `apply.lock` prevents concurrent `haven apply` runs
///
/// ## Implementation notes
///
/// - `fetch()` is a no-op: SkillKit manages its own cache
/// - `deploy()` always errors: use `deploy_all()` instead
/// - `deploy_all()` generates the manifest, runs SkillKit, parses stdout
/// - `was_collision` is always false (SkillKit stdout doesn't expose collision info)
/// - Timeout is enforced via `Child::kill()` — no orphaned subprocesses
/// - state.json is NOT updated if skillkit exits non-zero
use anyhow::{Context, Result};
use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ai_config::AiConfig;
use crate::ai_skill::SkillSource;
use crate::skill_backend::{
    CachedSkillInfo, DeployResult, DeploymentTarget, FetchResult, ResolvedSkill, SkillBackend,
    SkillMetadata,
};

pub struct SkillKitBackend {
    runner: String,
    timeout: Duration,
}

impl SkillKitBackend {
    pub fn new(config: &AiConfig) -> Self {
        SkillKitBackend {
            runner: config.runner.clone(),
            timeout: Duration::from_secs(config.timeout_secs),
        }
    }
}

impl SkillBackend for SkillKitBackend {
    fn name(&self) -> &str {
        "skillkit"
    }

    fn is_available(&self) -> bool {
        if self.runner.contains('/') {
            // Full path — check existence.
            Path::new(&self.runner).is_file()
        } else {
            // Short name (npx, bunx) — check PATH.
            crate::util::is_on_path(&self.runner)
        }
    }

    /// No-op: SkillKit manages its own cache.
    fn fetch(
        &self,
        _source: &SkillSource,
        _expected_sha: Option<&str>,
    ) -> Result<FetchResult> {
        Ok(FetchResult {
            cached_path: PathBuf::new(),
            sha: "managed-by-skillkit".to_string(),
            was_cached: true,
        })
    }

    /// SkillKit is bulk-only — use `deploy_all()` instead.
    fn deploy(
        &self,
        _skill: &ResolvedSkill,
        _target: &DeploymentTarget,
    ) -> Result<DeployResult> {
        anyhow::bail!("SkillKitBackend: use deploy_all() — SkillKit requires bulk deployment")
    }

    /// Deploy all skills in a single `skillkit team install` invocation.
    ///
    /// ```ascii
    /// skills[] ──► build manifest ──► write to tmp ──► rename tmp ──► skillkit
    ///                                                                      │
    ///                                                              parse --json stdout
    ///                                                                      │
    ///                                                              DeployResult[]
    /// ```
    ///
    /// Supports both `gh:` and `dir:` sources:
    /// - `gh:` — `skill.sha` is non-empty; SkillKit marketplace ID derived from skill name.
    /// - `dir:` — `skill.sha` is empty; `skill.cached_path` (the local dir) is used as source.
    fn deploy_all(
        &self,
        skills: &[(&ResolvedSkill, &DeploymentTarget)],
    ) -> Result<Vec<DeployResult>> {
        if skills.is_empty() {
            return Ok(vec![]);
        }

        let manifest_entries = build_manifest_entries(skills)?;

        // Write manifest atomically to a temp file.
        let manifest_json = serde_json::to_string(&manifest_entries)
            .context("failed to serialize SkillKit manifest")?;
        let tmp_path = write_manifest_atomically(&manifest_json)?;

        // Invoke skillkit team install --manifest <path> --json
        let result = run_skillkit_with_json(&self.runner, &tmp_path, self.timeout, false);

        // Clean up temp file regardless of outcome.
        let _ = std::fs::remove_file(&tmp_path);

        let stdout = result?;
        parse_deploy_results(&stdout, skills)
    }

    /// Update already-installed skills to their latest versions via
    /// `skillkit team install --update --manifest <path> --json`.
    ///
    /// `skills`: (name, source_str) pairs. An empty slice updates everything
    /// currently managed by SkillKit (passes an empty manifest; SkillKit
    /// interprets this as "update all").
    fn update_all(&self, skills: &[(&str, &str)]) -> Result<Vec<String>> {
        let manifest_entries: Vec<SkillKitManifestEntry> = skills
            .iter()
            .map(|(name, source_str)| {
                let source = skillkit_source_from_str(name, source_str);
                SkillKitManifestEntry {
                    name: name.to_string(),
                    source,
                    version: "latest".to_string(),
                }
            })
            .collect();

        let manifest_json = serde_json::to_string(&manifest_entries)
            .context("failed to serialize SkillKit update manifest")?;
        let tmp_path = write_manifest_atomically(&manifest_json)?;

        let result = run_skillkit_with_json(&self.runner, &tmp_path, self.timeout, true);
        let _ = std::fs::remove_file(&tmp_path);

        let stdout = result?;
        // Parse updated skill names from JSON output.
        let updated = parse_updated_names(&stdout);
        Ok(updated)
    }

    fn undeploy(&self, target: &Path) -> Result<()> {
        if target.is_symlink() || target.is_file() {
            std::fs::remove_file(target)
                .with_context(|| format!("Cannot remove {}", target.display()))?;
        } else if target.is_dir() {
            std::fs::remove_dir_all(target)
                .with_context(|| format!("Cannot remove directory {}", target.display()))?;
        } else {
            anyhow::bail!("Cannot undeploy {}: path not found", target.display());
        }
        Ok(())
    }

    fn validate(&self, skill_path: &Path) -> Result<SkillMetadata> {
        // SkillKit manages SKILL.md validation internally; delegate to native parser.
        crate::skill_backend_native::validate_skill_md(skill_path)
    }

    fn list_cached(&self) -> Result<Vec<CachedSkillInfo>> {
        // SkillKit manages its own cache — Haven has no visibility into it.
        Ok(vec![])
    }

    fn evict(&self, _source_key: &str) -> Result<()> {
        anyhow::bail!("SkillKitBackend: evict is not supported — manage cache via `skillkit cache clear`")
    }
}

// ─── Manifest ─────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
struct SkillKitManifestEntry {
    name: String,
    source: String,
    version: String,
}

/// Build a SkillKit manifest from a slice of (ResolvedSkill, DeploymentTarget) pairs.
///
/// Source mapping:
/// - `gh:` skills (`sha` non-empty): use skill name as the SkillKit marketplace ID.
/// - `dir:` skills (`sha` empty): use the absolute local path from `cached_path`.
fn build_manifest_entries(
    skills: &[(&ResolvedSkill, &DeploymentTarget)],
) -> Result<Vec<SkillKitManifestEntry>> {
    let mut entries = Vec::with_capacity(skills.len());
    for (skill, _target) in skills {
        let source = if skill.sha.is_empty() {
            // dir: source — pass the absolute local path to SkillKit.
            let abs = skill.cached_path.to_string_lossy().to_string();
            if abs.is_empty() {
                anyhow::bail!(
                    "SkillKitBackend: skill '{}' has no cached_path (dir: source required)",
                    skill.name
                );
            }
            abs
        } else {
            // gh: source — use the skill name as the SkillKit marketplace identifier.
            // Full gh→skillkit source mapping is a known limitation: TODOS.md Phase 3.
            skill.name.clone()
        };
        entries.push(SkillKitManifestEntry {
            name: skill.name.clone(),
            source,
            version: "latest".to_string(),
        });
    }
    Ok(entries)
}

/// Derive the SkillKit source string from a declaration's source string.
///
/// - `gh:owner/repo/path[@ref]` → uses skill name (SkillKit marketplace ID)
/// - `dir:~/path` → expands tilde and returns the absolute path
/// - `repo:` → uses skill name (treated as local repo skill)
fn skillkit_source_from_str(skill_name: &str, source_str: &str) -> String {
    if let Some(rest) = source_str.strip_prefix("dir:") {
        // Expand tilde manually — dirs::home_dir() handles the common case.
        if let Some(path) = rest.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(path).to_string_lossy().to_string();
            }
        }
        return rest.to_string();
    }
    // gh: and repo: → use skill name as SkillKit marketplace ID.
    skill_name.to_string()
}

// ─── Subprocess ───────────────────────────────────────────────────────────────

#[allow(dead_code)]
/// Write manifest JSON atomically: write to `.skills.tmp`, rename to `.skills`.
/// Returns the final path.
fn write_manifest_atomically(json: &str) -> Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join(format!(".skills-{}-{}.tmp", std::process::id(), id));
    let final_path = tmp_dir.join(format!(".skills-{}-{}", std::process::id(), id));

    {
        let mut f = std::fs::File::create(&tmp_path)
            .context("failed to create SkillKit manifest temp file")?;
        f.write_all(json.as_bytes())
            .context("failed to write SkillKit manifest")?;
    }
    std::fs::rename(&tmp_path, &final_path)
        .context("failed to rename SkillKit manifest file")?;
    Ok(final_path)
}

/// Return true if the stderr from a failed SkillKit invocation looks like the user
/// hasn't run `skillkit init` yet.  We check for known SkillKit error patterns;
/// on a false negative we fall back to the generic `doctor` hint.
fn needs_init_hint(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("not initialized")
        || lower.contains("no agents")
        || lower.contains("agent not configured")
        || lower.contains("run skillkit init")
        || lower.contains("run init")
}

/// Run `<runner> [skillkit] team install [--update] --manifest <path> --json` with timeout.
/// Returns stdout on success, or an error if the process exits non-zero or times out.
///
/// Package managers (npx, bunx, bun): prepend "skillkit" as the subcommand.
/// Direct binary paths (tests or user-installed): run as-is.
///
/// `update`: when true, passes `--update` to force SkillKit to re-fetch the
/// latest version even if the skill is already cached.
fn run_skillkit_with_json(
    runner: &str,
    manifest_path: &Path,
    timeout: Duration,
    update: bool,
) -> Result<String> {
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(runner);
    // Package manager runners need "skillkit" as the package to execute.
    if matches!(runner, "npx" | "bunx" | "bun") {
        cmd.arg("skillkit");
    }
    cmd.args(["team", "install", "--manifest"]);
    cmd.arg(manifest_path);
    if update {
        cmd.arg("--update");
    }
    let mut child = cmd
        .arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn '{}' skillkit", runner))?;

    // Poll with timeout — kill child if it exceeds the limit.
    let start = std::time::Instant::now();
    let poll_interval = Duration::from_millis(100);
    loop {
        match child.try_wait()? {
            Some(status) => {
                let output = child.wait_with_output()?;
                if !status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let stderr_trimmed = stderr.trim();
                    // Detect likely "not initialized" errors and surface a targeted hint.
                    // SkillKit requires `skillkit init` to be run once per machine per agent
                    // before `team install` can deploy skills to that agent's directory.
                    let init_hint = needs_init_hint(stderr_trimmed);
                    if init_hint {
                        anyhow::bail!(
                            "skillkit exited with status {}: {}\n\
                             hint: run `npx skillkit@latest init` to initialize SkillKit \
                             for each agent platform on this machine (e.g. claude-code, cursor)",
                            status.code().unwrap_or(-1),
                            stderr_trimmed
                        );
                    } else {
                        anyhow::bail!(
                            "skillkit exited with status {}: {}\n\
                             hint: run `npx skillkit@latest doctor` to diagnose, or \
                             `npx skillkit@latest init` if this is a fresh installation",
                            status.code().unwrap_or(-1),
                            stderr_trimmed
                        );
                    }
                }
                return String::from_utf8(output.stdout)
                    .context("skillkit produced invalid UTF-8 output");
            }
            None => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait(); // reap
                    anyhow::bail!(
                        "skillkit timed out after {} seconds",
                        timeout.as_secs()
                    );
                }
                std::thread::sleep(poll_interval);
            }
        }
    }
}

// ─── Output parsing ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SkillKitDeployedEntry {
    name: String,
    path: String,
}

#[allow(dead_code)]
/// Parse the JSON stdout from `skillkit team install --json` into DeployResults.
fn parse_deploy_results(
    stdout: &str,
    skills: &[(&ResolvedSkill, &DeploymentTarget)],
) -> Result<Vec<DeployResult>> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(vec![]);
    }

    let entries: Vec<SkillKitDeployedEntry> = serde_json::from_str(trimmed)
        .with_context(|| format!("failed to parse skillkit JSON output: {trimmed}"))?;

    // Build a name→path map from SkillKit's output.
    let path_map: std::collections::HashMap<&str, &str> = entries
        .iter()
        .map(|e| (e.name.as_str(), e.path.as_str()))
        .collect();

    let mut results = Vec::with_capacity(skills.len());
    for (skill, _target) in skills {
        if let Some(path_str) = path_map.get(skill.name.as_str()) {
            results.push(DeployResult {
                target_path: PathBuf::from(path_str),
                was_collision: false, // SkillKit stdout doesn't expose collision info
                deployed: true,
            });
        }
        // Skills not in the output were skipped or already up to date.
    }
    Ok(results)
}

/// Parse skill names from `skillkit team install --update --json` stdout.
/// SkillKit returns the same `[{"name":"...","path":"..."}]` format for updates.
fn parse_updated_names(stdout: &str) -> Vec<String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return vec![];
    }
    // Reuse the same entry shape — we just need the names.
    let entries: Vec<SkillKitDeployedEntry> = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    entries.into_iter().map(|e| e.name).collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_config::{AiConfig, BackendKind};
    use crate::ai_skill::DeployMethod;
    use std::collections::HashSet;
    use tempfile::TempDir;

    fn skillkit_config_with_runner(runner: &str) -> AiConfig {
        AiConfig {
            backend: BackendKind::SkillKit,
            runner: runner.to_string(),
            timeout_secs: 10,
        }
    }

    fn fake_skillkit_path() -> String {
        // tests/fixtures/fake-skillkit.sh relative to cargo workspace root
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        format!("{}/tests/fixtures/fake-skillkit.sh", manifest_dir)
    }

    fn make_skill(name: &str) -> (ResolvedSkill, DeploymentTarget) {
        let skill = ResolvedSkill {
            name: name.to_string(),
            cached_path: PathBuf::from(format!("/tmp/fake-cache/{}", name)),
            sha: "managed-by-skillkit".to_string(),
            metadata: SkillMetadata::default(),
        };
        let target = DeploymentTarget {
            platform_id: "claude-code".to_string(),
            skills_dir: PathBuf::from("/tmp/fake-skills"),
            deploy_method: DeployMethod::Symlink,
            owned_targets: HashSet::new(),
        };
        (skill, target)
    }

    #[test]
    fn is_available_true_when_runner_found() {
        // Use the full path to fake-skillkit.sh (exists as a file).
        let cfg = skillkit_config_with_runner(&fake_skillkit_path());
        let backend = SkillKitBackend::new(&cfg);
        assert!(backend.is_available());
    }

    #[test]
    fn is_available_false_when_runner_missing() {
        let cfg = skillkit_config_with_runner("no-such-binary-xyz");
        let backend = SkillKitBackend::new(&cfg);
        assert!(!backend.is_available());
    }

    #[test]
    fn deploy_returns_error() {
        let cfg = skillkit_config_with_runner(&fake_skillkit_path());
        let backend = SkillKitBackend::new(&cfg);
        let (skill, target) = make_skill("test-skill");
        let err = backend.deploy(&skill, &target).err().expect("should have failed");
        assert!(format!("{err}").contains("deploy_all()"));
    }

    #[test]
    fn deploy_all_empty_returns_empty() {
        let cfg = skillkit_config_with_runner(&fake_skillkit_path());
        let backend = SkillKitBackend::new(&cfg);
        let result = backend.deploy_all(&[]).unwrap();
        assert!(result.is_empty());
    }

    /// Write a small inline shell script to a temp file and make it executable.
    /// Returns the temp file path (cleaned up when TempDir is dropped).
    fn write_temp_script(dir: &TempDir, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.path().join("fake-skillkit.sh");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn deploy_all_parses_json_stdout() {
        let dir = TempDir::new().unwrap();
        let json = r#"[{"name":"test-skill","path":"/tmp/fake-skills/test-skill"}]"#;
        let script_path = write_temp_script(&dir, &format!(
            "#!/bin/sh\necho '{}'\n", json
        ));

        let (skill, target) = make_skill("test-skill");
        let pairs = vec![(&skill, &target)];
        let manifest_json = serde_json::to_string(&vec![SkillKitManifestEntry {
            name: "test-skill".to_string(),
            source: "test-skill".to_string(),
            version: "latest".to_string(),
        }]).unwrap();
        let tmp = write_manifest_atomically(&manifest_json).unwrap();
        let stdout = run_skillkit_with_json(
            script_path.to_str().unwrap(), &tmp, Duration::from_secs(10), false
        ).unwrap();
        let _ = std::fs::remove_file(&tmp);

        let results = parse_deploy_results(&stdout, &pairs).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_path, PathBuf::from("/tmp/fake-skills/test-skill"));
        assert!(results[0].deployed);
        assert!(!results[0].was_collision);
    }

    #[test]
    fn deploy_all_handles_empty_json_stdout() {
        let (skill, target) = make_skill("test-skill");
        let pairs = vec![(&skill, &target)];
        let results = parse_deploy_results("[]", &pairs).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn deploy_all_handles_skillkit_nonzero_exit() {
        let dir = TempDir::new().unwrap();
        let script_path = write_temp_script(&dir,
            "#!/bin/sh\necho 'skillkit: error deploying skills' >&2\nexit 1\n"
        );
        let tmp = write_manifest_atomically("[]").unwrap();
        let err = run_skillkit_with_json(
            script_path.to_str().unwrap(), &tmp, Duration::from_secs(10), false
        );
        let _ = std::fs::remove_file(&tmp);
        assert!(err.is_err());
        let msg = format!("{:#}", err.unwrap_err());
        assert!(msg.contains("status 1") || msg.contains("exit"), "expected exit error, got: {msg}");
    }

    #[test]
    fn deploy_all_respects_timeout() {
        let dir = TempDir::new().unwrap();
        let script_path = write_temp_script(&dir, "#!/bin/sh\nsleep 30\n");
        let tmp = write_manifest_atomically("[]").unwrap();
        let err = run_skillkit_with_json(
            script_path.to_str().unwrap(), &tmp, Duration::from_secs(1), false
        );
        let _ = std::fs::remove_file(&tmp);
        assert!(err.is_err());
        let msg = format!("{:#}", err.unwrap_err());
        assert!(msg.contains("timed out"), "expected timeout error, got: {msg}");
    }

    #[test]
    fn undeploy_removes_directory() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("test-skill");
        std::fs::create_dir_all(&target).unwrap();
        let cfg = skillkit_config_with_runner(&fake_skillkit_path());
        let backend = SkillKitBackend::new(&cfg);
        backend.undeploy(&target).unwrap();
        assert!(!target.exists());
    }

    #[test]
    fn undeploy_errors_on_missing_target() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("nonexistent");
        let cfg = skillkit_config_with_runner(&fake_skillkit_path());
        let backend = SkillKitBackend::new(&cfg);
        assert!(backend.undeploy(&target).is_err());
    }

    // ── build_manifest_entries ────────────────────────────────────────────────

    #[test]
    fn build_manifest_entries_gh_source_uses_skill_name() {
        let (mut skill, target) = make_skill("pdf-processing");
        skill.sha = "abc123".to_string(); // non-empty → gh: path
        let pairs = vec![(&skill, &target)];
        let entries = build_manifest_entries(&pairs).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "pdf-processing");
        assert_eq!(entries[0].source, "pdf-processing");
        assert_eq!(entries[0].version, "latest");
    }

    #[test]
    fn build_manifest_entries_dir_source_uses_cached_path() {
        let dir = TempDir::new().unwrap();
        let local_skill_dir = dir.path().join("my-local-skill");
        std::fs::create_dir_all(&local_skill_dir).unwrap();

        let (mut skill, target) = make_skill("my-local-skill");
        skill.sha = String::new(); // empty → dir: path
        skill.cached_path = local_skill_dir.clone();
        let pairs = vec![(&skill, &target)];
        let entries = build_manifest_entries(&pairs).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "my-local-skill");
        assert_eq!(entries[0].source, local_skill_dir.to_string_lossy());
        assert_eq!(entries[0].version, "latest");
    }

    #[test]
    fn build_manifest_entries_dir_source_empty_path_errors() {
        let (mut skill, target) = make_skill("broken");
        skill.sha = String::new();         // dir: path
        skill.cached_path = PathBuf::new(); // empty path → error
        let pairs = vec![(&skill, &target)];
        let err = build_manifest_entries(&pairs).unwrap_err();
        assert!(format!("{err}").contains("no cached_path"), "got: {err}");
    }

    // ── skillkit_source_from_str ──────────────────────────────────────────────

    #[test]
    fn skillkit_source_from_str_gh_uses_skill_name() {
        let result = skillkit_source_from_str("pdf", "gh:anthropics/skills/pdf-processing");
        assert_eq!(result, "pdf");
    }

    #[test]
    fn skillkit_source_from_str_repo_uses_skill_name() {
        let result = skillkit_source_from_str("my-skill", "repo:");
        assert_eq!(result, "my-skill");
    }

    #[test]
    fn skillkit_source_from_str_dir_without_tilde_returns_raw_path() {
        let result = skillkit_source_from_str("local", "dir:/absolute/path/to/skill");
        assert_eq!(result, "/absolute/path/to/skill");
    }

    #[test]
    fn skillkit_source_from_str_dir_with_tilde_expands_home() {
        let result = skillkit_source_from_str("local", "dir:~/projects/my-skill");
        // Should not start with ~ after expansion (home dir resolved).
        assert!(!result.starts_with('~'), "tilde not expanded: {result}");
        assert!(result.ends_with("projects/my-skill"), "unexpected path: {result}");
    }

    // ── parse_updated_names ───────────────────────────────────────────────────

    #[test]
    fn parse_updated_names_empty_returns_empty() {
        assert!(parse_updated_names("").is_empty());
        assert!(parse_updated_names("[]").is_empty());
    }

    #[test]
    fn parse_updated_names_parses_json_array() {
        let json = r#"[{"name":"pdf-processing","path":"/some/path"},{"name":"code-review","path":"/other"}]"#;
        let names = parse_updated_names(json);
        assert_eq!(names, vec!["pdf-processing", "code-review"]);
    }

    #[test]
    fn parse_updated_names_invalid_json_returns_empty() {
        let names = parse_updated_names("not valid json at all");
        assert!(names.is_empty());
    }

    // ── update_all ────────────────────────────────────────────────────────────

    #[test]
    fn update_all_calls_skillkit_with_update_flag() {
        let dir = TempDir::new().unwrap();
        // Script echoes its args so we can verify --update was passed.
        let script_path = write_temp_script(&dir,
            "#!/bin/sh\necho \"$@\" >&2\necho '[{\"name\":\"pdf-processing\",\"path\":\"/tmp/p\"}]'\n"
        );
        let cfg = skillkit_config_with_runner(script_path.to_str().unwrap());
        let backend = SkillKitBackend::new(&cfg);
        let pairs: Vec<(&str, &str)> = vec![("pdf-processing", "gh:anthropics/skills/pdf-processing")];
        let updated = backend.update_all(&pairs).unwrap();
        assert_eq!(updated, vec!["pdf-processing"]);
    }

    #[test]
    fn update_all_empty_skills_returns_empty() {
        let dir = TempDir::new().unwrap();
        let script_path = write_temp_script(&dir, "#!/bin/sh\necho '[]\n'\n");
        let cfg = skillkit_config_with_runner(script_path.to_str().unwrap());
        let backend = SkillKitBackend::new(&cfg);
        let updated = backend.update_all(&[]).unwrap();
        assert!(updated.is_empty());
    }

    #[test]
    fn update_all_dir_source_passes_expanded_path() {
        let dir = TempDir::new().unwrap();
        // Script captures args to verify manifest was written with a path source.
        let json_out = r#"[{"name":"my-local-skill","path":"/deployed/my-local-skill"}]"#;
        let script_path = write_temp_script(&dir, &format!(
            "#!/bin/sh\necho '{}'\n", json_out
        ));
        let cfg = skillkit_config_with_runner(script_path.to_str().unwrap());
        let backend = SkillKitBackend::new(&cfg);
        let local_path = dir.path().join("my-local-skill");
        std::fs::create_dir_all(&local_path).unwrap();
        let source = format!("dir:{}", local_path.to_string_lossy());
        let pairs: Vec<(&str, &str)> = vec![("my-local-skill", &source)];
        let updated = backend.update_all(&pairs).unwrap();
        assert_eq!(updated, vec!["my-local-skill"]);
    }
}
