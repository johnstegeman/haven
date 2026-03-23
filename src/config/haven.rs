use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Root config: `haven.toml` in the repo root.
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct HavenConfig {
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

    /// Custom template variables available in all `.tmpl` files.
    ///
    /// ```toml
    /// [data]
    /// host = "my-laptop"
    /// kanata_path = "/usr/local/bin/kanata"
    /// ```
    ///
    /// In templates: `{{ data.host }}` or `{{ data.kanata_path }}`
    #[serde(default)]
    pub data: HashMap<String, String>,
}

/// Security settings in `haven.toml`.
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

/// VCS settings in `haven.toml`.
///
/// ```toml
/// [vcs]
/// backend = "jj"
/// ```
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct VcsConfig {
    /// VCS backend: "git" (default) or "jj" (Jujutsu colocated).
    /// Can also be set via `--vcs` CLI flag or `HAVEN_VCS` env var.
    pub backend: Option<String>,
}

/// Telemetry settings in `haven.toml`.
///
/// ```toml
/// [telemetry]
/// enabled = true
/// ```
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct TelemetryConfig {
    /// Enable local telemetry. Defaults to false.
    /// Can also be enabled via the `HAVEN_TELEMETRY=1` environment variable,
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

impl HavenConfig {
    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = repo_root.join("haven.toml");
        if !path.exists() {
            // No haven.toml yet — auto-discover modules from modules/*.toml
            // so that `haven apply` works on a freshly-imported repo.
            return Self::discover(repo_root);
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("Cannot read {}", path.display()))?;
        let config: Self =
            toml::from_str(&text).with_context(|| format!("Invalid TOML in {}", path.display()))?;
        Ok(config)
    }

    /// Build a config from whatever module TOML files exist in `modules/`.
    /// Used as a fallback when `haven.toml` hasn't been created yet.
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
            "note: haven.toml not found — applying all discovered modules: {}",
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
            .with_context(|| format!("Profile '{}' not found in haven.toml", name))?;

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

    /// Write a fresh haven.toml scaffold.
    pub fn write_scaffold(repo_root: &Path) -> Result<()> {
        let path = repo_root.join("haven.toml");
        let scaffold = r#"# haven configuration
# Run `haven help` for usage.

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

/// Canonical path of the haven repo root.
///
/// Resolution order (first match wins):
/// 1. `$HAVEN_DIR` env var (explicit override)
/// 2. `~/haven` if it contains a haven repo (backward-compatible migration path)
/// 3. `$XDG_DATA_HOME/haven` if `$XDG_DATA_HOME` is set
/// 4. `~/.local/share/haven` (XDG default — same convention as chezmoi)
///
/// Use `haven source-path` to print the resolved path.
pub fn repo_root() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("HAVEN_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = dirs::home_dir().context("Cannot determine home directory")?;

    // Migration: honour ~/haven if it already contains a haven repo.
    let legacy = home.join("haven");
    if legacy.join("haven.toml").exists() || legacy.join("source").exists() {
        return Ok(legacy);
    }

    // XDG Data Home: $XDG_DATA_HOME/haven or ~/.local/share/haven.
    let xdg_data = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".local").join("share"));
    Ok(xdg_data.join("haven"))
}
