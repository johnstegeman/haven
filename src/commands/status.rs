use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::config::{sort_modules, HavenConfig, ModuleConfig};
use crate::config::module::expand_tilde;
use crate::drift::{check_drift, check_drift_link, check_drift_link_template, check_drift_template, drift_marker, DriftKind};
use crate::fs::sha256_of_bytes;
use crate::ignore::IgnoreList;
use crate::source;
use crate::state::State;
use crate::template::TemplateContext;

pub struct StatusOptions<'a> {
    pub repo_root: &'a Path,
    pub dest_root: &'a Path,
    /// Where Claude Code skills/commands live (`~/.claude` in production).
    pub claude_dir: &'a Path,
    /// Where state.json is stored (`~/.haven` in production).
    pub state_dir: &'a Path,
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

    println!("Profile: {}", opts.profile);

    let config = HavenConfig::load(opts.repo_root)?;
    let modules = config.resolve_modules(opts.profile)?;
    let sorted = sort_modules(&modules);

    let template_ctx = TemplateContext::from_env(opts.profile, opts.repo_root, config.data.clone());
    let mut any_drift = false;

    // Load state for conflict (C marker) detection.
    let state = State::load(opts.state_dir).unwrap_or_default();

    // ── File drift ────────────────────────────────────────────────────────────────
    if show_files {
        let source_dir = opts.repo_root.join("source");
        let ignore = IgnoreList::load(opts.repo_root, &template_ctx);
        let entries = source::scan(&source_dir, &ignore)?;

        let mut file_drift: Vec<(String, String)> = Vec::new();
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

            // C marker: check whether dest was edited since last apply.
            // Only applies to plain/template files that have a prior hash.
            // Symlinks, extdirs, extfiles, and create_only files are excluded.
            let user_edited = if !entry.flags.extdir
                && !entry.flags.extfile
                && !entry.flags.symlink
                && !entry.flags.create_only
            {
                if let Some(prior) = state.applied_files.get(&entry.dest_tilde) {
                    match std::fs::read(&dest) {
                        Ok(bytes) => sha256_of_bytes(&bytes) != prior.sha256,
                        Err(_) => false, // unreadable dest — no C marker
                    }
                } else {
                    false // no prior hash — no C marker
                }
            } else {
                false
            };

            let marker = match (drift != DriftKind::Clean, user_edited) {
                (true,  true)  => Some("MC".to_string()),
                (true,  false) => Some(drift_marker(drift).to_string()),
                (false, true)  => Some("C".to_string()),
                (false, false) => None,
            };
            if let Some(m) = marker {
                file_drift.push((entry.dest_tilde.clone(), m));
            }
        }

        if !file_drift.is_empty() {
            any_drift = true;
            println!("[files]");
            for (label, marker) in file_drift {
                println!("  {} {}", marker, label);
            }
        }
    }

    // ── Brew drift ────────────────────────────────────────────────────────────────
    // Collect all Brewfile paths (master + module) for a single unified diff.
    if show_brews {
        if let Some(brew) = crate::homebrew::brew_path() {
            let mut brewfile_paths: Vec<PathBuf> = Vec::new();

            let master = opts.repo_root.join("brew").join("Brewfile");
            if master.exists() {
                brewfile_paths.push(master);
            }

            for module_name in &sorted {
                if let Ok(module) = ModuleConfig::load(opts.repo_root, module_name) {
                    if let Some(hb) = &module.homebrew {
                        let bf = opts.repo_root.join(&hb.brewfile);
                        if bf.exists() {
                            brewfile_paths.push(bf);
                        } else {
                            any_drift = true;
                            println!("[{}]", module_name);
                            println!("  ! {} (Brewfile not found)", hb.brewfile);
                        }
                    }
                }
            }

            if !brewfile_paths.is_empty() {
                let refs: Vec<&Path> = brewfile_paths.iter().map(PathBuf::as_path).collect();
                let diff = crate::homebrew::brewfile_diff(&brew, &refs)?;
                if !diff.is_clean() {
                    any_drift = true;
                    println!("[brew]");
                    for name in &diff.missing_formulas {
                        println!("  ? {}  (missing — haven apply --brews)", name);
                    }
                    for name in &diff.missing_casks {
                        println!("  ? {} --cask  (missing — haven apply --brews)", name);
                    }
                    for name in &diff.extra_formulas {
                        println!("  + {}  (installed, not in Brewfile)", name);
                    }
                    for name in &diff.extra_casks {
                        println!("  + {} --cask  (installed, not in Brewfile)", name);
                    }
                }
            }
        }
    }

    // ── Per-module drift (mise, AI — brew handled above) ─────────────────────────
    for module_name in &sorted {
        let module = ModuleConfig::load(opts.repo_root, module_name)?;
        if module.is_empty() {
            continue;
        }

        let mut module_drift: Vec<(String, DriftKind)> = vec![];

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
        println!("✓ Everything up to date");
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
