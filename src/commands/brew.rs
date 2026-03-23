/// `haven brew install` / `haven brew uninstall`
///
/// Runs the real `brew` command and keeps your haven Brewfiles in sync.
///
/// Brewfile layout:
///   brew/Brewfile            — master list (always applied on `haven apply`)
///   brew/Brewfile.<module>   — module subset (applied via `haven apply --module <m>`)
///
/// install (no --module):     add to brew/Brewfile (master), create if needed
/// install (--module <name>): add to brew/Brewfile.<name>, update module config
/// uninstall:                 remove from ALL Brewfiles under brew/
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::config::module::{HomebrewConfig, ModuleConfig};
use crate::homebrew;

// ─── Public entry points ──────────────────────────────────────────────────────

/// `haven brew install <name> [--cask] [--module <module>]`
pub fn install(repo_root: &Path, name: &str, cask: bool, module_filter: Option<&str>) -> Result<()> {
    let kind = if cask { "cask" } else { "brew" };

    let brew = homebrew::brew_path()
        .context("Homebrew not found. Install it from https://brew.sh")?;

    let brewfile = resolve_install_target(repo_root, module_filter)?;
    let brewfile_rel = brewfile
        .strip_prefix(repo_root)
        .unwrap_or(&brewfile)
        .display()
        .to_string();

    // Add to Brewfile first (idempotent).
    let added = homebrew::add_to_brewfile(&brewfile, kind, name)
        .with_context(|| format!("Cannot update {}", brewfile.display()))?;

    if added {
        println!("  + {} \"{}\"  →  {}", kind, name, brewfile_rel);
    } else {
        println!("  ~ {} \"{}\" already in {}  (skipped)", kind, name, brewfile_rel);
    }

    // Run brew install.
    println!();
    homebrew::brew_install(&brew, name, cask)?;

    Ok(())
}

/// `haven brew uninstall <name> [--cask]`
pub fn uninstall(repo_root: &Path, name: &str, cask: bool) -> Result<()> {
    let kind = if cask { "cask" } else { "brew" };

    let brew = homebrew::brew_path()
        .context("Homebrew not found. Install it from https://brew.sh")?;

    // Remove from every Brewfile under brew/.
    let brewfiles = all_brewfiles(repo_root)?;

    if brewfiles.is_empty() {
        println!("No Brewfiles found in this haven repo.");
    } else {
        let mut total_removed = 0usize;
        for brewfile_path in &brewfiles {
            let brewfile_rel = brewfile_path
                .strip_prefix(repo_root)
                .unwrap_or(brewfile_path)
                .display()
                .to_string();

            let removed = homebrew::remove_from_brewfile(brewfile_path, kind, name)
                .with_context(|| format!("Cannot update {}", brewfile_path.display()))?;

            if removed > 0 {
                println!("  - {} \"{}\"  from {}", kind, name, brewfile_rel);
                total_removed += removed;
            }
        }
        if total_removed == 0 {
            println!(
                "  ~ {} \"{}\" not found in any Brewfile  (no changes made)",
                kind, name
            );
        }
    }

    // Run brew uninstall.
    println!();
    homebrew::brew_uninstall(&brew, name, cask)?;

    Ok(())
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Collect every Brewfile path under `brew/` in the repo.
fn all_brewfiles(repo_root: &Path) -> Result<Vec<PathBuf>> {
    let brew_dir = repo_root.join("brew");
    if !brew_dir.exists() {
        return Ok(Vec::new());
    }

    let mut result = Vec::new();
    for entry in std::fs::read_dir(&brew_dir)
        .with_context(|| format!("Cannot read {}", brew_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if name == "Brewfile" || name.starts_with("Brewfile.") {
            result.push(path);
        }
    }

    result.sort();
    Ok(result)
}

/// Determine which Brewfile to write to for `install`.
///
/// Resolution:
///   `--module <name>` → use `brew/Brewfile.<name>`, update module config
///   (no module)       → use `brew/Brewfile` (master)
fn resolve_install_target(repo_root: &Path, module_filter: Option<&str>) -> Result<PathBuf> {
    if let Some(module) = module_filter {
        return resolve_module_brewfile(repo_root, module);
    }

    // No module — write to master brew/Brewfile.
    let brew_dir = repo_root.join("brew");
    std::fs::create_dir_all(&brew_dir)
        .with_context(|| format!("Cannot create {}", brew_dir.display()))?;
    Ok(brew_dir.join("Brewfile"))
}

/// Get (or create) the Brewfile for a named module: `brew/Brewfile.<module>`.
/// Also registers it in the module's config TOML.
fn resolve_module_brewfile(repo_root: &Path, module_name: &str) -> Result<PathBuf> {
    let brew_dir = repo_root.join("brew");
    std::fs::create_dir_all(&brew_dir)
        .with_context(|| format!("Cannot create {}", brew_dir.display()))?;

    let rel = format!("brew/Brewfile.{}", module_name);
    let brewfile = repo_root.join(&rel);

    // Ensure module config points to this Brewfile.
    let mut config = ModuleConfig::load(repo_root, module_name)?;
    if config.homebrew.as_ref().map(|h| h.brewfile.as_str()) != Some(&rel) {
        config.homebrew = Some(HomebrewConfig { brewfile: rel.clone() });
        config
            .save(repo_root, module_name)
            .with_context(|| format!("Cannot update modules/{}.toml", module_name))?;
        if !brewfile.exists() {
            println!(
                "Created {} and registered it in modules/{}.toml",
                rel, module_name
            );
        }
    }

    Ok(brewfile)
}
