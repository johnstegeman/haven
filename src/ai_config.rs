/// Configuration from `ai/config.toml` (optional file).
///
/// ```toml
/// [skills]
/// backend = "native"      # "native" | "akm"
/// ```
///
/// When `ai/config.toml` is absent, all defaults apply (native backend).
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// Which backend manages skills.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BackendKind {
    #[default]
    Native,
    Akm,
}

impl BackendKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            BackendKind::Native => "native",
            BackendKind::Akm    => "akm",
        }
    }
}

/// Resolved AI configuration for skill management.
#[derive(Debug, Clone)]
pub struct AiConfig {
    pub backend: BackendKind,
}

impl Default for AiConfig {
    fn default() -> Self {
        AiConfig {
            backend: BackendKind::Native,
        }
    }
}

impl AiConfig {
    /// Load from `ai/config.toml`. Returns `Ok(AiConfig::default())` if the
    /// file is absent. Returns an error if the file exists but is invalid.
    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = repo_root.join("ai").join("config.toml");
        if !path.exists() {
            return Ok(AiConfig::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("Cannot read {}", path.display()))?;
        let raw: RawAiConfig = toml::from_str(&text)
            .with_context(|| format!("Invalid TOML in {}", path.display()))?;
        raw.resolve(&path.display().to_string())
    }
}

// ─── Raw deserialization ───────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct RawAiConfig {
    #[serde(default)]
    skills: RawSkillsSection,
}

#[derive(Deserialize, Default)]
struct RawSkillsSection {
    backend: Option<String>,
}

impl RawAiConfig {
    fn resolve(self, path_display: &str) -> Result<AiConfig> {
        let backend = match self.skills.backend.as_deref().unwrap_or("native") {
            "native" => BackendKind::Native,
            "akm"    => BackendKind::Akm,
            other => anyhow::bail!(
                "{}: unknown skill backend '{}'\n\
                 hint: valid values are 'native', 'akm'",
                path_display, other
            ),
        };

        Ok(AiConfig { backend })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_config(dir: &TempDir, content: &str) {
        let ai_dir = dir.path().join("ai");
        fs::create_dir_all(&ai_dir).unwrap();
        fs::write(ai_dir.join("config.toml"), content).unwrap();
    }

    #[test]
    fn ai_config_defaults_to_native_when_no_file() {
        let dir = TempDir::new().unwrap();
        let cfg = AiConfig::load(dir.path()).unwrap();
        assert_eq!(cfg.backend, BackendKind::Native);
    }

    #[test]
    fn ai_config_reads_native_backend() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "[skills]\nbackend = \"native\"\n");
        let cfg = AiConfig::load(dir.path()).unwrap();
        assert_eq!(cfg.backend, BackendKind::Native);
    }

    #[test]
    fn ai_config_errors_on_unknown_backend() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "[skills]\nbackend = \"frobble\"\n");
        let err = AiConfig::load(dir.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("frobble"), "error should name the bad value: {msg}");
        assert!(msg.contains("hint:"), "error should include hint: {msg}");
    }

}
