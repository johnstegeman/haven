/// List all tracked items: files, brews, and AI skills.
///
/// By default shows all sections. Use --files, --brews, or --ai to filter.
use anyhow::Result;
use std::path::Path;

use crate::config::{sort_modules, HavenConfig, ModuleConfig};
use crate::ignore::IgnoreList;
use crate::source;
use crate::template::TemplateContext;

pub struct ListOptions<'a> {
    pub repo_root: &'a Path,
    pub profile: &'a str,
    pub show_files: bool,
    pub show_brews: bool,
    pub show_ai: bool,
}

pub fn run(opts: &ListOptions<'_>) -> Result<()> {
    let none_specified = !opts.show_files && !opts.show_brews && !opts.show_ai;
    let show_files = opts.show_files || none_specified;
    let show_brews = opts.show_brews || none_specified;
    let show_ai    = opts.show_ai   || none_specified;

    let config = HavenConfig::load(opts.repo_root).unwrap_or_default();
    let modules = config.resolve_modules(opts.profile)?;
    let sorted = sort_modules(&modules);

    // ── Files ─────────────────────────────────────────────────────────────────
    if show_files {
        let source_dir = opts.repo_root.join("source");
        let ctx = TemplateContext::from_env_for_repo(opts.repo_root);
        let ignore = IgnoreList::load(opts.repo_root, &ctx);
        let entries = source::scan(&source_dir, &ignore)?;

        if !entries.is_empty() {
            if !none_specified {
                println!("[files]");
            }
            for entry in &entries {
                let mut tags: Vec<&str> = Vec::new();
                if entry.flags.template   { tags.push("template"); }
                if entry.flags.symlink    { tags.push("symlink"); }
                if entry.flags.private    { tags.push("private"); }
                if entry.flags.executable { tags.push("executable"); }
                if entry.flags.extdir     { tags.push("extdir"); }
                if entry.flags.extfile    { tags.push("extfile"); }
                if entry.flags.create_only { tags.push("create-only"); }
                if entry.flags.exact      { tags.push("exact"); }

                let indent = if none_specified { "" } else { "  " };
                if tags.is_empty() {
                    println!("{}{}", indent, entry.dest_tilde);
                } else {
                    println!("{}{}  ({})", indent, entry.dest_tilde, tags.join(", "));
                }
            }
        } else if opts.show_files {
            println!("[files]");
            println!("  (none — run `haven add <file>` to start tracking)");
        }
    }

    // ── Brews ─────────────────────────────────────────────────────────────────
    if show_brews {
        let mut printed_header = false;
        let mut print_brew_header = |printed: &mut bool| {
            if !*printed {
                println!("[brew]");
                *printed = true;
            }
        };

        // Master Brewfile
        let master = opts.repo_root.join("brew").join("Brewfile");
        if master.exists() {
            print_brew_header(&mut printed_header);
            list_brewfile(&master, opts.repo_root)?;
        }

        // Module Brewfiles
        for module_name in &sorted {
            if let Ok(module) = ModuleConfig::load(opts.repo_root, module_name) {
                if let Some(hb) = &module.homebrew {
                    let bf = opts.repo_root.join(&hb.brewfile);
                    if bf.exists() {
                        print_brew_header(&mut printed_header);
                        println!("  # {}", hb.brewfile);
                        list_brewfile(&bf, opts.repo_root)?;
                    }
                }
            }
        }

        if !printed_header && opts.show_brews {
            println!("[brew]");
            println!("  (none — run `haven brew install <package>` to track packages)");
        }
    }

    // ── AI skills ─────────────────────────────────────────────────────────────
    if show_ai {
        if let Some(skills_config) = crate::ai_skill::SkillsConfig::load(opts.repo_root)? {
            if !skills_config.skills.is_empty() {
                println!("[ai]");
                for skill in &skills_config.skills {
                    println!("  {}  ({})", skill.name, skill.source);
                }
            } else if opts.show_ai {
                println!("[ai]");
                println!("  (none)");
            }
        } else if opts.show_ai {
            println!("[ai]");
            println!("  (none — no ai/skills.toml found)");
        }
    }

    Ok(())
}

/// Print each entry from a Brewfile as `  <kind> <name>`.
fn list_brewfile(path: &Path, _repo_root: &Path) -> Result<()> {
    let contents = std::fs::read_to_string(path)?;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Lines look like: brew "ripgrep" or cask "iterm2"
        if let Some(rest) = line.strip_prefix("brew ") {
            let name = rest.trim().trim_matches('"');
            println!("  brew {}", name);
        } else if let Some(rest) = line.strip_prefix("cask ") {
            let name = rest.trim().trim_matches('"');
            println!("  cask {}", name);
        } else {
            // tap, mas, etc — show verbatim
            println!("  {}", line);
        }
    }
    Ok(())
}
