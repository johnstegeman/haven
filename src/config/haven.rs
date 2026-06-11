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

    /// Package backend configuration.
    #[serde(default)]
    pub packages: PackagesConfig,

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

const KNOWN_BACKENDS: &[&str] = &["brew", "mise"];

/// Package backend settings in `haven.toml`.
///
/// ```toml
/// [packages]
/// backends = ["mise", "brew"]
/// ```
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct PackagesConfig {
    /// Ordered list of allowed backends. First entry becomes the default.
    /// Valid values: `"brew"`, `"mise"`. Defaults to `["brew", "mise"]`.
    #[serde(default)]
    pub backends: Vec<String>,
}

impl PackagesConfig {
    /// Returns the resolved ordered list of allowed backends.
    ///
    /// If the configured list is empty, returns the built-in default `["brew", "mise"]`.
    /// Errors if any entry is not a known backend name.
    pub fn allowed_backends(&self) -> Result<Vec<String>> {
        if self.backends.is_empty() {
            return Ok(KNOWN_BACKENDS.iter().map(|s| s.to_string()).collect());
        }
        for b in &self.backends {
            if !KNOWN_BACKENDS.contains(&b.as_str()) {
                bail!(
                    "unknown package backend '{}' in [packages] backends (valid: {})",
                    b,
                    KNOWN_BACKENDS.join(", ")
                );
            }
        }
        Ok(self.backends.clone())
    }

    /// Returns the default backend (first in the resolved allowed list).
    pub fn default_backend(&self) -> Result<String> {
        let backends = self.allowed_backends()?;
        Ok(backends.into_iter().next().unwrap())
    }
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
        let mut config: Self =
            toml::from_str(&text).with_context(|| format!("Invalid TOML in {}", path.display()))?;

        // Merge haven.local.toml if present — local overrides win over the shared config.
        // This file should be gitignored and never committed, allowing per-machine customization.
        let local_path = repo_root.join("haven.local.toml");
        if local_path.exists() {
            let local_text = std::fs::read_to_string(&local_path)
                .with_context(|| format!("Cannot read {}", local_path.display()))?;
            let local: Self = toml::from_str(&local_text)
                .with_context(|| format!("Invalid TOML in {}", local_path.display()))?;
            // Local [data] keys override shared ones; keys not in local are kept.
            config.data.extend(local.data);
        }

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
            if modules.is_empty() {
                "(none)".to_string()
            } else {
                modules.join(", ")
            }
        );
        let mut profile = HashMap::new();
        profile.insert(
            "default".to_string(),
            ProfileConfig {
                modules,
                extends: None,
            },
        );
        Ok(Self {
            profile,
            ..Self::default()
        })
    }

    /// Returns the resolved module list for a profile, flattening `extends`.
    pub fn resolve_modules(&self, profile_name: &str) -> Result<Vec<String>> {
        let mut seen = vec![];
        self.collect_modules(profile_name, &mut seen, 0)?;
        Ok(seen)
    }

    fn collect_modules(&self, name: &str, out: &mut Vec<String>, depth: usize) -> Result<()> {
        if depth > 10 {
            bail!("Profile inheritance too deep — possible circular extends");
        }
        let profile = match self.profile.get(name) {
            Some(p) => p,
            None if name == "default" => {
                // The default profile is optional — treat it as empty rather than
                // erroring, so repos without an explicit [profile.default] still work.
                return Ok(());
            }
            None => bail!("Profile '{}' not found in haven.toml", name),
        };

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
