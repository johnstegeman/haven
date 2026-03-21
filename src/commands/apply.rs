/// Apply source files and packages to the destination.
///
/// File pipeline (scan-based — no TOML needed for file metadata):
///
///   scan source/ → decode magic names → create dirs → copy / render / symlink
///        │
///        ▼
///   stop on first error (files already applied stay applied)
///
/// Module pipeline (brew + AI only):
///
///   no --module  → brew bundle --file brew/Brewfile
///   --module foo → brew bundle --file brew/Brewfile.foo
///                  + install foo module's AI skills / commands
///                  + run foo module's mise
///
use anyhow::{Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};

use crate::config::{sort_modules, DfilesConfig, ModuleConfig};
use crate::config::module::expand_tilde;
use crate::fs::{apply_permissions, backup_file, copy_to_dest, write_to_dest};
use crate::ignore::IgnoreList;
use crate::source::{scan, SourceEntry};
use crate::state::{ModuleState, State};
use crate::template::TemplateContext;

pub struct ApplyOptions<'a> {
    pub repo_root: &'a Path,
    /// Where files are written. In tests this is a temp dir; in production it is `/`.
    pub dest_root: &'a Path,
    pub backup_dir: &'a Path,
    pub state_dir: &'a Path,
    pub claude_dir: &'a Path,
    pub profile: &'a str,
    /// When set, only apply this module's brew/AI config (files always apply in full).
    pub module_filter: Option<&'a str>,
    pub dry_run: bool,
    /// Apply source files to their destinations.
    pub apply_files: bool,
    /// Run brew bundle install (and optionally purge unreferenced packages).
    pub apply_brews: bool,
    /// Apply AI skills, commands, mise, and externals. Placeholder — not yet implemented.
    pub apply_ai: bool,
    /// When true, `git pull --ff-only` existing extdir_ clones in addition to cloning
    /// missing ones. By default existing clones are left as-is (idempotent).
    pub apply_externals: bool,
    /// After installing packages, uninstall any leaf formula or cask that is not
    /// referenced by any Brewfile in the active profile.
    pub remove_unreferenced_brews: bool,
    /// When true (combined with remove_unreferenced_brews), show the candidate list
    /// and prompt for confirmation before removing anything.
    pub interactive: bool,
}

