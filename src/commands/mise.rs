/// Mise backend implementation for `haven pkg install` / `haven pkg uninstall`
///
/// Mise config layout:
///   mise/mise.toml            — master list (always applied on `haven apply`)
///   mise/mise.<module>.toml   — module subset (applied via `haven apply --module <m>`)
///
/// install (no --module):     add to mise/mise.toml (master), create if needed
/// install (--module <name>): add to mise/mise.<name>.toml, update module config
/// uninstall:                 remove from ALL mise/mise*.toml under mise/
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::config::module::{MiseConfig, ModuleConfig};
use crate::mise;

// ─── Public entry points ──────────────────────────────────────────────────────

/// `haven pkg install <name> [--module <module>]` (mise backend)
pub fn install(repo_root: &Path, name: &str, module_filter: Option<&str>) -> Result<()> {
    let (tool_name, version) = mise::parse_tool_spec(name);

    let target = resolve_install_target(repo_root, module_filter)?;
    let target_rel = target
        .strip_prefix(repo_root)
        .unwrap_or(&target)
        .display()
        .to_string();

    let added = mise::add_to_misefile(&target, &tool_name, &version)
        .with_context(|| format!("Cannot update {}", target.display()))?;

    if added {
        println!("  + \"{}\" @ {}  →  {}", tool_name, version, target_rel);
    } else {
        println!("  ~ \"{}\" already in {}  (skipped)", tool_name, target_rel);
    }

    println!();
    let all_files = all_misefiles(repo_root)?;
    let global_config = crate::mise::mise_global_config_path()?;
    crate::mise::merge_module_tools_into_global(&all_files, &global_config)
        .context("failed to update global mise config")?;
    if let Some(mise_bin) = crate::mise::mise_path() {
        crate::mise::install_tools(&mise_bin, None).context("mise install failed")?;
    }

    Ok(())
}

/// `haven pkg uninstall <name>` (mise backend)
pub fn uninstall(repo_root: &Path, name: &str) -> Result<()> {
    let (tool_name, _) = mise::parse_tool_spec(name);

    let misefiles = all_misefiles(repo_root)?;

    if misefiles.is_empty() {
        println!("No mise config files found in this haven repo.");
    } else {
        let mut total_removed = 0usize;
        for misefile_path in &misefiles {
            let misefile_rel = misefile_path
                .strip_prefix(repo_root)
                .unwrap_or(misefile_path)
                .display()
                .to_string();

            let removed = mise::remove_from_misefile(misefile_path, &tool_name)
                .with_context(|| format!("Cannot update {}", misefile_path.display()))?;

            if removed > 0 {
                println!("  - \"{}\"  from {}", tool_name, misefile_rel);
                total_removed += removed;
            }
        }
        if total_removed == 0 {
            println!(
                "  ~ \"{}\" not found in any mise config  (no changes made)",
                tool_name
            );
        }
    }

    println!();
    let global_config = crate::mise::mise_global_config_path()?;
    crate::mise::merge_module_tools_into_global(&misefiles, &global_config)
        .context("failed to update global mise config")?;

    if let Some(mise_bin) = mise::mise_path() {
        mise::mise_uninstall(&mise_bin, &tool_name).context("mise uninstall failed")?;
    } else {
        println!("  [mise] mise not found — uninstall skipped (run: haven apply to sync)");
    }

    Ok(())
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Collect every `mise/mise*.toml` path under the repo root.
pub fn all_misefiles(repo_root: &Path) -> Result<Vec<PathBuf>> {
    let mise_dir = repo_root.join("mise");
    if !mise_dir.exists() {
        return Ok(Vec::new());
    }

    let mut result = Vec::new();
    for entry in std::fs::read_dir(&mise_dir)
        .with_context(|| format!("Cannot read {}", mise_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "mise.toml" || name.starts_with("mise.") && name.ends_with(".toml") {
            result.push(path);
        }
    }

    result.sort();
    Ok(result)
}

/// Determine which mise config file to write to for `install`.
///
/// Resolution:
///   `--module <name>` → use `mise/mise.<name>.toml`, update module config
///   (no module, master exists)   → use `mise/mise.toml`
///   (no module, no master, one module file) → use that module file
///   (no module, no master, many module files) → error with hint
fn resolve_install_target(repo_root: &Path, module_filter: Option<&str>) -> Result<PathBuf> {
    if let Some(module) = module_filter {
        return resolve_module_misefile(repo_root, module);
    }

    let mise_dir = repo_root.join("mise");
    let master = mise_dir.join("mise.toml");

    if master.exists() {
        return Ok(master);
    }

    let module_files: Vec<PathBuf> = if mise_dir.exists() {
        std::fs::read_dir(&mise_dir)
            .with_context(|| format!("Cannot read {}", mise_dir.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("mise.") && n.ends_with(".toml"))
                    .unwrap_or(false)
            })
            .collect()
    } else {
        vec![]
    };

    match module_files.len() {
        0 => {
            std::fs::create_dir_all(&mise_dir)
                .with_context(|| format!("Cannot create {}", mise_dir.display()))?;
            Ok(master)
        }
        1 => {
            let path = module_files.into_iter().next().unwrap();
            let rel = path.strip_prefix(repo_root).unwrap_or(&path);
            println!(
                "note: no mise/mise.toml found; using existing {}",
                rel.display()
            );
            Ok(path)
        }
        _ => {
            let names: Vec<String> = module_files
                .iter()
                .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_string))
                .collect();
            anyhow::bail!(
                "Multiple module mise configs found ({}) and no master mise/mise.toml.\n\
                 Use --module <name> to specify which one to update.",
                names.join(", ")
            )
        }
    }
}

/// Get (or create) the mise config for a named module: `mise/mise.<module>.toml`.
/// Also registers it in the module's config TOML.
fn resolve_module_misefile(repo_root: &Path, module_name: &str) -> Result<PathBuf> {
    let mise_dir = repo_root.join("mise");
    std::fs::create_dir_all(&mise_dir)
        .with_context(|| format!("Cannot create {}", mise_dir.display()))?;

    let rel = format!("mise/mise.{}.toml", module_name);
    let misefile = repo_root.join(&rel);

    let mut config = ModuleConfig::load(repo_root, module_name)?;
    if config.mise.as_ref().and_then(|m| m.config.as_deref()) != Some(&rel) {
        config.mise = Some(MiseConfig {
            config: Some(rel.clone()),
        });
        config
            .save(repo_root, module_name)
            .with_context(|| format!("Cannot update modules/{}.toml", module_name))?;
        if !misefile.exists() {
            println!(
                "Created {} and registered it in modules/{}.toml",
                rel, module_name
            );
        }
    }

    Ok(misefile)
}
