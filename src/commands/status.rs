use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::config::{sort_modules, DfilesConfig, ModuleConfig};
use crate::config::module::expand_tilde;
use crate::drift::{check_drift, check_drift_link, check_drift_link_template, check_drift_template, drift_marker, DriftKind};
use crate::ignore::IgnoreList;
use crate::source;
use crate::template::TemplateContext;

pub struct StatusOptions<'a> {
    pub repo_root: &'a Path,
    pub dest_root: &'a Path,
    /// Where Claude Code skills/commands live (`~/.claude` in production).
    pub claude_dir: &'a Path,
    pub profile: &'a str,
    /// Show dotfile drift. When all three section flags are false, all sections show.
    pub show_files: bool,
    /// Show Homebrew drift.
    pub show_brews: bool,
    /// Show AI skill drift.
    pub show_ai: bool,
}

pub fn run(opts: &StatusOptions<'_>) -> Result<()> {
    // If none of the section flags are set, show everything.
    let none_specified = !opts.show_files && !opts.show_brews && !opts.show_ai;
    let show_files = opts.show_files || none_specified;
    let show_brews = opts.show_brews || none_specified;
    let show_ai    = opts.show_ai   || none_specified;

    let config = DfilesConfig::load(opts.repo_root)?;
    let modules = config.resolve_modules(opts.profile)?;
    let sorted = sort_modules(&modules);

    let template_ctx = TemplateContext::from_env(opts.profile, opts.repo_root, config.data.clone());
    let mut any_drift = false;

    // ── File drift ────────────────────────────────────────────────────────────────
    if show_files {
        let source_dir = opts.repo_root.join("source");
        let ignore = IgnoreList::load(opts.repo_root);
        let entries = source::scan(&source_dir, &ignore)?;

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
    }

    // ── Brew drift ────────────────────────────────────────────────────────────────
    if show_brews {
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
    }

    // ── Per-module drift (module brewfile, mise, AI) ──────────────────────────────
    for module_name in &sorted {
        let module = ModuleConfig::load(opts.repo_root, module_name)?;
        if module.is_empty() {
            continue;
        }

        let mut module_drift: Vec<(String, DriftKind)> = vec![];

        // Module Brewfile drift
        if show_brews {
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
        }

        // Mise drift (grouped with brews — tool installs)
        if show_brews {
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
        }

        // AI drift is driven by ai/skills.toml, handled below.

        if !module_drift.is_empty() {
            any_drift = true;
            println!("[{}]", module_name);
            for (label, kind) in module_drift {
                println!("  {} {}", drift_marker(kind), label);
            }
        }
    }

    // ── AI skill drift (ai/skills.toml) ──────────────────────────────────────
    if show_ai {
        if let Some(skills_config) = crate::ai_skill::SkillsConfig::load(opts.repo_root)? {
            let mut ai_drift: Vec<(String, DriftKind)> = Vec::new();
            for skill in &skills_config.skills {
                let skill_dir = opts.claude_dir.join("skills").join(&skill.name);
                if !skill_dir.exists() {
                    ai_drift.push((skill.source.clone(), DriftKind::Missing));
                }
            }
            if !ai_drift.is_empty() {
                any_drift = true;
                println!("[ai]");
                for (label, kind) in ai_drift {
                    println!("  {} {}", drift_marker(kind), label);
                }
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