pub fn run(opts: &ApplyOptions<'_>) -> Result<()> {
    let template_ctx = TemplateContext::from_env(opts.profile, opts.repo_root);
    let source_dir = opts.repo_root.join("source");

    // ── 1. Scan and apply all source files ───────────────────────────────────
    let ignore = IgnoreList::load(opts.repo_root);
    let entries = scan(&source_dir, &ignore)?;

    if opts.dry_run {
        let mut sections = Vec::new();
        if opts.apply_files  { sections.push("files"); }
        if opts.apply_brews  { sections.push("brews"); }
        if opts.apply_ai     { sections.push("ai"); }
        println!("Dry run — no files will be written.\n");
        println!("Profile:  {}", opts.profile);
        println!("Applying: {}", if sections.is_empty() { "nothing" } else { sections.join(", ").leak() });
        if let Some(m) = opts.module_filter {
            println!("Module:   {} (brew/AI only)", m);
        }
        println!();
    }

    // ── 1. Source files ───────────────────────────────────────────────────────
    let mut files_applied = 0usize;
    if opts.apply_files {
        if opts.dry_run {
            println!("[files]");
        }
        for entry in &entries {
            if opts.dry_run {
                print_dry_run_entry(entry, opts.dest_root);
                continue;
            }
            apply_entry(entry, opts, &template_ctx)?;
            files_applied += 1;
        }
        if opts.dry_run && entries.is_empty() {
            println!("  (no files in source/)");
        }
        if opts.dry_run {
            println!();
        }
    }

    // ── 2. Apply module brew / AI / mise / externals ─────────────────────────
    let modules_to_apply: Vec<String> = match opts.module_filter {
        Some(m) => vec![m.to_string()],
        None => DfilesConfig::load(opts.repo_root)?.resolve_modules(opts.profile)
            .unwrap_or_default(),
    };
    let sorted = sort_modules(&modules_to_apply);

    // Brew: apply master Brewfile when no filter, module brewfile when filtered.
    if opts.apply_brews {
        apply_brew(opts, &sorted)?;

        // Optionally purge unreferenced packages after installing.
        if opts.remove_unreferenced_brews || opts.interactive {
            purge_unreferenced_brews(opts, &sorted)?;
        }
    }

    let mut state = State::load(opts.state_dir)?;
    let mut lock = crate::lock::LockFile::load(opts.repo_root)?;
    let mut module_applied = 0usize;

    // AI / mise / externals — only when apply_ai is set.
    if opts.apply_ai {
        for module_name in &sorted {
            let module = ModuleConfig::load(opts.repo_root, module_name)?;
            if module.is_empty() {
                continue;
            }

            if opts.dry_run {
                print_dry_run_module(module_name, &module, opts);
                continue;
            }

            // ── 1Password guard ──────────────────────────────────────────────
            if module.requires_op {
                let op_ok = crate::onepassword::op_path()
                    .map(|p| crate::onepassword::is_authenticated(&p))
                    .unwrap_or(false);
                if !op_ok {
                    let reason = if crate::onepassword::op_path().is_none() {
                        "op CLI not installed"
                    } else {
                        "not signed into 1Password (run: op signin)"
                    };
                    eprintln!(
                        "warning: [{}] skipped — {}",
                        module_name, reason
                    );
                    continue;
                }
            }

            // ── AI skills / commands ─────────────────────────────────────────
            if let Some(ai) = &module.ai {
                for source_str in &ai.skills {
                    let source = crate::github::GhSource::parse(source_str)
                        .with_context(|| format!("Invalid AI skill source: {}", source_str))?;
                    let skills_dir = opts.claude_dir.join("skills");
                    print!("  Installing skill {}… ", source.name());
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                    match crate::github::fetch_to_dir(&source, &skills_dir) {
                        Ok(sha) => {
                            lock.pin(source_str, &sha);
                            println!("✓");
                            module_applied += 1;
                        }
                        Err(e) => {
                            println!("✗");
                            eprintln!(
                                "  warning: [{}] skill {} — fetch failed: {}",
                                module_name, source_str, e
                            );
                        }
                    }
                }
                for source_str in &ai.commands {
                    let source = crate::github::GhSource::parse(source_str)
                        .with_context(|| format!("Invalid AI command source: {}", source_str))?;
                    let commands_dir = opts.claude_dir.join("commands");
                    print!("  Installing command {}… ", source.name());
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                    match crate::github::fetch_to_dir(&source, &commands_dir) {
                        Ok(sha) => {
                            lock.pin(source_str, &sha);
                            println!("✓");
                            module_applied += 1;
                        }
                        Err(e) => {
                            println!("✗");
                            eprintln!(
                                "  warning: [{}] command {} — fetch failed: {}",
                                module_name, source_str, e
                            );
                        }
                    }
                }
            }

            // ── Mise ─────────────────────────────────────────────────────────
            if let Some(mise_cfg) = &module.mise {
                match crate::mise::mise_path() {
                    None => {
                        println!("  [mise] mise not found — install from https://mise.jdx.dev");
                    }
                    Some(mise) => {
                        let config_path = mise_cfg.config.as_ref().map(|c| opts.repo_root.join(c));
                        println!("  Installing mise tools…");
                        crate::mise::install_tools(&mise, config_path.as_deref())
                            .context("mise install failed")?;
                        println!("  ✓ mise tools installed");
                        module_applied += 1;
                    }
                }
            }

            state.modules.insert(
                module_name.clone(),
                ModuleState {
                    status: "clean".into(),
                    files: module_applied,
                },
            );
        }
    }

    if !opts.dry_run {
        if !lock.sources.is_empty() {
            if let Err(e) = lock.save(opts.repo_root) {
                eprintln!("warning: Could not write dfiles.lock: {}", e);
            }
        }
        if let Err(e) = crate::claude_md::generate(opts.claude_dir, opts.profile) {
            eprintln!("warning: CLAUDE.md generation failed: {}", e);
        }
        state.version = "1".into();
        state.last_apply = Some(Utc::now());
        state.profile = Some(opts.profile.to_string());
        state.hostname = crate::state::hostname();
        state.save(opts.state_dir)?;

        println!();
        println!(
            "Applied {} file(s) across {} module(s) — profile: {}",
            files_applied,
            sorted.len(),
            opts.profile
        );
    }

    Ok(())
}

