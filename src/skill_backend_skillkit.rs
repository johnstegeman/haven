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
    fn deploy_all(
        &self,
        skills: &[(&ResolvedSkill, &DeploymentTarget)],
    ) -> Result<Vec<DeployResult>> {
        if skills.is_empty() {
            return Ok(vec![]);
        }

        // Build manifest entries from skills. Only gh: sources are supported.
        let mut manifest_entries: Vec<SkillKitManifestEntry> = Vec::new();
        for (skill, _target) in skills {
            let source_key = gh_source_to_skillkit(&skill.name, &skill.cached_path)?;
            manifest_entries.push(SkillKitManifestEntry {
                name: skill.name.clone(),
                source: source_key,
                version: "latest".to_string(),
            });
        }

        // Write manifest atomically to a temp file.
        let manifest_json = serde_json::to_string(&manifest_entries)
            .context("failed to serialize SkillKit manifest")?;
        let tmp_path = write_manifest_atomically(&manifest_json)?;

        // Invoke skillkit team install --manifest <path> --json
        let result = run_skillkit_with_json(&self.runner, &tmp_path, self.timeout);

        // Clean up temp file regardless of outcome.
        let _ = std::fs::remove_file(&tmp_path);

        let stdout = result?;
        parse_deploy_results(&stdout, skills)
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

#[derive(serde::Serialize)]
struct SkillKitManifestEntry {
    name: String,
    source: String,
    version: String,
}

#[allow(dead_code)]
/// Convert a ResolvedSkill's cached_path to a SkillKit marketplace source ID.
///
/// SkillKit source IDs look like "anthropics/skills/pdf-processing" (no gh: prefix).
/// For Phase 3, this derives from the skill's cached_path directory name using
/// the convention established by NativeBackend's cache key: "owner--repo[--subpath]".
fn gh_source_to_skillkit(skill_name: &str, _cached_path: &Path) -> Result<String> {
    // In Phase 3, the ResolvedSkill.sha carries the original source string.
    // For now, use the skill name as the source identifier (SkillKit marketplace lookup).
    // This is a known limitation documented in TODOS.md — full source mapping
    // from cached_path→gh: source comes with SkillKit integration testing.
    Ok(skill_name.to_string())
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

#[allow(dead_code)]
/// Run `<runner> [skillkit] team install --manifest <path> --json` with timeout.
/// Returns stdout on success, or an error if the process exits non-zero or times out.
///
/// Package managers (npx, bunx, bun): prepend "skillkit" as the subcommand.
/// Direct binary paths (tests or user-installed): run as-is.
fn run_skillkit_with_json(runner: &str, manifest_path: &Path, timeout: Duration) -> Result<String> {
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(runner);
    // Package manager runners need "skillkit" as the package to execute.
    if matches!(runner, "npx" | "bunx" | "bun") {
        cmd.arg("skillkit");
    }
    let mut child = cmd
        .args(["team", "install", "--manifest"])
        .arg(manifest_path)
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
                    anyhow::bail!(
                        "skillkit exited with status {}: {}",
                        status.code().unwrap_or(-1),
                        stderr.trim()
                    );
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
            script_path.to_str().unwrap(), &tmp, Duration::from_secs(10)
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
            script_path.to_str().unwrap(), &tmp, Duration::from_secs(10)
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
            script_path.to_str().unwrap(), &tmp, Duration::from_secs(1)
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
}
