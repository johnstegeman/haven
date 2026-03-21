use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::config::{sort_modules, DfilesConfig, ModuleConfig};
use crate::config::module::expand_tilde;
use crate::drift::{check_drift, check_drift_link, check_drift_link_template, check_drift_template, drift_marker, DriftKind};
use crate::source;
use crate::template::TemplateContext;

pub struct StatusOptions<'a> {
    pub repo_root: &'a Path,
    pub dest_root: &'a Path,
    /// Where Claude Code skills/commands live (`~/.claude` in production).
    pub claude_dir: &'a Path,
    pub profile: &'a str,
}

pub fn run(opts: &StatusOptions<'_>) -> Result<()> {
    let config = DfilesConfig::load(opts.repo_root)?;
    let modules = config.resolve_modules(opts.profile)?;
    let sorted = sort_modules(&modules);

    let template_ctx = TemplateContext::from_env(opts.profile, opts.repo_root);
    let mut any_drift = false;

    // ── File drift (global — files in source/ always apply regardless of module) ──
    let source_dir = opts.repo_root.join("source");
    let entries = source::scan(&source_dir)?;

    let mut file_drift: Vec<(String, DriftKind)> = Vec::new();
    for entry in &entries {
        let dest_expanded = expand_tilde(&entry.dest_tilde)?;
        let dest = resolve_dest(dest_expanded, opts.dest_root);

        let drift = if entry.flags.extdir {
            if !dest.exists() {
                DriftKind::Missing
            } else if !dest.join(".git").exists() {
                DriftKind::Modified
            } else {
                DriftKind::Clean
            }
        } else if entry.flags.symlink {
            if entry.flags.template {
                check_drift_link_template(&entry.src, &template_ctx, &dest)?
            } else {
                check_drift_link(&entry.src, &dest)
            }
        } else if entry.flags.template {
            check_drift_template(&entry.src, &template_ctx, &dest)?
        } else {
            check_drift(&entry.src, &dest)
        };

        if drift != DriftKind::Clean {
            file_drift.push((entry.dest_tilde.clone(), drift));
        }
    }

    if !file_drift.is_empty() {
        any_drift = true;
        println!("[files]");
        for (label, kind) in file_drift {
            println!("  {} {}", drift_marker(kind), label);
        }
    }

    // ── Master Brewfile drift (brew/Brewfile — not tied to any module) ───────────
    let master_brewfile = opts.repo_root.join("brew").join("Brewfile");
    if master_brewfile.exists() {
        if let Some(brew) = crate::homebrew::brew_path() {
            if !crate::homebrew::bundle_check(&brew, &master_brewfile) {
                any_drift = true;
                println!("[brew]");
                println!("  M brew/Brewfile");
            }
        }
    }

    // ── Per-module drift (externals, module brewfile, mise, AI) ──────────────────
    for module_name in &sorted {
        let module = ModuleConfig::load(opts.repo_root, module_name)?;
        if module.is_empty() {
            continue;
        }

        let mut module_drift: Vec<(String, DriftKind)> = vec![];

        // Module Brewfile drift
        if let Some(hb) = &module.homebrew {
            match crate::homebrew::brew_path() {
                None => {
                    module_drift.push((hb.brewfile.clone(), DriftKind::Missing));
                }
                Some(brew) => {
                    let brewfile = opts.repo_root.join(&hb.brewfile);
                    if !brewfile.exists() {
                        module_drift.push((hb.brewfile.clone(), DriftKind::SourceMissing));
                    } else if !crate::homebrew::bundle_check(&brew, &brewfile) {
                        module_drift.push((hb.brewfile.clone(), DriftKind::Modified));
                    }
                }
            }
        }

        // Mise drift
        if let Some(mise_cfg) = &module.mise {
            if let Some(mise) = crate::mise::mise_path() {
                let config_path = mise_cfg.config.as_ref().map(|c| opts.repo_root.join(c));
                let label = mise_cfg
                    .config
                    .as_deref()
                    .unwrap_or("mise.toml")
                    .to_string();
                if !crate::mise::tools_installed(&mise, config_path.as_deref()) {
                    module_drift.push((label, DriftKind::Modified));
                }
            }
            // If mise not installed, we don't report drift (can't know the state).
        }

        // AI drift
        if let Some(ai) = &module.ai {
            for source_str in &ai.skills {
                if let Ok(source) = crate::github::GhSource::parse(source_str) {
                    let installed = opts.claude_dir.join("skills").join(source.name());
                    if !installed.exists() {
                        module_drift.push((source_str.clone(), DriftKind::Missing));
                    }
                }
            }
            for source_str in &ai.commands {
                if let Ok(source) = crate::github::GhSource::parse(source_str) {
                    let installed = opts.claude_dir.join("commands").join(source.name());
                    if !installed.exists() {
                        module_drift.push((source_str.clone(), DriftKind::Missing));
                    }
                }
            }
        }

        if !module_drift.is_empty() {
            any_drift = true;
            println!("[{}]", module_name);
            for (label, kind) in module_drift {
                println!("  {} {}", drift_marker(kind), label);
            }
        }
    }

    if !any_drift {
        println!("✓ Everything up to date (profile: {})", opts.profile);
    }

    Ok(())
}

fn resolve_dest(dest: PathBuf, dest_root: &Path) -> PathBuf {
    if dest_root == Path::new("/") {
        dest
    } else {
        let rel = dest.strip_prefix("/").unwrap_or(&dest);
        dest_root.join(rel)
    }
}