// ─── Extdir marker ────────────────────────────────────────────────────────────

/// Parsed content of an `extdir_<name>` marker file in `source/`.
#[derive(serde::Deserialize)]
struct ExtdirContent {
    url: String,
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    #[serde(rename = "type", default = "default_extdir_type")]
    kind: String,
}

fn default_extdir_type() -> String {
    "git".to_string()
}

fn parse_extdir_content(src: &Path) -> Result<ExtdirContent> {
    let text = std::fs::read_to_string(src)
        .with_context(|| format!("Cannot read extdir marker {}", src.display()))?;
    let content: ExtdirContent = toml::from_str(&text)
        .with_context(|| format!("Invalid TOML in extdir marker {}", src.display()))?;
    Ok(content)
}

// ─── File application ─────────────────────────────────────────────────────────

fn apply_entry(
    entry: &SourceEntry,
    opts: &ApplyOptions<'_>,
    template_ctx: &TemplateContext,
) -> Result<()> {
    // Expand dest and rebase onto dest_root.
    let dest = resolve_dest(
        expand_tilde(&entry.dest_tilde)?,
        opts.dest_root,
    );

    // Ensure parent directories exist with correct permissions.
    for dir in &entry.dirs {
        let dir_path = resolve_dest(expand_tilde(&dir.dest_tilde)?, opts.dest_root);
        if !dir_path.exists() {
            std::fs::create_dir_all(&dir_path)
                .with_context(|| format!("Cannot create directory {}", dir_path.display()))?;
            if dir.flags.private {
                apply_permissions(&dir_path, true, false)
                    .with_context(|| format!("Cannot set permissions on {}", dir_path.display()))?;
            }
        }
    }

    if entry.flags.extdir {
        let content = parse_extdir_content(&entry.src)
            .with_context(|| format!("Bad extdir marker: {}", entry.src.display()))?;
        if content.kind != "git" {
            anyhow::bail!(
                "extdir type '{}' is not supported (only 'git'): {}",
                content.kind,
                entry.src.display()
            );
        }
        apply_git_external(
            &content.url,
            content.ref_name.as_deref(),
            &dest,
            opts.apply_externals,
        )?;
        println!("  ✓ {}", dest.display());
        return Ok(());
    }

    if entry.flags.symlink {
        if entry.flags.private || entry.flags.executable {
            eprintln!(
                "warning: private/executable flags are ignored for symlink entries ({})",
                entry.dest_tilde
            );
        }
        // For symlink+template: render the file content to get the target path.
        let link_target = if entry.flags.template {
            let source_text = std::fs::read_to_string(&entry.src)
                .with_context(|| format!("Cannot read template {}", entry.src.display()))?;
            let rendered = crate::template::render(&source_text, template_ctx)
                .with_context(|| format!("Cannot render template '{}'", entry.src.display()))?;
            PathBuf::from(rendered.trim())
        } else {
            entry.src.clone()
        };
        let backup = apply_symlink(&link_target, &dest, opts.backup_dir)
            .with_context(|| {
                format!("Cannot link {} → {}", dest.display(), link_target.display())
            })?;
        if let Some(b) = backup {
            println!("  backed up {} → {}", dest.display(), b.display());
        }
        println!("  ✓ {} ⟶ {}", dest.display(), link_target.display());
        return Ok(());
    }

    // Back up existing file before overwriting.
    if dest.exists() {
        let backup = backup_file(&dest, opts.backup_dir)
            .with_context(|| format!("Cannot back up {}", dest.display()))?;
        println!("  backed up {} → {}", dest.display(), backup.display());
    }

    if entry.flags.template {
        let source_text = std::fs::read_to_string(&entry.src)
            .with_context(|| format!("Cannot read template {}", entry.src.display()))?;
        let rendered = crate::template::render(&source_text, template_ctx)
            .with_context(|| format!("Cannot render template '{}'", entry.src.display()))?;
        write_to_dest(&rendered, &dest)
            .with_context(|| format!("Cannot write rendered file to {}", dest.display()))?;
    } else {
        copy_to_dest(&entry.src, &dest)
            .with_context(|| format!("Cannot copy {} → {}", entry.src.display(), dest.display()))?;
    }

    if entry.flags.private || entry.flags.executable {
        apply_permissions(&dest, entry.flags.private, entry.flags.executable)
            .with_context(|| format!("Cannot set permissions on {}", dest.display()))?;
    }

    println!("  ✓ {}", dest.display());
    Ok(())
}

