use anyhow::{bail, Result};
use std::path::Path;

use crate::commands::brew as brew_cmd;
use crate::commands::mise as mise_cmd;
use crate::config::haven::HavenConfig;
use crate::homebrew;
use crate::mise as mise_lib;

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
        "brew" => brew_cmd::install(repo_root, name, cask, module),
        "mise" => mise_cmd::install(repo_root, name, module),
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
            if let Err(e) = brew_cmd::uninstall(repo_root, name, false) {
                eprintln!("warning: brew uninstall failed: {}", e);
            }
            if let Err(e) = mise_cmd::uninstall(repo_root, name) {
                eprintln!("warning: mise uninstall failed: {}", e);
            }
            return Ok(());
        }
    }

    let backend = resolve_backend(brew_flag, mise_flag, cask, cfg)?;
    match backend.as_str() {
        "brew" => brew_cmd::uninstall(repo_root, name, cask),
        "mise" => mise_cmd::uninstall(repo_root, name),
        other => unreachable!(
            "backend '{}' passed resolve_backend but has no handler",
            other
        ),
    }
}

pub fn outdated(repo_root: &Path, cfg: &HavenConfig) -> Result<()> {
    let allowed = cfg.packages.allowed_backends()?;

    for backend in &allowed {
        match backend.as_str() {
            "brew" => {
                let brew = match homebrew::brew_path() {
                    Some(p) => p,
                    None => {
                        println!("brew not available — skipping");
                        continue;
                    }
                };
                let brew_str = brew.to_string_lossy();
                match homebrew::brew_outdated(&brew_str) {
                    Err(e) => println!("brew not available — skipping ({})", e),
                    Ok(pkgs) if pkgs.is_empty() => println!("brew: nothing outdated"),
                    Ok(pkgs) => {
                        println!("==> brew outdated");
                        for pkg in pkgs {
                            println!(
                                "  {}  {} → {}",
                                pkg.name, pkg.current_version, pkg.latest_version
                            );
                        }
                    }
                }
            }
            "mise" => {
                let mise_bin = match mise_lib::mise_path() {
                    Some(p) => p,
                    None => {
                        println!("mise not available — skipping");
                        continue;
                    }
                };
                let mise_str = mise_bin.to_string_lossy();
                let misefiles = mise_cmd::all_misefiles(repo_root)?;
                if misefiles.is_empty() {
                    println!("mise: no config files found");
                    continue;
                }
                let mut any_outdated = false;
                for config_path in &misefiles {
                    match mise_lib::mise_outdated(&mise_str, config_path) {
                        Err(e) => println!("mise not available — skipping ({})", e),
                        Ok(pkgs) if pkgs.is_empty() => {}
                        Ok(pkgs) => {
                            if !any_outdated {
                                println!("==> mise outdated");
                                any_outdated = true;
                            }
                            for pkg in pkgs {
                                println!(
                                    "  {}  {} → {}",
                                    pkg.name, pkg.current_version, pkg.latest_version
                                );
                            }
                        }
                    }
                }
                if !any_outdated {
                    println!("mise: nothing outdated");
                }
            }
            other => unreachable!("unknown backend '{}'", other),
        }
    }

    Ok(())
}

pub fn upgrade(repo_root: &Path, name: Option<&str>, cfg: &HavenConfig) -> Result<()> {
    let allowed = cfg.packages.allowed_backends()?;

    for backend in &allowed {
        match backend.as_str() {
            "brew" => {
                let brew = match homebrew::brew_path() {
                    Some(p) => p,
                    None => {
                        println!("brew not available — skipping");
                        continue;
                    }
                };
                let brew_str = brew.to_string_lossy();
                match homebrew::brew_upgrade(&brew_str, name) {
                    Ok(()) => println!("brew: upgraded {}", name.unwrap_or("all packages")),
                    Err(e) => eprintln!("brew upgrade failed: {}", e),
                }
            }
            "mise" => {
                let mise_bin = match mise_lib::mise_path() {
                    Some(p) => p,
                    None => {
                        println!("mise not available — skipping");
                        continue;
                    }
                };
                let mise_str = mise_bin.to_string_lossy();
                let misefiles = mise_cmd::all_misefiles(repo_root)?;
                if misefiles.is_empty() {
                    println!("mise: no config files found");
                    continue;
                }
                for config_path in &misefiles {
                    match mise_lib::mise_upgrade(&mise_str, config_path, name) {
                        Ok(()) => println!(
                            "mise: upgraded {} (config pin updated: {})",
                            name.unwrap_or("all tools"),
                            config_path.display()
                        ),
                        Err(e) => {
                            eprintln!("mise upgrade failed for {}: {}", config_path.display(), e)
                        }
                    }
                }
            }
            other => unreachable!("unknown backend '{}'", other),
        }
    }

    Ok(())
}

pub fn search(_repo_root: &Path, _term: &str, _cfg: &HavenConfig) -> Result<()> {
    Err(anyhow::anyhow!("not implemented"))
}
