use anyhow::{bail, Result};
use std::path::Path;

use crate::commands::brew;
use crate::config::haven::HavenConfig;

pub fn resolve_backend(brew_flag: bool, mise_flag: bool, cask: bool, cfg: &HavenConfig) -> Result<String> {
    let backend = if cask || brew_flag {
        "brew".to_string()
    } else if mise_flag {
        "mise".to_string()
    } else {
        cfg.packages.default_backend()?
    };

    let allowed = cfg.packages.allowed_backends()?;
    if !allowed.contains(&backend) {
        bail!(
            "backend '{}' is not in the allowed backends list for this repo (allowed: {})",
            backend,
            allowed.join(", ")
        );
    }

    Ok(backend)
}

pub fn install(
    repo_root: &Path,
    name: &str,
    brew_flag: bool,
    mise_flag: bool,
    cask: bool,
    module: Option<&str>,
    cfg: &HavenConfig,
) -> Result<()> {
    let backend = resolve_backend(brew_flag, mise_flag, cask, cfg)?;
    match backend.as_str() {
        "brew" => brew::install(repo_root, name, cask, module),
        "mise" => bail!("mise backend not yet available"),
        other => bail!("unhandled backend '{}'", other),
    }
}

pub fn uninstall(
    repo_root: &Path,
    name: &str,
    brew_flag: bool,
    mise_flag: bool,
    cask: bool,
    cfg: &HavenConfig,
) -> Result<()> {
    let backend = resolve_backend(brew_flag, mise_flag, cask, cfg)?;
    match backend.as_str() {
        "brew" => brew::uninstall(repo_root, name, cask),
        "mise" => bail!("mise backend not yet available"),
        other => bail!("unhandled backend '{}'", other),
    }
}