fn print_dry_run_entry(entry: &SourceEntry, dest_root: &Path) {
    use crate::config::module::expand_tilde;
    let dest = match expand_tilde(&entry.dest_tilde) {
        Ok(p) => resolve_dest(p, dest_root),
        Err(_) => PathBuf::from(&entry.dest_tilde),
    };

    if entry.flags.extdir {
        match parse_extdir_content(&entry.src) {
            Ok(content) => {
                let ref_hint = content.ref_name.as_deref().unwrap_or("default branch");
                println!(
                    "  [extdir] clone {} → {}  ({})",
                    content.url,
                    dest.display(),
                    ref_hint
                );
            }
            Err(_) => {
                println!("  [extdir] {}  (marker unreadable)", dest.display());
            }
        }
        return;
    }

    let mut tags: Vec<&str> = Vec::new();
    if entry.flags.template  { tags.push("template"); }
    if entry.flags.private   { tags.push("private"); }
    if entry.flags.executable { tags.push("executable"); }
    if entry.flags.symlink   { tags.push("symlink"); }
    let annotation = if tags.is_empty() {
        String::new()
    } else {
        format!("  ({})", tags.join(", "))
    };
    let src_rel = entry.src.file_name().unwrap_or(entry.src.as_os_str());
    println!(
        "  source/{} → {}{}",
        entry.src
            .components()
            .last()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_default(),
        dest.display(),
        annotation,
    );
    // Print as the encoded relative path for clarity
    let _ = src_rel; // suppress warning
}

// ─── Brew ─────────────────────────────────────────────────────────────────────

fn apply_brew(opts: &ApplyOptions<'_>, sorted_modules: &[String]) -> Result<()> {
    let brewfile_path = if let Some(module) = opts.module_filter {
        // --module specified: use that module's brewfile.
        let config = ModuleConfig::load(opts.repo_root, module)?;
        config.homebrew.map(|hb| opts.repo_root.join(&hb.brewfile))
    } else {
        // No module filter: use the master Brewfile if it exists.
        let master = opts.repo_root.join("brew").join("Brewfile");
        if master.exists() { Some(master) } else { None }
    };

    let brewfile = match brewfile_path {
        None => return Ok(()),
        Some(p) => p,
    };
    if !brewfile.exists() {
        return Ok(());
    }

    if opts.dry_run {
        println!(
            "[brew] brew bundle --file {}",
            brewfile.strip_prefix(opts.repo_root).unwrap_or(&brewfile).display()
        );
        println!();
        return Ok(());
    }

    match crate::homebrew::ensure_brew(false)? {
        None => {
            println!("[brew] skipped (brew not available)");
        }
        Some(brew) => {
            println!(
                "Installing packages from {}…",
                brewfile.strip_prefix(opts.repo_root).unwrap_or(&brewfile).display()
            );
            crate::homebrew::bundle_install(&brew, &brewfile)
                .with_context(|| {
                    format!("brew bundle install failed for {}", brewfile.display())
                })?;
            println!("  ✓ brew bundle");
        }
    }
    let _ = sorted_modules;
    Ok(())
}

