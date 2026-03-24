/// Configuration from `ai/config.toml` (optional file).
///
/// ```toml
/// [skills]
/// backend = "native"      # "native" | "skillkit" | "akm"
/// runner  = "npx"         # runner for external backends (default: "npx" for skillkit)
/// timeout_secs = 60       # subprocess timeout in seconds (default: 60)
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
    SkillKit,
    Akm,
}

impl BackendKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            BackendKind::Native  => "native",
            BackendKind::SkillKit => "skillkit",
            BackendKind::Akm     => "akm",
        }
    }
}

/// Resolved AI configuration for skill management.
#[derive(Debug, Clone)]
pub struct AiConfig {
    pub backend: BackendKind,
    /// Runner used to invoke external backends (e.g. "npx", "bunx").
    pub runner: String,
    /// Timeout in seconds for external backend subprocess calls.
    pub timeout_secs: u64,
}

impl Default for AiConfig {
    fn default() -> Self {
        AiConfig {
            backend:      BackendKind::Native,
            runner:       "npx".to_string(),
            timeout_secs: 60,
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
    backend:      Option<String>,
    runner:       Option<String>,
    timeout_secs: Option<u64>,
}

impl RawAiConfig {
    fn resolve(self, path_display: &str) -> Result<AiConfig> {
        let backend = match self.skills.backend.as_deref().unwrap_or("native") {
            "native"   => BackendKind::Native,
            "skillkit" => BackendKind::SkillKit,
            "akm"      => BackendKind::Akm,
            other => anyhow::bail!(
                "{}: unknown skill backend '{}'\n\
                 hint: valid values are 'native', 'skillkit', 'akm'",
                path_display, other
            ),
        };

        let default_runner = match backend {
            BackendKind::SkillKit => "npx",
            BackendKind::Akm     => "bun",
            BackendKind::Native  => "npx", // unused for native
        };

        Ok(AiConfig {
            backend,
            runner:       self.skills.runner.unwrap_or_else(|| default_runner.to_string()),
            timeout_secs: self.skills.timeout_secs.unwrap_or(60),
        })
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
        assert_eq!(cfg.timeout_secs, 60);
    }

    #[test]
    fn ai_config_reads_native_backend() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "[skills]\nbackend = \"native\"\n");
        let cfg = AiConfig::load(dir.path()).unwrap();
        assert_eq!(cfg.backend, BackendKind::Native);
    }

    #[test]
    fn ai_config_reads_skillkit_backend() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "[skills]\nbackend = \"skillkit\"\n");
        let cfg = AiConfig::load(dir.path()).unwrap();
        assert_eq!(cfg.backend, BackendKind::SkillKit);
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

    #[test]
    fn ai_config_reads_timeout_secs() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "[skills]\ntimeout_secs = 30\n");
        let cfg = AiConfig::load(dir.path()).unwrap();
        assert_eq!(cfg.timeout_secs, 30);
    }

    #[test]
    fn ai_config_reads_runner() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "[skills]\nbackend = \"skillkit\"\nrunner = \"bunx\"\n");
        let cfg = AiConfig::load(dir.path()).unwrap();
        assert_eq!(cfg.runner, "bunx");
    }

    #[test]
    fn ai_config_default_runner_for_skillkit_is_npx() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "[skills]\nbackend = \"skillkit\"\n");
        let cfg = AiConfig::load(dir.path()).unwrap();
        assert_eq!(cfg.runner, "npx");
    }
}
