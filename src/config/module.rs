use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A module config file, e.g. `modules/shell.toml`.
///
/// Modules scope **brew and mise only** — file tracking is handled by magic-name
/// encoding in `source/` and always applies in full. AI skills and commands are
/// managed in `ai/skills.toml` and `ai/platforms.toml`.
///
/// ```toml
/// # modules/shell.toml
/// [homebrew]
/// brewfile = "brew/Brewfile.shell"
///
/// [mise]
/// config = "source/mise.toml"
/// ```
#[derive(Debug, Deserialize, Serialize, Default)]

pub struct ModuleConfig {
    /// Homebrew Brewfile for this module.
    pub homebrew: Option<HomebrewConfig>,

    /// Mise tool version management.
    pub mise: Option<MiseConfig>,

    /// If true, skip this module with a warning when 1Password CLI (`op`) is
    /// not installed or the user is not signed in.
    #[serde(default)]
    pub requires_op: bool,
}

/// Homebrew configuration within a module.
///
/// ```toml
/// [homebrew]
/// brewfile = "brew/Brewfile.shell"
/// sort = true   # optional; sorts entries alphabetically on apply
/// ```
#[derive(Debug, Deserialize, Serialize, Default, Clone)]

pub struct HomebrewConfig {
    /// Path to this module's Brewfile, relative to the haven repo root.
    /// Convention: `brew/Brewfile.<module>`.
    pub brewfile: String,

    /// When true, sort the Brewfile alphabetically by entry name before
    /// running `brew bundle install`. Entries are sorted per-kind (taps,
    /// formulas, and casks are each sorted independently), so existing
    /// section groupings are preserved. Default: false (opt-in).
    #[serde(default)]
    pub sort: bool,
}

/// Mise (runtime version manager) configuration within a module.
///
/// ```toml
/// [mise]
/// config = "source/mise.toml"
/// ```
#[derive(Debug, Deserialize, Serialize, Clone)]

pub struct MiseConfig {
    /// Path to the mise config file, relative to the haven repo root.
    pub config: Option<String>,
}

impl ModuleConfig {
    /// Returns true if this module has nothing to apply.
    pub fn is_empty(&self) -> bool {
        self.homebrew.is_none() && self.mise.is_none()
    }
}

impl ModuleConfig {
    pub fn load(repo_root: &Path, module_name: &str) -> Result<Self> {
        let path = repo_root
            .join("modules")
            .join(format!("{}.toml", module_name));
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("Cannot read {}", path.display()))?;
        let config: Self = toml::from_str(&text)
            .with_context(|| format!("Invalid TOML in {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self, repo_root: &Path, module_name: &str) -> Result<()> {
        let dir = repo_root.join("modules");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.toml", module_name));
        let text = toml::to_string_pretty(self)?;
        std::fs::write(&path, text)
            .with_context(|| format!("Cannot write {}", path.display()))?;
        Ok(())
    }

}

/// Canonical dependency order for modules.
/// Modules not in this list are applied after the listed ones.
const MODULE_ORDER: &[&str] = &["shell", "git", "packages", "secrets", "ai"];

/// Sort a list of module names into canonical dependency order.
pub fn sort_modules(modules: &[String]) -> Vec<String> {
    let mut ordered: Vec<String> = MODULE_ORDER
        .iter()
        .filter(|m| modules.contains(&m.to_string()))
        .map(|m| m.to_string())
        .collect();
    for m in modules {
        if !ordered.contains(m) {
            ordered.push(m.clone());
        }
    }
    ordered
}

pub fn expand_tilde(path: &str) -> Result<PathBuf> {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = dirs::home_dir().context("Cannot determine home directory")?;
        Ok(home.join(rest))
    } else if path == "~" {
        Ok(dirs::home_dir().context("Cannot determine home directory")?)
    } else {
        Ok(PathBuf::from(path))
    }
}
