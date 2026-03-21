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
use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ai_platform::{PlatformPlugin, PlatformsConfig};
use crate::ai_skill::{SkillDeclaration, SkillSource, SkillsConfig};
use crate::github::GhSource;
use crate::config::{sort_modules, DfilesConfig, ModuleConfig};
use crate::config::module::expand_tilde;
use crate::fs::{apply_permissions, backup_file, copy_to_dest, write_to_dest};
use crate::ignore::IgnoreList;
use crate::skill_cache::SkillCache;
use crate::source::{scan, scan_scripts, ScriptExecWhen, SourceEntry};
use crate::state::{AiDeployedEntry, ModuleState, State};
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
    /// Apply AI skills (from ai/skills.toml) and legacy module [ai] sections.
    pub apply_ai: bool,
    /// When true, `git pull --ff-only` existing extdir_ clones in addition to cloning
    /// missing ones. By default existing clones are left as-is (idempotent).
    pub apply_externals: bool,
    /// When true, execute scripts from `source/scripts/` during apply.
    /// `run_once_` scripts are only executed if they haven't run on this machine yet.
    /// Without this flag, scripts are never executed (opt-in for safety).
    pub run_scripts: bool,
    /// After installing packages, uninstall any leaf formula or cask that is not
    /// referenced by any Brewfile in the active profile.
    pub remove_unreferenced_brews: bool,
    /// When true (combined with remove_unreferenced_brews), show the candidate list
    /// and prompt for confirmation before removing anything.
    pub interactive: bool,
}

/// RAII guard that holds `~/.dfiles/apply.lock` for the duration of apply.
struct ApplyLock {
    path: PathBuf,
}

