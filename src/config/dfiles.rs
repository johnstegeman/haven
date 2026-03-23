use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Root config: `dfiles.toml` in the repo root.
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct DfilesConfig {
    #[serde(default)]
    pub profile: HashMap<String, ProfileConfig>,

    /// Opt-in local telemetry.
    #[serde(default)]
    pub telemetry: TelemetryConfig,

    /// VCS backend selection (git or jj colocated).
    #[serde(default)]
    pub vcs: VcsConfig,

    /// Security scanning settings.
    #[serde(default)]
    pub security: SecurityConfig,
}

/// Security settings in `dfiles.toml`.
///
/// ```toml
/// [security]
/// allow = ["~/.config/gh/hosts.yml", "~/.config/gcloud/**"]
/// ```
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct SecurityConfig {
    /// Paths to exclude from security scanning (glob patterns matched against dest_tilde).
    #[serde(default)]
    pub allow: Vec<String>,
}

/// VCS settings in `dfiles.toml`.
///
/// ```toml
/// [vcs]
/// backend = "jj"
/// ```
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct VcsConfig {
    /// VCS backend: "git" (default) or "jj" (Jujutsu colocated).
    /// Can also be set via `--vcs` CLI flag or `DFILES_VCS` env var.
    pub backend: Option<String>,
}

/// Telemetry settings in `dfiles.toml`.
///
/// ```toml
/// [telemetry]
/// enabled = true
/// ```
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct TelemetryConfig {
    /// Enable local telemetry. Defaults to false.
    /// Can also be enabled via the `DFILES_TELEMETRY=1` environment variable,
    /// or at compile time with the `telemetry-default-on` Cargo feature.
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Serialize)]

pub struct ProfileConfig {
    /// List of module names to apply for this profile.
    #[serde(default)]
    pub modules: Vec<String>,

    /// Optional parent profile to inherit modules from.
    pub extends: Option<String>,
}

impl DfilesConfig {
    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = repo_root.join("dfiles.toml");
        if !path.exists() {
            // No dfiles.toml yet — auto-discover modules from modules/*.toml
            // so that `dfiles apply` works on a freshly-imported repo.
            return Self::discover(repo_root);
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("Cannot read {}", path.display()))?;
        let config: Self =
            toml::from_str(&text).with_context(|| format!("Invalid TOML in {}", path.display()))?;
        Ok(config)
    }

    /// Build a config from whatever module TOML files exist in `modules/`.
    /// Used as a fallback when `dfiles.toml` hasn't been created yet.
    fn discover(repo_root: &Path) -> Result<Self> {
        let modules_dir = repo_root.join("modules");
        let mut modules: Vec<String> = Vec::new();
        if modules_dir.exists() {
            for entry in std::fs::read_dir(&modules_dir)
                .with_context(|| format!("Cannot read {}", modules_dir.display()))?
            {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        modules.push(stem.to_string());
                    }
                }
            }
            modules.sort();
        }
        eprintln!(
            "note: dfiles.toml not found — applying all discovered modules: {}",
            if modules.is_empty() { "(none)".to_string() } else { modules.join(", ") }
        );
        let mut profile = HashMap::new();
        profile.insert(
            "default".to_string(),
            ProfileConfig { modules, extends: None },
        );
        Ok(Self { profile, ..Self::default() })
    }

    /// Returns the resolved module list for a profile, flattening `extends`.
    pub fn resolve_modules(&self, profile_name: &str) -> Result<Vec<String>> {
        let mut seen = vec![];
        self.collect_modules(profile_name, &mut seen, 0)?;
        Ok(seen)
    }

    fn collect_modules(
        &self,
        name: &str,
        out: &mut Vec<String>,
        depth: usize,
    ) -> Result<()> {
        if depth > 10 {
            bail!("Profile inheritance too deep — possible circular extends");
        }
        let profile = self
            .profile
            .get(name)
            .with_context(|| format!("Profile '{}' not found in dfiles.toml", name))?;

        // Apply parent first, then override with this profile's modules.
        if let Some(parent) = &profile.extends {
            self.collect_modules(parent, out, depth + 1)?;
        }

        for m in &profile.modules {
            if !out.contains(m) {
                out.push(m.clone());
            }
        }
        Ok(())
    }

    /// Write a fresh dfiles.toml scaffold.
    pub fn write_scaffold(repo_root: &Path) -> Result<()> {
        let path = repo_root.join("dfiles.toml");
        let scaffold = r#"# dfiles configuration
# Run `dfiles help` for usage.

[profile.default]
modules = ["shell"]

[profile.minimal]
modules = ["shell"]

[profile.work]
extends = "default"
modules = []

[profile.personal]
extends = "default"
modules = []
"#;
        std::fs::write(&path, scaffold)
            .with_context(|| format!("Cannot write {}", path.display()))?;
        Ok(())
    }
}

/// Canonical path of the dfiles repo root.
///
/// Resolution order (first match wins):
/// 1. `$DFILES_DIR` env var (explicit override)
/// 2. `~/dfiles` if it contains a dfiles repo (backward-compatible migration path)
/// 3. `$XDG_DATA_HOME/dfiles` if `$XDG_DATA_HOME` is set
/// 4. `~/.local/share/dfiles` (XDG default — same convention as chezmoi)
///
/// Use `dfiles source-path` to print the resolved path.
pub fn repo_root() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("DFILES_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = dirs::home_dir().context("Cannot determine home directory")?;

    // Migration: honour ~/dfiles if it already contains a dfiles repo.
    let legacy = home.join("dfiles");
    if legacy.join("dfiles.toml").exists() || legacy.join("source").exists() {
        return Ok(legacy);
    }

    // XDG Data Home: $XDG_DATA_HOME/dfiles or ~/.local/share/dfiles.
    let xdg_data = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".local").join("share"));
    Ok(xdg_data.join("dfiles"))
}
