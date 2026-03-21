use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Written to `~/.dfiles/state.json` after every successful apply.
/// Used by `dfiles status`, `dfiles diff`, and the future web dashboard.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct State {
    pub version: String,
    pub last_apply: Option<DateTime<Utc>>,
    pub profile: Option<String>,
    pub hostname: String,
    pub modules: HashMap<String, ModuleState>,
    /// AI skill deployment state. Absent in old state.json — default is None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<AiState>,
    /// Tracks which `run_once_` scripts have been executed on this machine.
    /// Key: script name (filename only, e.g. `"run_once_setup.sh"`).
    /// Value: ISO-8601 timestamp of first successful execution.
    /// Absent in old state.json files — defaults to an empty map.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub scripts_run: std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ModuleState {
    pub status: String, // "clean" | "drift" | "error"
    pub files: usize,
}

/// Tracks which skills dfiles has deployed to which platforms.
///
/// Written as the `"ai"` key in state.json. Old state files that lack this key
/// deserialize fine thanks to `#[serde(default)]` on `State::ai`.
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct AiState {
    /// deployed_skills[platform_id][skill_name] = entry
    #[serde(default)]
    pub deployed_skills: HashMap<String, HashMap<String, AiDeployedEntry>>,
}

/// A single deployed skill entry in state.json.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AiDeployedEntry {
    /// Full source string as declared in skills.toml (e.g. `gh:owner/repo/subpath`).
    pub source: String,
    /// Deploy method used: `"symlink"` or `"copy"`.
    pub deploy: String,
    /// Absolute path where the skill was deployed.
    pub target: PathBuf,
    /// RFC-3339 timestamp of this deployment.
    pub applied_at: String,
    /// SHA from the lock file at deploy time (git SHA or tarball SHA-256).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
}

impl State {
    pub fn load(state_dir: &Path) -> Result<Self> {
        let path = state_dir.join("state.json");
        if !path.exists() {
            return Ok(Self {
                version: "1".into(),
                hostname: hostname(),
                ..Default::default()
            });
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("Cannot read {}", path.display()))?;
        let state: Self = serde_json::from_str(&text)
            .with_context(|| format!("Invalid JSON in {}", path.display()))?;
        Ok(state)
    }

    pub fn save(&self, state_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(state_dir)?;
        let path = state_dir.join("state.json");
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, text)
            .with_context(|| format!("Cannot write {}", path.display()))?;
        Ok(())
    }
}

pub fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn state_round_trips_with_ai_section() {
        let dir = TempDir::new().unwrap();

        let mut state = State {
            version: "1".into(),
            hostname: "testhost".into(),
            ..Default::default()
        };

        let mut platform_skills = HashMap::new();
        platform_skills.insert(
            "pdf-processing".to_string(),
            AiDeployedEntry {
                source: "gh:anthropics/skills/pdf-processing".into(),
                deploy: "symlink".into(),
                target: PathBuf::from("/home/user/.claude/skills/pdf-processing"),
                applied_at: "2026-03-21T10:00:00Z".into(),
                sha: Some("abc123".into()),
            },
        );
        let mut ai = AiState::default();
        ai.deployed_skills.insert("claude-code".to_string(), platform_skills);
        state.ai = Some(ai);

        state.save(dir.path()).unwrap();

        let loaded = State::load(dir.path()).unwrap();
        let ai = loaded.ai.unwrap();
        let entry = &ai.deployed_skills["claude-code"]["pdf-processing"];
        assert_eq!(entry.source, "gh:anthropics/skills/pdf-processing");
        assert_eq!(entry.deploy, "symlink");
        assert_eq!(entry.sha, Some("abc123".into()));
    }

    #[test]
    fn old_state_without_ai_section_loads_cleanly() {
        let dir = TempDir::new().unwrap();
        // Write an old-format state.json (no "ai" key).
        std::fs::write(
            dir.path().join("state.json"),
            r#"{"version":"1","hostname":"old-host","modules":{}}"#,
        )
        .unwrap();

        let state = State::load(dir.path()).unwrap();
        assert_eq!(state.hostname, "old-host");
        // ai field defaults to None — no error.
        assert!(state.ai.is_none());
    }

    #[test]
    fn state_without_ai_serializes_without_ai_key() {
        // Ensure old state.json files stay clean when no AI skills have been deployed.
        let dir = TempDir::new().unwrap();
        let state = State {
            version: "1".into(),
            hostname: "host".into(),
            ..Default::default()
        };
        state.save(dir.path()).unwrap();

        let text = std::fs::read_to_string(dir.path().join("state.json")).unwrap();
        assert!(!text.contains("\"ai\""), "state.json should not contain ai key when ai is None");
    }
}
