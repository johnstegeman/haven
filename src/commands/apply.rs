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
}

pub fn run(opts: &ApplyOptions<'_>) -> Result<()> {
    let template_ctx = TemplateContext::from_env(opts.profile, opts.repo_root);
    let source_dir = opts.repo_root.join("source");

    // ── 1. Scan and apply all source files ───────────────────────────────────
    let entries = scan(&source_dir)?;

    if opts.dry_run {
        println!("Dry run — no files will be written.\n");
        println!("Profile: {}", opts.profile);
        if let Some(m) = opts.module_filter {
            println!("Module:  {} (brew/AI only)", m);
        }
        println!();
        println!("[files]");
    }

    let mut files_applied = 0usize;
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

    // ── 2. Apply module brew / AI / mise / externals ─────────────────────────
    let modules_to_apply: Vec<String> = match opts.module_filter {
        Some(m) => vec![m.to_string()],
        None => DfilesConfig::load(opts.repo_root)?.resolve_modules(opts.profile)
            .unwrap_or_default(),
    };
    let sorted = sort_modules(&modules_to_apply);

    // Brew: apply master Brewfile when no filter, module brewfile when filtered.
    apply_brew(opts, &sorted)?;

    let mut state = State::load(opts.state_dir)?;
    let mut lock = crate::lock::LockFile::load(opts.repo_root)?;
    let mut module_applied = 0usize;

    for module_name in &sorted {
        let module = ModuleConfig::load(opts.repo_root, module_name)?;
        if module.is_empty() {
            continue;
        }

        if opts.dry_run {
            print_dry_run_module(module_name, &module, opts);
            continue;
        }

        // ── 1Password guard ──────────────────────────────────────────────────
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

        // ── Externals ────────────────────────────────────────────────────────
        for ext in &module.externals {
            let dest = resolve_dest(ext.dest_expanded()?, opts.dest_root);
            apply_git_external(&ext.url, ext.ref_name.as_deref(), &dest)
                .with_context(|| {
                    format!("External failed: {} → {}", ext.url, dest.display())
                })?;
            println!("  ✓ {}", dest.display());
            module_applied += 1;
        }

        // ── AI skills / commands ─────────────────────────────────────────────
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

        // ── Mise ─────────────────────────────────────────────────────────────
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

    if entry.flags.symlink {
        if entry.flags.private || entry.flags.executable {
            eprintln!(
                "warning: private/executable flags are ignored for symlink entries ({})",
                entry.dest_tilde
            );
        }
        // Symlink entry: dest → source file (source is the authoritative copy).
        let backup = apply_symlink(&entry.src, &dest, opts.backup_dir)
            .with_context(|| {
                format!("Cannot link {} → {}", dest.display(), entry.src.display())
            })?;
        if let Some(b) = backup {
            println!("  backed up {} → {}", dest.display(), b.display());
        }
        println!("  ✓ {} ⟶ {}", dest.display(), entry.src.display());
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

fn print_dry_run_module(module_name: &str, module: &ModuleConfig, opts: &ApplyOptions<'_>) {
    let mut has_output = false;

    if let Some(hb) = &module.homebrew {
        if !has_output { println!("[{}]", module_name); has_output = true; }
        println!(
            "  brew bundle --file {}",
            hb.brewfile
        );
    }
    for ext in &module.externals {
        if !has_output { println!("[{}]", module_name); has_output = true; }
        let dest = match expand_tilde(&ext.dest) {
            Ok(p) => resolve_dest(p, opts.dest_root),
            Err(_) => PathBuf::from(&ext.dest),
        };
        let ref_label = ext.ref_name.as_deref().unwrap_or("default branch");
        println!("  git clone {}  → {}  ({})", ext.url, dest.display(), ref_label);
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

fn apply_git_external(url: &str, ref_name: Option<&str>, dest: &Path) -> Result<()> {
    if dest.exists() {
        let git_dir = dest.join(".git");
        if !git_dir.exists() {
            anyhow::bail!(
                "{} already exists and is not a git repository",
                dest.display()
            );
        }
        println!("  Pulling {}…", dest.display());
        let status = std::process::Command::new("git")
            .args(["-C", &dest.to_string_lossy(), "pull", "--ff-only"])
            .status()
            .context("Failed to run git pull")?;
        if !status.success() {
            anyhow::bail!("git pull --ff-only failed in {}", dest.display());
        }
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

// ─── Dest resolution ─────────────────────────────────────────────────────────

fn resolve_dest(dest: PathBuf, dest_root: &Path) -> PathBuf {
    if dest_root == Path::new("/") {
        dest
    } else {
        let rel = dest.strip_prefix("/").unwrap_or(&dest);
        dest_root.join(rel)
    }
}