fn print_dry_run_module(module_name: &str, module: &ModuleConfig, _opts: &ApplyOptions<'_>) {
    let mut has_output = false;

    if let Some(hb) = &module.homebrew {
        if !has_output { println!("[{}]", module_name); has_output = true; }
        println!(
            "  brew bundle --file {}",
            hb.brewfile
        );
    }
    if let Some(mise_cfg) = &module.mise {
        if !has_output { println!("[{}]", module_name); has_output = true; }
        let config_hint = mise_cfg.config.as_ref()
            .map(|c| format!(" --config-file {}", c))
            .unwrap_or_default();
        println!("  mise install{}", config_hint);
    }
    if let Some(ai) = &module.ai {
        for s in &ai.skills {
            if !has_output { println!("[{}]", module_name); has_output = true; }
            println!("  fetch skill: {}", s);
        }
        for s in &ai.commands {
            if !has_output { println!("[{}]", module_name); has_output = true; }
            println!("  fetch command: {}", s);
        }
    }
    if has_output { println!(); }
}

// ─── Symlink helper ───────────────────────────────────────────────────────────

fn apply_symlink(
    source_abs: &Path,
    dest: &Path,
    backup_dir: &Path,
) -> Result<Option<PathBuf>> {
    // Fast path: already the correct symlink.
    if dest.is_symlink() {
        if let Ok(target) = std::fs::read_link(dest) {
            if target == source_abs {
                return Ok(None);
            }
        }
    }

    let backup_path = if dest.is_symlink() || dest.exists() {
        if dest.is_dir() && !dest.is_symlink() {
            anyhow::bail!(
                "{} is a directory — cannot replace with a symlink",
                dest.display()
            );
        }
        let b = backup_file(dest, backup_dir)
            .with_context(|| format!("Cannot back up {}", dest.display()))?;
        std::fs::remove_file(dest)
            .with_context(|| format!("Cannot remove {}", dest.display()))?;
        Some(b)
    } else {
        None
    };

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create parent dir {}", parent.display()))?;
    }

    #[cfg(unix)]
    std::os::unix::fs::symlink(source_abs, dest)
        .with_context(|| {
            format!("Cannot create symlink {} → {}", dest.display(), source_abs.display())
        })?;

    #[cfg(not(unix))]
    anyhow::bail!("Symlink entries are not supported on non-Unix platforms");

    Ok(backup_path)
}

// ─── Git external helper ─────────────────────────────────────────────────────

fn apply_git_external(url: &str, ref_name: Option<&str>, dest: &Path, pull_if_exists: bool) -> Result<()> {
    if dest.exists() {
        let git_dir = dest.join(".git");
        if !git_dir.exists() {
            anyhow::bail!(
                "{} already exists and is not a git repository",
                dest.display()
            );
        }
        if pull_if_exists {
            println!("  Pulling {}…", dest.display());
            let status = std::process::Command::new("git")
                .args(["-C", &dest.to_string_lossy(), "pull", "--ff-only"])
                .status()
                .context("Failed to run git pull")?;
            if !status.success() {
                anyhow::bail!("git pull --ff-only failed in {}", dest.display());
            }
        }
        // else: already present, skip silently (caller prints ✓)
    } else {
        println!("  Cloning {} → {}…", url, dest.display());
        let mut cmd = std::process::Command::new("git");
        cmd.arg("clone").arg("--depth").arg("1");
        if let Some(r) = ref_name {
            cmd.args(["--branch", r]);
        }
        cmd.arg(url).arg(dest);
        let status = cmd.status().context("Failed to run git clone")?;
        if !status.success() {
            anyhow::bail!("git clone failed for {}", url);
        }
    }
    Ok(())
}

// ─── Unreferenced brew purge ──────────────────────────────────────────────────

