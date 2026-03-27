/// List all tracked items: files, brews, and AI skills.
///
/// By default shows all sections. Use --files, --brews, or --ai to filter.
/// Use --filter to show only entries containing a substring, and --count to
/// print the total count instead of individual entries.
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
    /// Case-insensitive substring filter applied to each entry's path/name.
    pub filter: Option<&'a str>,
    /// When true, print only the total count of matching entries.
    pub count: bool,
}

pub fn run(opts: &ListOptions<'_>) -> Result<()> {
    let none_specified = !opts.show_files && !opts.show_brews && !opts.show_ai;
    let show_files = opts.show_files || none_specified;
    let show_brews = opts.show_brews || none_specified;
    let show_ai    = opts.show_ai   || none_specified;

    let filter_lc = opts.filter.map(|f| f.to_lowercase());
    let matches = |s: &str| -> bool {
        filter_lc.as_deref().map_or(true, |f| s.to_lowercase().contains(f))
    };

    let config = HavenConfig::load(opts.repo_root).unwrap_or_default();
    let modules = config.resolve_modules(opts.profile)?;
    let sorted = sort_modules(&modules);

    let mut total_count: usize = 0;

    // ── Files ─────────────────────────────────────────────────────────────────
    if show_files {
        let source_dir = opts.repo_root.join("source");
        let ctx = TemplateContext::from_env_for_repo(opts.repo_root);
        let ignore = IgnoreList::load(opts.repo_root, &ctx);
        let entries = source::scan(&source_dir, &ignore)?;

        let matched: Vec<_> = entries.iter().filter(|e| matches(&e.dest_tilde)).collect();

        if opts.count {
            total_count += matched.len();
        } else if !matched.is_empty() {
            if !none_specified {
                println!("[files]");
            }
            for entry in &matched {
                let mut tags: Vec<&str> = Vec::new();
                if entry.flags.template    { tags.push("template"); }
                if entry.flags.symlink     { tags.push("symlink"); }
                if entry.flags.private     { tags.push("private"); }
                if entry.flags.executable  { tags.push("executable"); }
                if entry.flags.extdir      { tags.push("extdir"); }
                if entry.flags.extfile     { tags.push("extfile"); }
                if entry.flags.create_only { tags.push("create-only"); }
                if entry.flags.exact       { tags.push("exact"); }

                let indent = if none_specified { "" } else { "  " };
                if tags.is_empty() {
                    println!("{}{}", indent, entry.dest_tilde);
                } else {
                    println!("{}{}  ({})", indent, entry.dest_tilde, tags.join(", "));
                }
            }
        } else if opts.show_files && filter_lc.is_none() {
            println!("[files]");
            println!("  (none — run `haven add <file>` to start tracking)");
        }
    }

    // ── Brews ─────────────────────────────────────────────────────────────────
    if show_brews {
        let brew_lines = collect_brew_lines(opts.repo_root, &sorted)?;
        let matched: Vec<_> = brew_lines.iter().filter(|(s, _)| matches(s)).collect();

        if opts.count {
            total_count += matched.len();
        } else {
            let mut printed_header = false;
            let mut last_section: Option<&str> = None;

            for (name, section_comment) in &matched {
                if !printed_header {
                    println!("[brew]");
                    printed_header = true;
                }
                if let Some(sc) = section_comment {
                    if last_section.as_deref() != Some(sc.as_str()) {
                        println!("  # {}", sc);
                        last_section = Some(sc.as_str());
                    }
                }
                println!("  {}", name);
            }

            if !printed_header && opts.show_brews && filter_lc.is_none() {
                println!("[brew]");
                println!("  (none — run `haven brew install <package>` to track packages)");
            }
        }
    }

    // ── AI skills ─────────────────────────────────────────────────────────────
    if show_ai {
        if let Some(skills_config) = crate::ai_skill::SkillsConfig::load(opts.repo_root)? {
            let matched: Vec<_> = skills_config
                .skills
                .iter()
                .filter(|s| matches(&s.name) || matches(&s.source))
                .collect();

            if opts.count {
                total_count += matched.len();
            } else if !matched.is_empty() {
                println!("[ai]");
                for skill in &matched {
                    println!("  {}  ({})", skill.name, skill.source);
                }
            } else if opts.show_ai && filter_lc.is_none() {
                println!("[ai]");
                if skills_config.skills.is_empty() {
                    println!("  (none)");
                }
            }
        } else if opts.show_ai && filter_lc.is_none() {
            println!("[ai]");
            println!("  (none — no ai/skills.toml found)");
        }
    }

    if opts.count {
        println!("{}", total_count);
    }

    Ok(())
}

/// Collect all brew entries from all Brewfiles as `(display_string, section_comment)` pairs.
///
/// `section_comment` is `Some(brewfile_path)` for module Brewfiles (used to print
/// section headers), or `None` for the master Brewfile.
fn collect_brew_lines(
    repo_root: &Path,
    sorted_modules: &[String],
) -> Result<Vec<(String, Option<String>)>> {
    let mut out = Vec::new();

    let master = repo_root.join("brew").join("Brewfile");
    if master.exists() {
        for line in read_brewfile_entries(&master)? {
            out.push((line, None));
        }
    }

    for module_name in sorted_modules {
        if let Ok(module) = ModuleConfig::load(repo_root, module_name) {
            if let Some(hb) = &module.homebrew {
                let bf = repo_root.join(&hb.brewfile);
                if bf.exists() {
                    for line in read_brewfile_entries(&bf)? {
                        out.push((line, Some(hb.brewfile.clone())));
                    }
                }
            }
        }
    }

    Ok(out)
}

/// Parse a Brewfile and return display strings for each entry.
fn read_brewfile_entries(path: &Path) -> Result<Vec<String>> {
    let contents = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("brew ") {
            out.push(format!("brew {}", rest.trim().trim_matches('"')));
        } else if let Some(rest) = line.strip_prefix("cask ") {
            out.push(format!("cask {}", rest.trim().trim_matches('"')));
        } else {
            out.push(line.to_string());
        }
    }
    Ok(out)
}
