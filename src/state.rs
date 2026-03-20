use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Written to `~/.dfiles/state.json` after every successful apply.
/// Used by `dfiles status`, `dfiles diff`, and the future web dashboard.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct State {
    pub version: String,
    pub last_apply: Option<DateTime<Utc>>,
    pub profile: Option<String>,
    pub hostname: String,
    pub modules: HashMap<String, ModuleState>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ModuleState {
    pub status: String, // "clean" | "drift" | "error"
    pub files: usize,
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
