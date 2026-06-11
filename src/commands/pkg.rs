use anyhow::{bail, Result};
use std::path::Path;

use crate::commands::brew;
use crate::commands::mise;
use crate::config::haven::HavenConfig;

pub fn resolve_backend(
    brew_flag: bool,
    mise_flag: bool,
    cask: bool,
    cfg: &HavenConfig,
) -> Result<String> {
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
    if cask && mise_flag {
        bail!("--cask is not supported with the mise backend (mise has no casks)");
    }

    let backend = resolve_backend(brew_flag, mise_flag, cask, cfg)?;
    match backend.as_str() {
        "brew" => brew::install(repo_root, name, cask, module),
        "mise" => mise::install(repo_root, name, module),
        other => unreachable!(
            "backend '{}' passed resolve_backend but has no handler",
            other
        ),
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
    if !brew_flag && !mise_flag && !cask {
        let allowed = cfg.packages.allowed_backends()?;
        let has_brew = allowed.contains(&"brew".to_string());
        let has_mise = allowed.contains(&"mise".to_string());
        if has_brew && has_mise {
            if let Err(e) = brew::uninstall(repo_root, name, false) {
                eprintln!("warning: brew uninstall failed: {}", e);
            }
            if let Err(e) = mise::uninstall(repo_root, name) {
                eprintln!("warning: mise uninstall failed: {}", e);
            }
            return Ok(());
        }
    }

    let backend = resolve_backend(brew_flag, mise_flag, cask, cfg)?;
    match backend.as_str() {
        "brew" => brew::uninstall(repo_root, name, cask),
        "mise" => mise::uninstall(repo_root, name),
        other => unreachable!(
            "backend '{}' passed resolve_backend but has no handler",
            other
        ),
    }
}

pub fn outdated(_repo_root: &Path, _cfg: &HavenConfig) -> Result<()> {
    Err(anyhow::anyhow!("not implemented"))
}

pub fn upgrade(_repo_root: &Path, _name: Option<&str>, _cfg: &HavenConfig) -> Result<()> {
    Err(anyhow::anyhow!("not implemented"))
}

pub fn search(_repo_root: &Path, _term: &str, _cfg: &HavenConfig) -> Result<()> {
    Err(anyhow::anyhow!("not implemented"))
}