/// Collect the paths of all Brewfiles declared for the active profile:
/// the master `brew/Brewfile` plus any per-module Brewfile for every module
/// in `sorted_modules`. Missing files are silently skipped.
fn profile_brewfile_paths(opts: &ApplyOptions<'_>, sorted_modules: &[String]) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    let master = opts.repo_root.join("brew").join("Brewfile");
    if master.exists() {
        paths.push(master);
    }

    for module_name in sorted_modules {
        if let Ok(cfg) = ModuleConfig::load(opts.repo_root, module_name) {
            if let Some(hb) = cfg.homebrew {
                let p = opts.repo_root.join(&hb.brewfile);
                if p.exists() && !paths.contains(&p) {
                    paths.push(p);
                }
            }
        }
    }

    paths
}

/// After `brew bundle install`, find any leaf formula or installed cask that is
/// not declared in any Brewfile for the active profile, then remove them.
///
/// Flow:
///   collect profile Brewfile entries (formulas + casks)
///   brew leaves       → installed leaf formulas not needed as deps
///   brew list --cask  → all installed casks
///   diff              → unreferenced = installed − declared
///   interactive?      → show list, prompt [y/N]
///   remove each one via brew uninstall / brew uninstall --cask
fn purge_unreferenced_brews(opts: &ApplyOptions<'_>, sorted_modules: &[String]) -> Result<()> {
    let brew = match crate::homebrew::brew_path() {
        Some(b) => b,
        None => {
            if !opts.dry_run {
                eprintln!("note: brew not found — skipping unreferenced package check");
            }
            return Ok(());
        }
    };

    // Collect every Brewfile path for the active profile.
    let brewfile_paths = profile_brewfile_paths(opts, sorted_modules);
    let path_refs: Vec<&std::path::Path> = brewfile_paths.iter().map(|p| p.as_path()).collect();

    // Compute which installed packages are not declared using the shared helper.
    let diff = crate::homebrew::brewfile_diff(&brew, &path_refs)?;
    let unreferenced_formulas = diff.extra_formulas;
    let unreferenced_casks = diff.extra_casks;

    let total = unreferenced_formulas.len() + unreferenced_casks.len();

    if total == 0 {
        println!("\n✓ No unreferenced packages found.");
        return Ok(());
    }

    // Display the candidate list.
    println!(
        "\nThe following package{} installed but not in any Brewfile for profile '{}':",
        if total == 1 { " is" } else { "s are" },
        opts.profile
    );
    if !unreferenced_formulas.is_empty() {
        println!("  Formulas:");
        for name in &unreferenced_formulas {
            println!("    {}", name);
        }
    }
    if !unreferenced_casks.is_empty() {
        println!("  Casks:");
        for name in &unreferenced_casks {
            println!("    {}", name);
        }
    }

    if opts.dry_run {
        println!(
            "\n  (dry run — {} package{} would be removed)",
            total,
            if total == 1 { "" } else { "s" }
        );
        return Ok(());
    }

    // Interactive: ask before removing.
    if opts.interactive {
        use std::io::Write;
        print!("\nRemove {} package{}? [y/N] ", total, if total == 1 { "" } else { "s" });
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("Skipped.");
            return Ok(());
        }
    }

    // Remove.
    for name in &unreferenced_formulas {
        print!("  Removing formula {}… ", name);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        crate::homebrew::brew_uninstall(&brew, name, false)
            .with_context(|| format!("Failed to uninstall formula '{}'", name))?;
        println!("✓");
    }
    for name in &unreferenced_casks {
        print!("  Removing cask {}… ", name);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        crate::homebrew::brew_uninstall(&brew, name, true)
            .with_context(|| format!("Failed to uninstall cask '{}'", name))?;
        println!("✓");
    }

    println!(
        "\n✓ Removed {} unreferenced package{}.",
        total,
        if total == 1 { "" } else { "s" }
    );
    Ok(())
}

// ─── Dest resolution ─────────────────────────────────────────────────────────

fn resolve_dest(dest: PathBuf, dest_root: &Path) -> PathBuf {
    if dest_root == Path::new("/") {
        dest
    } else {
        let rel = dest.strip_prefix("/").unwrap_or(&dest);
        dest_root.join(rel)
    }
}