impl ApplyLock {
    fn acquire(state_dir: &Path) -> Result<Self> {
        let path = state_dir.join("apply.lock");
        if let Ok(contents) = std::fs::read_to_string(&path) {
            // Check whether the PID recorded in the lock file is still alive.
            if let Ok(pid) = contents.trim().parse::<u32>() {
                let alive = std::path::Path::new(&format!("/proc/{}/status", pid)).exists()
                    || {
                        // On macOS /proc doesn't exist; use kill(pid, 0) via ps.
                        std::process::Command::new("kill")
                            .args(["-0", &pid.to_string()])
                            .output()
                            .map(|o| o.status.success())
                            .unwrap_or(false)
                    };
                if alive {
                    bail!(
                        "dfiles apply is already running (PID {}). \
                         If this is wrong, delete {}",
                        pid,
                        path.display()
                    );
                }
                // Stale lock — remove it and continue.
                let _ = std::fs::remove_file(&path);
            }
        }
        std::fs::create_dir_all(state_dir).context("Cannot create state directory")?;
        std::fs::write(&path, std::process::id().to_string())
            .with_context(|| format!("Cannot write lock file {}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for ApplyLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn run(opts: &ApplyOptions<'_>) -> Result<()> {
    // Prevent two simultaneous `dfiles apply` runs from racing on state.json.
    let _lock = if !opts.dry_run {
        Some(ApplyLock::acquire(opts.state_dir)?)
    } else {
        None
    };

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

        // Enforce exact directories: remove untracked entries from dirs declared exact_.
        if !opts.dry_run {
            let exact_dirs = collect_exact_dirs(&entries, opts.dest_root);
            for (dir_path, tracked) in &exact_dirs {
                purge_exact_dir(dir_path, tracked, opts.backup_dir)?;
            }
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

    // ── 3. AI skills (ai/skills.toml) ────────────────────────────────────────
    if opts.apply_ai {
        if opts.dry_run {
            if let Some(skills_config) = SkillsConfig::load(opts.repo_root)? {
                if !skills_config.skills.is_empty() {
                    println!("[ai]");
                    for skill in &skills_config.skills {
                        println!("  fetch skill: {}", skill.source);
                    }
                    println!();
                }
            }
        } else {
            apply_ai_skills(opts, &mut state, &mut lock)?;
        }
    }

    // ── 4. Run scripts from source/scripts/ ──────────────────────────────────
    if opts.run_scripts && !opts.dry_run {
        let scripts_dir = opts.repo_root.join("source").join("scripts");
        let script_entries = scan_scripts(&scripts_dir)
            .context("Cannot scan source/scripts/")?;
        apply_scripts(&script_entries, &mut state)?;
    }

    if !opts.dry_run {
        if !lock.sources.is_empty() || !lock.skill.is_empty() {
            if let Err(e) = lock.save(opts.repo_root) {
                eprintln!("warning: Could not write dfiles.lock: {}", e);
            }
        }
        // Inject skill snippets into platform config files (e.g. CLAUDE.md).
        let inj_skills = SkillsConfig::load(opts.repo_root)
            .ok()
            .flatten()
            .map(|c| c.skills)
            .unwrap_or_default();
        let inj_platforms = PlatformsConfig::load(opts.repo_root)
            .ok()
            .flatten()
            .and_then(|c| c.resolve_active_platforms().ok())
            .unwrap_or_default();
        if let Err(e) = crate::config_injection::inject_managed_sections(
            opts.repo_root,
            &inj_skills,
            &inj_platforms,
            &mut state,
            opts.interactive,
            false,
        ) {
            eprintln!("warning: config injection failed: {}", e);
        }
        // Regenerate CLAUDE.md with updated skills/commands listing.
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

// ─── Extfile marker ───────────────────────────────────────────────────────────

/// Parsed content of an `extfile_<name>` marker file in `source/`.
///
/// ```toml
/// type   = "file"          # "file" (default) or "archive" (.tar.gz / .tgz)
/// url    = "https://..."   # required: download URL
/// ref    = "v1.0"          # optional: version label for display / changelog
/// sha256 = "abc123..."     # optional: hex SHA-256 of the downloaded content
/// ```
#[derive(serde::Deserialize)]
struct ExtfileContent {
    url: String,
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    #[serde(rename = "type", default = "default_extfile_type")]
    kind: String,
    sha256: Option<String>,
}

fn default_extfile_type() -> String {
    "file".to_string()
}

fn parse_extfile_content(src: &Path) -> Result<ExtfileContent> {
    let text = std::fs::read_to_string(src)
        .with_context(|| format!("Cannot read extfile marker {}", src.display()))?;
    let content: ExtfileContent = toml::from_str(&text)
        .with_context(|| format!("Invalid TOML in extfile marker {}", src.display()))?;
    Ok(content)
}

/// Download an `extfile_` source and write it to `dest`.
///
/// - `kind = "file"`: writes the downloaded bytes directly to `dest`.
/// - `kind = "archive"`: extracts the tarball to `dest` (treated as a directory).
///
/// If `sha256` is set in the marker, the downloaded content is verified
/// before writing. A mismatch is a hard error.
fn apply_extfile_entry(content: &ExtfileContent, dest: &Path, backup_dir: &Path) -> Result<()> {
    use sha2::{Digest, Sha256};

    println!("  Downloading {}…", content.url);
    let bytes = crate::github::download_bytes(&content.url)
        .with_context(|| format!("Failed to download {}", content.url))?;

    // Verify SHA-256 if provided.
    if let Some(expected_hex) = &content.sha256 {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let actual_hex = format!("{:x}", hasher.finalize());
        if actual_hex != expected_hex.to_lowercase() {
            anyhow::bail!(
                "extfile SHA-256 mismatch for {}:\n  expected: {}\n  actual:   {}\n\
                 Update the sha256 field in your extfile_ marker to accept the new content.",
                dest.display(),
                expected_hex,
                actual_hex,
            );
        }
    }

    match content.kind.as_str() {
        "file" => {
            // Back up existing file.
            if dest.exists() {
                let backup = crate::fs::backup_file(dest, backup_dir)
                    .with_context(|| format!("Cannot back up {}", dest.display()))?;
                println!("  backed up {} → {}", dest.display(), backup.display());
            }
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(dest, &bytes)
                .with_context(|| format!("Cannot write {}", dest.display()))?;
        }
        "archive" => {
            // Extract tarball to dest directory.
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::create_dir_all(dest)
                .with_context(|| format!("Cannot create dest dir {}", dest.display()))?;
            crate::github::extract_tarball(&bytes, None, dest)
                .with_context(|| format!("Cannot extract archive to {}", dest.display()))?;
        }
        other => {
            anyhow::bail!(
                "extfile type '{}' is not supported (only 'file' or 'archive'): {}",
                other,
                dest.display()
            );
        }
    }

    Ok(())
}

// ─── Exact directory enforcement ──────────────────────────────────────────────

/// Walk all source entries and build a map from each `exact_`-declared destination
/// directory to the set of direct-child names that are tracked in source/.
///
/// Only direct children (one level deep) of each exact dir are collected.
/// Subdirectory names are tracked the same as file names — both are protected.
fn collect_exact_dirs(entries: &[SourceEntry], dest_root: &Path) -> HashMap<PathBuf, HashSet<String>> {
    let mut exact_dirs: HashMap<PathBuf, HashSet<String>> = HashMap::new();

    for entry in entries {
        if entry.flags.extdir { continue; }

        for (idx, dir) in entry.dirs.iter().enumerate() {
            if !dir.flags.exact { continue; }

            let exact_dir_path = match expand_tilde(&dir.dest_tilde) {
                Ok(p) => resolve_dest(p, dest_root),
                Err(_) => continue,
            };

            // The direct child is either the next dir in the chain, or the file itself.
            let direct_child_tilde = if idx + 1 < entry.dirs.len() {
                &entry.dirs[idx + 1].dest_tilde
            } else {
                &entry.dest_tilde
            };

            let child_path = match expand_tilde(direct_child_tilde) {
                Ok(p) => resolve_dest(p, dest_root),
                Err(_) => continue,
            };

            if child_path.parent() == Some(&exact_dir_path) {
                if let Some(name) = child_path.file_name().map(|n| n.to_string_lossy().to_string()) {
                    exact_dirs.entry(exact_dir_path).or_default().insert(name);
                }
            }
        }
    }

    exact_dirs
}

/// Delete (with backup) any entry inside `dir_path` whose name is not in `tracked`.
///
/// Only regular files are deleted. Directories are never removed — this matches
/// chezmoi's behaviour and avoids accidentally deleting deeply-nested content.
/// Does nothing if `dir_path` does not exist.
fn purge_exact_dir(dir_path: &Path, tracked: &HashSet<String>, backup_dir: &Path) -> Result<()> {
    if !dir_path.exists() {
        return Ok(());
    }

    for dent in std::fs::read_dir(dir_path)
        .with_context(|| format!("Cannot read exact dir {}", dir_path.display()))?
    {
        let dent = dent?;
        // Only remove regular files, never directories.
        if !dent.file_type()?.is_file() {
            continue;
        }
        let name = dent.file_name().to_string_lossy().to_string();
        if !tracked.contains(&name) {
            let path = dent.path();
            let backup = backup_file(&path, backup_dir)
                .with_context(|| format!("Cannot back up {}", path.display()))?;
            std::fs::remove_file(&path)
                .with_context(|| format!("Cannot remove untracked file {}", path.display()))?;
            println!("  [exact] removed {} → backed up to {}", path.display(), backup.display());
        }
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

    if entry.flags.extfile {
        let content = parse_extfile_content(&entry.src)
            .with_context(|| format!("Bad extfile marker: {}", entry.src.display()))?;
        apply_extfile_entry(&content, &dest, opts.backup_dir)?;
        // chmod +x when executable_ prefix is set.
        if entry.flags.executable && content.kind == "file" {
            apply_permissions(&dest, false, true)
                .with_context(|| format!("Cannot set permissions on {}", dest.display()))?;
        }
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

    // create_only: seed-only file — don't overwrite if destination already exists.
    if entry.flags.create_only && dest.exists() {
        println!("  ~ {} (create_only — already exists, not overwritten)", dest.display());
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

    if entry.flags.extfile {
        match parse_extfile_content(&entry.src) {
            Ok(content) => {
                let ref_hint = content.ref_name.as_deref().unwrap_or("latest");
                let type_hint = if content.kind == "archive" { "extract" } else { "download" };
                println!(
                    "  [extfile] {} {} → {}  ({})",
                    type_hint,
                    content.url,
                    dest.display(),
                    ref_hint
                );
            }
            Err(_) => {
                println!("  [extfile] {}  (marker unreadable)", dest.display());
            }
        }
        return;
    }

    let mut tags: Vec<&str> = Vec::new();
    if entry.flags.template    { tags.push("template"); }
    if entry.flags.private     { tags.push("private"); }
    if entry.flags.executable  { tags.push("executable"); }
    if entry.flags.symlink     { tags.push("symlink"); }
    if entry.flags.create_only { tags.push("create_only"); }
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

// ─── Script execution ─────────────────────────────────────────────────────────

/// Execute scripts from `source/scripts/`.
///
/// `run_once_` / `once_` scripts are executed only if they are not recorded in
/// `state.scripts_run`. After a successful run, the timestamp is saved.
/// `run_` scripts execute unconditionally on every apply.
///
/// Scripts are run with the user's default shell (`$SHELL` or `/bin/sh`).
fn apply_scripts(scripts: &[crate::source::ScriptEntry], state: &mut State) -> Result<()> {
    if scripts.is_empty() {
        return Ok(());
    }

    println!("[scripts]");

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());

    for script in scripts {
        if script.when == ScriptExecWhen::Once {
            if state.scripts_run.contains_key(&script.name) {
                println!("  ~ {} (already run — skipped)", script.name);
                continue;
            }
        }

        print!("  running {}… ", script.name);
        let _ = std::io::Write::flush(&mut std::io::stdout());

        let status = std::process::Command::new(&shell)
            .arg(&script.src)
            .status()
            .with_context(|| format!("Cannot execute script {}", script.src.display()))?;

        if status.success() {
            println!("✓");
            if script.when == ScriptExecWhen::Once {
                let ts = chrono::Utc::now().to_rfc3339();
                state.scripts_run.insert(script.name.clone(), ts);
            }
        } else {
            let code = status.code().unwrap_or(-1);
            println!("✗ (exit code {})", code);
            anyhow::bail!(
                "Script {} failed with exit code {}",
                script.name, code
            );
        }
    }

    println!();
    Ok(())
}

// ─── AI skills ────────────────────────────────────────────────────────────────

/// Deploy AI skills from `ai/skills.toml` to the declared platform skill dirs.
///
/// Three-phase pipeline:
///   1. Validate each skill, check cache hits. Collect `gh:` cache-miss tasks.
///   2. Fetch all cache-miss skills in parallel (one thread per skill).
///      On failure: mark skill as failed, continue with others.
///   3. Deploy all non-failed skills and write AiState once (atomic).
fn apply_ai_skills(
    opts: &ApplyOptions<'_>,
    state: &mut State,
    lock: &mut crate::lock::LockFile,
) -> Result<()> {
    let platforms_config = PlatformsConfig::load(opts.repo_root)?;
    let skills_config = SkillsConfig::load(opts.repo_root)?;

    let (platforms_config, skills_config) = match (platforms_config, skills_config) {
        (Some(p), Some(s)) => (p, s),
        _ => return Ok(()), // no ai/ config — skip silently
    };

    if platforms_config.active.is_empty() {
        eprintln!("warning: ai/platforms.toml has no active platforms — skipping skill deployment");
        return Ok(());
    }
    if skills_config.skills.is_empty() {
        return Ok(());
    }

    let active_platforms = platforms_config.resolve_active_platforms()?;
    let skill_cache = SkillCache::new(opts.state_dir);

    // Collect existing deployed state so we can check ownership.
    let mut ai_state = state.ai.clone().unwrap_or_default();

    // Build the set of paths currently owned by dfiles (for collision check).
    let owned_targets: HashSet<PathBuf> = ai_state
        .deployed_skills
        .values()
        .flat_map(|m| m.values())
        .map(|e| e.target.clone())
        .collect();

    println!("\nDeploying AI skills…");

    let mut skills_failed = 0usize;

    // ── Phase 1: validate skills, check cache hits ────────────────────────────

    struct SkillPlan<'cfg> {
        skill: &'cfg SkillDeclaration,
        source_str: &'cfg str,
        target_platforms: Vec<&'cfg PlatformPlugin>,
        path: Option<PathBuf>,
        sha: Option<String>,
        failed: bool,
    }

    // Tasks for cache-miss gh: sources: (plan_idx, source, expected_sha).
    let mut fetch_tasks: Vec<(usize, GhSource, Option<String>)> = Vec::new();
    let mut plans: Vec<SkillPlan> = Vec::with_capacity(skills_config.skills.len());

    for skill in &skills_config.skills {
        let source_str = &skill.source;

        let skill_source = match SkillSource::parse(source_str) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  error: skill '{}' — invalid source: {:#}", skill.name, e);
                plans.push(SkillPlan {
                    skill, source_str, target_platforms: vec![],
                    path: None, sha: None, failed: true,
                });
                skills_failed += 1;
                continue;
            }
        };

        let target_platforms = skill.resolve_platforms(&active_platforms);
        if target_platforms.is_empty() {
            continue; // no active platforms for this skill — skip silently
        }

        match skill_source {
            SkillSource::Gh(gh) => {
                let lock_sha = lock.skill_sha(&gh.source_key()).map(str::to_string);
                let cached = skill_cache.cached_sha(&gh);

                // Cache hit: cached SHA exists and matches the lock.
                if let (Some(ref lsha), Some(ref csha)) = (&lock_sha, &cached) {
                    if lsha == csha {
                        plans.push(SkillPlan {
                            skill, source_str, target_platforms,
                            path: Some(skill_cache.cache_path(&gh)),
                            sha: Some(csha.clone()),
                            failed: false,
                        });
                        continue;
                    }
                }

                // Cache miss or stale — schedule a fetch.
                let plan_idx = plans.len();
                plans.push(SkillPlan {
                    skill, source_str, target_platforms,
                    path: None, sha: None, failed: false,
                });
                fetch_tasks.push((plan_idx, gh, lock_sha));
            }
            SkillSource::Dir(path) => {
                if !path.exists() {
                    eprintln!(
                        "  error: skill '{}' — dir: path not found: {}",
                        skill.name, path.display()
                    );
                    plans.push(SkillPlan {
                        skill, source_str, target_platforms,
                        path: None, sha: None, failed: true,
                    });
                    skills_failed += 1;
                } else {
                    plans.push(SkillPlan {
                        skill, source_str, target_platforms,
                        path: Some(path), sha: None, failed: false,
                    });
                }
            }
        }
    }

    // ── Phase 2: fetch cache-miss skills in parallel ──────────────────────────

    if !fetch_tasks.is_empty() {
        let n = fetch_tasks.len();
        if n == 1 {
            print!("  Fetching {}… ", plans[fetch_tasks[0].0].skill.name);
            let _ = std::io::Write::flush(&mut std::io::stdout());
        } else {
            println!("  Fetching {} skills in parallel…", n);
        }

        // Each thread owns its GhSource + expected_sha; borrows &skill_cache
        // (SkillCache: Sync) via a shared reference within the scope lifetime.
        let skill_cache_ref = &skill_cache;
        let results: Vec<(usize, GhSource, Result<String>)> =
            std::thread::scope(|s| {
                let handles: Vec<_> = fetch_tasks
                    .into_iter()
                    .map(|(plan_idx, gh, expected_sha)| {
                        s.spawn(move || {
                            let result = skill_cache_ref
                                .fetch_and_verify(&gh, expected_sha.as_deref());
                            (plan_idx, gh, result)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().expect("fetch thread panicked"))
                    .collect()
            });

        for (plan_idx, gh, result) in results {
            match result {
                Ok(sha) => {
                    if n == 1 {
                        println!("✓");
                    } else {
                        println!("  ✓ {}", plans[plan_idx].skill.name);
                    }
                    plans[plan_idx].path = Some(skill_cache.cache_path(&gh));
                    plans[plan_idx].sha = Some(sha.clone());
                    lock.pin_skill(&gh.source_key(), &sha);
                }
                Err(e) => {
                    if n == 1 {
                        println!("✗");
                    }
                    eprintln!("  error: skill '{}' — fetch failed: {:#}", plans[plan_idx].skill.name, e);
                    plans[plan_idx].failed = true;
                    skills_failed += 1;
                }
            }
        }
    }

    // ── Phase 3: deploy ───────────────────────────────────────────────────────

    let mut skills_applied = 0usize;

    for plan in &plans {
        if plan.failed {
            continue;
        }
        let skill_path = match &plan.path {
            Some(p) => p,
            None => continue, // unreachable for non-failed plans
        };

        // Deploy to each platform, deduplicating by resolved target path.
        let mut deployed_targets: HashSet<PathBuf> = HashSet::new();

        for platform in &plan.target_platforms {
            let target = platform.skills_dir.join(&plan.skill.name);

            // Deduplicate: github-copilot and cross-client both use ~/.agents/skills/.
            if deployed_targets.contains(&target) {
                continue;
            }
            deployed_targets.insert(target.clone());

            match crate::ai_skill::deploy_skill(
                skill_path,
                &target,
                &plan.skill.deploy,
                &owned_targets,
            ) {
                Ok(true) => {
                    let platform_map = ai_state
                        .deployed_skills
                        .entry(platform.id.clone())
                        .or_default();
                    platform_map.insert(
                        plan.skill.name.clone(),
                        AiDeployedEntry {
                            source: plan.source_str.to_string(),
                            deploy: plan.skill.deploy.as_str().to_string(),
                            target: target.clone(),
                            applied_at: Utc::now().to_rfc3339(),
                            sha: plan.sha.clone(),
                        },
                    );
                    println!("  ✓ {} → {}", plan.skill.name, target.display());
                    skills_applied += 1;
                }
                Ok(false) => {
                    // Warned + skipped inside deploy_skill.
                }
                Err(e) => {
                    eprintln!(
                        "  error: skill '{}' → {} — deploy failed: {:#}",
                        plan.skill.name, target.display(), e
                    );
                    skills_failed += 1;
                }
            }
        }
    }

    // Write AI state back (once, after all skills complete — atomic).
    state.ai = Some(ai_state);

    if skills_failed > 0 {
        eprintln!("  {} skill(s) failed — see errors above", skills_failed);
    }
    if skills_applied > 0 {
        println!("  {} skill(s) deployed", skills_applied);
    }

    Ok(())
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
