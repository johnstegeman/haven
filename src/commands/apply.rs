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
use crate::config::{sort_modules, HavenConfig, ModuleConfig};
use crate::vcs::{self, MigrateOutcome, VcsBackend};
use crate::config::module::expand_tilde;
use crate::fs::{apply_permissions, backup_file, copy_to_dest, sha256_of_bytes, sha256_of_str, write_to_dest};
use crate::ignore::IgnoreList;
use crate::ai_config::{AiConfig, BackendKind};
use crate::skill_backend::{DeploymentTarget, ResolvedSkill, SkillMetadata};
use crate::skill_backend_factory::create_backend;
use crate::skill_cache::SkillCache;
use crate::source::{scan, scan_scripts, ScriptExecWhen, SourceEntry};
use crate::state::{AiDeployedEntry, AppliedFileEntry, ModuleState, State};
use crate::template::TemplateContext;

/// How to resolve a conflict when the destination file was edited since last apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnConflict {
    /// Prompt interactively (default when stdin is a TTY).
    Prompt,
    /// Skip the file (keep user's version). Exit code 1.
    Skip,
    /// Overwrite silently with source content. Exit code 0.
    Overwrite,
}

impl std::str::FromStr for OnConflict {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "prompt"    => Ok(Self::Prompt),
            "skip"      => Ok(Self::Skip),
            "overwrite" => Ok(Self::Overwrite),
            other => anyhow::bail!("unknown --on-conflict value '{}' (expected: prompt | skip | overwrite)", other),
        }
    }
}

/// Outcome returned from `run()`, distinct from an error.
/// `had_conflict_skips = true` → caller should exit with code 1.
pub struct ApplyOutcome {
    pub had_conflict_skips: bool,
}

/// Mutable per-run state threaded through the file-application loop.
struct ApplyRunState<'a> {
    /// When true, overwrite all subsequent conflicts without prompting.
    overwrite_all: bool,
    /// When true, migrate all extdir entries to jj without prompting.
    jj_migrate_all: bool,
    state: &'a mut State,
    /// Set to true when any conflict is resolved by skipping.
    had_conflict_skips: bool,
}

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
    /// When removing unreferenced casks, also remove their associated data/files
    /// (`brew uninstall --cask --zap`). Implies `remove_unreferenced_brews`.
    pub zap: bool,
    /// VCS backend to use for new extdir clones. When set to Jj, also offers
    /// `jj git init --colocate` for existing extdirs that don't have a `.jj/`.
    pub vcs_backend: VcsBackend,
    /// How to resolve conflicts when destination was edited since last apply.
    pub on_conflict: OnConflict,
}

/// RAII guard that holds `~/.haven/apply.lock` for the duration of apply.
struct ApplyLock {
    path: PathBuf,
}

impl ApplyLock {
    fn acquire(state_dir: &Path) -> Result<Self> {
        use std::io::Write;
        let path = state_dir.join("apply.lock");
        std::fs::create_dir_all(state_dir).context("Cannot create state directory")?;
        loop {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    write!(f, "{}", std::process::id())
                        .with_context(|| format!("Cannot write lock file {}", path.display()))?;
                    return Ok(Self { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Lock file exists — check if the recorded PID is still alive.
                    if let Ok(contents) = std::fs::read_to_string(&path) {
                        if let Ok(pid) = contents.trim().parse::<u32>() {
                            if is_process_alive(pid) {
                                bail!(
                                    "haven apply is already running (PID {}). \
                                     If this is wrong, delete {}",
                                    pid,
                                    path.display()
                                );
                            }
                        }
                    }
                    // Stale lock (process dead or file unreadable) — remove and retry.
                    let _ = std::fs::remove_file(&path);
                }
                Err(e) => {
                    return Err(e)
                        .with_context(|| format!("Cannot create lock file {}", path.display()));
                }
            }
        }
    }
}

impl Drop for ApplyLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Returns true if a process with the given PID is currently running.
fn is_process_alive(pid: u32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    // kill(pid, None) sends signal 0: succeeds if the process exists and is
    // accessible, fails with ESRCH if the process is gone.
    kill(Pid::from_raw(pid as i32), None).is_ok()
}

pub fn run(opts: &ApplyOptions<'_>) -> Result<ApplyOutcome> {
    // Prevent two simultaneous `haven apply` runs from racing on state.json.
    let _lock = if !opts.dry_run {
        Some(ApplyLock::acquire(opts.state_dir)?)
    } else {
        None
    };

    let config = HavenConfig::load(opts.repo_root).unwrap_or_default();
    let template_ctx = TemplateContext::from_env(opts.profile, opts.repo_root, config.data);
    let source_dir = opts.repo_root.join("source");

    // ── 1. Scan and apply all source files ───────────────────────────────────
    let ignore = IgnoreList::load(opts.repo_root, &template_ctx);
    let entries = scan(&source_dir, &ignore)?;

    // Load state early so conflict detection has access to prior hashes.
    let mut state = if opts.dry_run {
        State::default()
    } else {
        State::load(opts.state_dir)?
    };

    if opts.dry_run {
        let mut sections = Vec::new();
        if opts.apply_files  { sections.push("files"); }
        if opts.apply_brews  { sections.push("brews"); }
        if opts.apply_ai     { sections.push("ai"); }
        println!("Dry run — no files will be written.\n");
        println!("Profile:  {}", opts.profile);
        let applying = if sections.is_empty() { "nothing".to_string() } else { sections.join(", ") };
        println!("Applying: {}", applying);
        if let Some(m) = opts.module_filter {
            println!("Module:   {} (brew/AI only)", m);
        }
        println!();
    }

    // ── 1. Source files ───────────────────────────────────────────────────────
    let mut files_applied = 0usize;
    let mut had_conflict_skips = false;
    if opts.apply_files {
        if opts.dry_run {
            println!("[files]");
        }
        let mut run_state = ApplyRunState {
            overwrite_all: false,
            jj_migrate_all: false,
            state: &mut state,
            had_conflict_skips: false,
        };
        for entry in &entries {
            if opts.dry_run {
                print_dry_run_entry(entry, opts.dest_root);
                continue;
            }
            if apply_entry(entry, opts, &template_ctx, &mut run_state)? {
                files_applied += 1;
            }
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
            had_conflict_skips = run_state.had_conflict_skips;
        }
    }

    // ── 2. Apply module brew / AI / mise / externals ─────────────────────────
    let modules_to_apply: Vec<String> = match opts.module_filter {
        Some(m) => vec![m.to_string()],
        None => HavenConfig::load(opts.repo_root)?.resolve_modules(opts.profile)
            .unwrap_or_default(),
    };
    let sorted = sort_modules(&modules_to_apply);

    // Brew: apply master Brewfile when no filter, module brewfile when filtered.
    let mut brewfiles_run = 0usize;
    if opts.apply_brews {
        brewfiles_run = apply_brew(opts, &sorted)?;

        // Optionally purge unreferenced packages after installing.
        if opts.remove_unreferenced_brews || opts.interactive {
            purge_unreferenced_brews(opts, &sorted)?;
        }
    }

    // ── Retain cleanup: remove stale applied_files entries ───────────────────
    // Any dest that is no longer tracked should not carry a lingering hash.
    if opts.apply_files && !opts.dry_run {
        let tracked: HashSet<&str> = entries.iter()
            .filter(|e| !e.flags.extdir && !e.flags.extfile && !e.flags.symlink && !e.flags.create_only)
            .map(|e| e.dest_tilde.as_str())
            .collect();
        state.applied_files.retain(|k, _| tracked.contains(k.as_str()));
    }

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
                eprintln!("warning: Could not write haven.lock: {}", e);
            }
        }
        // Inject skill snippets into non-claude-code platform config files.
        // Claude Code's CLAUDE.md is handled by claude_md::generate below,
        // which merges the skills listing and snippets into one haven section.
        let inj_skills = SkillsConfig::load(opts.repo_root)
            .ok()
            .flatten()
            .map(|c| c.skills)
            .unwrap_or_default();
        let inj_platforms: Vec<_> = PlatformsConfig::load(opts.repo_root)
            .ok()
            .flatten()
            .and_then(|c| c.resolve_active_platforms().ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.id != "claude-code")
            .collect();
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
        // Regenerate CLAUDE.md with updated skills, commands, and snippets.
        if let Err(e) = crate::claude_md::generate(opts.claude_dir, Some(opts.repo_root), opts.profile) {
            eprintln!("warning: CLAUDE.md generation failed: {}", e);
        }
        state.version = "1".into();
        state.last_apply = Some(Utc::now());
        state.profile = Some(opts.profile.to_string());
        state.hostname = crate::state::hostname();
        state.save(opts.state_dir)?;

        println!();
        let brew_suffix = if brewfiles_run > 0 {
            format!(", {} Brewfile(s)", brewfiles_run)
        } else {
            String::new()
        };
        println!(
            "Applied {} file(s){} across {} module(s) — profile: {}",
            files_applied,
            brew_suffix,
            sorted.len(),
            opts.profile
        );
    }

    Ok(ApplyOutcome { had_conflict_skips })
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

/// Returns `true` if the file was actually written or updated, `false` if skipped as up-to-date.
fn apply_entry(
    entry: &SourceEntry,
    opts: &ApplyOptions<'_>,
    template_ctx: &TemplateContext,
    run_state: &mut ApplyRunState<'_>,
) -> Result<bool> {
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
        apply_vcs_external(
            &content.url,
            content.ref_name.as_deref(),
            &dest,
            opts,
            &mut run_state.jj_migrate_all,
        )?;
        println!("  ✓ {}", dest.display());
        return Ok(true);
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
        return Ok(true);
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
        // Skip silently when the symlink already points to the right target,
        // matching the behaviour of regular files that are already up-to-date.
        let already_correct = dest.is_symlink()
            && std::fs::read_link(&dest).map(|t| t == link_target).unwrap_or(false);
        if already_correct {
            return Ok(false);
        }
        let backup = apply_symlink(&link_target, &dest, opts.backup_dir)
            .with_context(|| {
                format!("Cannot link {} → {}", dest.display(), link_target.display())
            })?;
        if let Some(b) = backup {
            println!("  backed up {} → {}", dest.display(), b.display());
        }
        println!("  ✓ {} ⟶ {}", dest.display(), link_target.display());
        return Ok(true);
    }

    // create_only: seed-only file — excluded from hash tracking (apply never
    // overwrites them, so a C marker in status would be misleading).
    if entry.flags.create_only && dest.exists() {
        println!("  ~ {} (create_only — already exists, not overwritten)", dest.display());
        return Ok(false);
    }

    // Render template or read source content.
    // Some(_) for templates, None for plain copies
    let new_content: Option<String> = if entry.flags.template {
        let source_text = std::fs::read_to_string(&entry.src)
            .with_context(|| format!("Cannot read template {}", entry.src.display()))?;
        let rendered = crate::template::render(&source_text, template_ctx)
            .with_context(|| format!("Cannot render template '{}'", entry.src.display()))?;
        Some(rendered)
    } else {
        None
    };

    // ── Conflict detection ────────────────────────────────────────────────────
    // Path A: no prior hash in state — use existing idempotency logic.
    // Path B: prior hash exists — check whether dest was edited since last apply.
    if dest.exists() && !dest.is_symlink() {
        let prior = run_state.state.applied_files.get(&entry.dest_tilde).cloned();
        match prior {
            None => {
                // Path A: no prior hash. Check idempotency with files_equal / string compare.
                let already_matches = match &new_content {
                    Some(rendered) => std::fs::read_to_string(&dest)
                        .map(|existing| existing == *rendered)
                        .unwrap_or(false),
                    None => files_equal(&entry.src, &dest),
                };
                if already_matches {
                    // Seed the hash so conflict detection is active from next apply.
                    let hash = match &new_content {
                        Some(rendered) => sha256_of_str(rendered),
                        None => {
                            let bytes = std::fs::read(&dest).unwrap_or_default();
                            sha256_of_bytes(&bytes)
                        }
                    };
                    run_state.state.applied_files.insert(
                        entry.dest_tilde.clone(),
                        AppliedFileEntry { sha256: hash },
                    );
                    // Re-apply permissions in case they drifted, then silently return.
                    if entry.flags.private || entry.flags.executable {
                        apply_permissions(&dest, entry.flags.private, entry.flags.executable)
                            .with_context(|| format!("Cannot set permissions on {}", dest.display()))?;
                    }
                    return Ok(false);
                }
                // Path A fall-through: dest differs from source — write it (no prompt).
            }
            Some(prior_entry) => {
                // Path B: prior hash exists. Read dest bytes to detect user edit.
                let dest_bytes = std::fs::read(&dest)
                    .with_context(|| format!("Cannot read {}", dest.display()))?;
                let dest_hash = sha256_of_bytes(&dest_bytes);

                if dest_hash != prior_entry.sha256 {
                    // User edited dest since last apply — resolve conflict.
                    let action = resolve_conflict(entry, opts, run_state, &dest, &dest_bytes, &new_content)?;
                    match action {
                        ConflictAction::Skip => {
                            run_state.had_conflict_skips = true;
                            println!("  ~ {} (skipped — keeping your version)", dest.display());
                            return Ok(false);
                        }
                        ConflictAction::Overwrite => {
                            // Fall through to the write path below.
                        }
                    }
                } else {
                    // dest unchanged since last apply. Check if source changed.
                    let source_matches = match &new_content {
                        Some(rendered) => std::str::from_utf8(&dest_bytes)
                            .map(|s| s == rendered.as_str())
                            .unwrap_or(false),
                        None => {
                            let src_bytes = std::fs::read(&entry.src)
                                .with_context(|| format!("Cannot read {}", entry.src.display()))?;
                            dest_bytes == src_bytes
                        }
                    };
                    if source_matches {
                        // Still identical — no write needed.
                        if entry.flags.private || entry.flags.executable {
                            apply_permissions(&dest, entry.flags.private, entry.flags.executable)
                                .with_context(|| format!("Cannot set permissions on {}", dest.display()))?;
                        }
                        return Ok(false);
                    }
                    // Source changed, dest unchanged — write silently (no prompt).
                }
            }
        }
    }

    // ── Write ─────────────────────────────────────────────────────────────────

    // Back up existing file before overwriting.
    if dest.exists() {
        let backup = backup_file(&dest, opts.backup_dir)
            .with_context(|| format!("Cannot back up {}", dest.display()))?;
        println!("  backed up {} → {}", dest.display(), backup.display());
    }

    let written_hash = match &new_content {
        Some(rendered) => {
            write_to_dest(rendered, &dest)
                .with_context(|| format!("Cannot write rendered file to {}", dest.display()))?;
            sha256_of_str(rendered)
        }
        None => {
            let src_bytes = std::fs::read(&entry.src)
                .with_context(|| format!("Cannot read {}", entry.src.display()))?;
            copy_to_dest(&entry.src, &dest)
                .with_context(|| format!("Cannot copy {} → {}", entry.src.display(), dest.display()))?;
            sha256_of_bytes(&src_bytes)
        }
    };

    // Record hash so conflict detection fires on the next apply.
    run_state.state.applied_files.insert(
        entry.dest_tilde.clone(),
        AppliedFileEntry { sha256: written_hash },
    );

    if entry.flags.private || entry.flags.executable {
        apply_permissions(&dest, entry.flags.private, entry.flags.executable)
            .with_context(|| format!("Cannot set permissions on {}", dest.display()))?;
    }

    println!("  ✓ {}", dest.display());
    Ok(true)
}

// ─── Conflict resolution ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConflictAction {
    Skip,
    Overwrite,
}

/// Resolve a conflict according to `opts.on_conflict`.
/// When `Prompt`, uses `ApplyRunState::overwrite_all` to short-circuit subsequent prompts.
fn resolve_conflict(
    entry: &SourceEntry,
    opts: &ApplyOptions<'_>,
    run_state: &mut ApplyRunState<'_>,
    dest: &Path,
    dest_bytes: &[u8],
    new_content: &Option<String>,
) -> Result<ConflictAction> {
    if run_state.overwrite_all {
        return Ok(ConflictAction::Overwrite);
    }

    match opts.on_conflict {
        OnConflict::Overwrite => return Ok(ConflictAction::Overwrite),
        OnConflict::Skip => return Ok(ConflictAction::Skip),
        OnConflict::Prompt => {}
    }

    // Non-TTY: warn and skip.
    // HAVEN_FORCE_INTERACTIVE=1 bypasses the TTY check (for integration tests).
    use std::io::IsTerminal;
    let force_interactive = std::env::var("HAVEN_FORCE_INTERACTIVE").as_deref() == Ok("1");
    if !force_interactive && !std::io::stdin().is_terminal() {
        eprintln!(
            "warning: {} was edited since last apply; --on-conflict=prompt requires a TTY — skipping",
            entry.dest_tilde
        );
        return Ok(ConflictAction::Skip);
    }

    // Interactive prompt loop.
    loop {
        print!(
            "  conflict: {} was edited since last apply.\n  [s]kip / [o]verwrite / [A]pply all / [d]iff: ",
            entry.dest_tilde
        );
        use std::io::Write;
        std::io::stdout().flush()?;

        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        match line.trim() {
            "s" | "S" => return Ok(ConflictAction::Skip),
            "o" | "O" => return Ok(ConflictAction::Overwrite),
            "A" => {
                run_state.overwrite_all = true;
                return Ok(ConflictAction::Overwrite);
            }
            "d" | "D" => {
                show_conflict_diff(entry, dest, dest_bytes, new_content);
                // Loop — show prompt again after diff.
            }
            _ => {
                eprintln!("  Please enter s, o, A, or d.");
            }
        }
    }
}

/// Print a unified diff of what haven would write vs. what is currently on disk.
fn show_conflict_diff(
    entry: &SourceEntry,
    dest: &Path,
    dest_bytes: &[u8],
    new_content: &Option<String>,
) {
    let dest_str = match std::str::from_utf8(dest_bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => {
            println!("  (binary file — diff not available)");
            return;
        }
    };

    let src_str: String = match new_content {
        Some(rendered) => rendered.clone(),
        None => match std::fs::read(&entry.src).ok().and_then(|b| String::from_utf8(b).ok()) {
            Some(s) => s,
            None => {
                println!("  (binary file — diff not available)");
                return;
            }
        },
    };

    let label_dest = dest.to_string_lossy();
    let label_src = &entry.dest_tilde;
    match crate::diff_util::unified_diff(&dest_str, &src_str, &label_dest, label_src, 3) {
        Some(diff) => {
            use std::io::IsTerminal;
            if std::io::stdout().is_terminal() {
                print!("{}", crate::diff_util::colorize_diff(&diff));
            } else {
                print!("{}", diff);
            }
        }
        None => println!("  (no diff — files are identical)"),
    }
}

/// Return true if both paths exist and have identical byte content.
fn files_equal(a: &Path, b: &Path) -> bool {
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
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
            .next_back()
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
        if script.when == ScriptExecWhen::Once
            && state.scripts_run.contains_key(&script.name)
        {
            println!("  ~ {} (already run — skipped)", script.name);
            continue;
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
///   3. Deploy all non-failed skills via backend.deploy_all() and write AiState
///      once (atomic).
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
    // Validate backend config and availability before starting any work.
    let ai_config = AiConfig::load(opts.repo_root)?;
    let backend = create_backend(&ai_config, opts.state_dir)?;
    let skill_cache = SkillCache::new(opts.state_dir);

    // Collect existing deployed state so we can check ownership.
    let mut ai_state = state.ai.clone().unwrap_or_default();

    // Build the set of paths currently owned by haven (for collision check).
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
                // AgentSkills backend: skip SkillCache entirely — agent-skills-cli
                // fetches and installs during deploy(). cached_path is ignored by deploy().
                if ai_config.backend == BackendKind::AgentSkills {
                    plans.push(SkillPlan {
                        skill, source_str, target_platforms,
                        path: Some(PathBuf::new()), // sentinel: deploy() ignores this
                        sha: None, failed: false,
                    });
                    continue;
                }

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
            SkillSource::Repo => {
                // repo: skills live at <repo_root>/ai/skills/<name>/files/.
                let files_path = opts
                    .repo_root
                    .join("ai")
                    .join("skills")
                    .join(&skill.name)
                    .join("files");
                if !files_path.exists() {
                    eprintln!(
                        "  error: skill '{}' — repo: files not found at {}",
                        skill.name,
                        files_path.display()
                    );
                    plans.push(SkillPlan {
                        skill, source_str, target_platforms,
                        path: None, sha: None, failed: true,
                    });
                    skills_failed += 1;
                } else {
                    plans.push(SkillPlan {
                        skill, source_str, target_platforms,
                        path: Some(files_path), sha: None, failed: false,
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
    //
    // Build a flat list of (skill, target) pairs — deduplicating by target path
    // so github-copilot and cross-client platforms sharing ~/.agents/skills/ only
    // result in a single deploy call — then hand the entire batch to deploy_all().
    //
    struct DeployEntry {
        skill_name: String,
        source_str: String,
        sha: Option<String>,
        deploy_method: String,
        platform_id: String,
        resolved: ResolvedSkill,
        target: DeploymentTarget,
    }

    let mut deploy_entries: Vec<DeployEntry> = Vec::new();
    let mut seen_targets: HashSet<PathBuf> = HashSet::new();

    for plan in &plans {
        if plan.failed {
            continue;
        }
        let skill_path = match &plan.path {
            Some(p) => p,
            None => continue, // unreachable for non-failed plans
        };
        for platform in &plan.target_platforms {
            let target_path = platform.skills_dir.join(&plan.skill.name);
            // Deduplicate: github-copilot and cross-client both use ~/.agents/skills/.
            if seen_targets.contains(&target_path) {
                continue;
            }
            seen_targets.insert(target_path);
            deploy_entries.push(DeployEntry {
                skill_name: plan.skill.name.clone(),
                source_str: plan.source_str.to_string(),
                sha: plan.sha.clone(),
                deploy_method: plan.skill.deploy.as_str().to_string(),
                platform_id: platform.id.clone(),
                resolved: {
                    // For the AgentSkills backend, inject _haven_source into metadata
                    // so that AgentSkillsBackend::deploy() can map the source to CLI args.
                    // This threads the original source declaration through the SkillBackend
                    // trait boundary without modifying it. See skill_backend_agentskills.rs.
                    let mut meta = SkillMetadata::default();
                    if ai_config.backend == BackendKind::AgentSkills {
                        meta.metadata.insert(
                            "_haven_source".to_string(),
                            plan.source_str.to_string(),
                        );
                    }
                    ResolvedSkill {
                        name: plan.skill.name.clone(),
                        cached_path: skill_path.clone(),
                        sha: plan.sha.clone().unwrap_or_default(),
                        metadata: meta,
                    }
                },
                target: DeploymentTarget {
                    platform_id: platform.id.clone(),
                    skills_dir: platform.skills_dir.clone(),
                    deploy_method: plan.skill.deploy.clone(),
                    owned_targets: owned_targets.clone(),
                },
            });
        }
    }

    let mut skills_applied = 0usize;

    if !deploy_entries.is_empty() {
        let pairs: Vec<(&ResolvedSkill, &DeploymentTarget)> =
            deploy_entries.iter().map(|e| (&e.resolved, &e.target)).collect();
        match backend.deploy_all(&pairs) {
            Ok(results) => {
                for (entry, result) in deploy_entries.iter().zip(results.iter()) {
                    if result.deployed {
                        let platform_map = ai_state
                            .deployed_skills
                            .entry(entry.platform_id.clone())
                            .or_default();
                        platform_map.insert(
                            entry.skill_name.clone(),
                            AiDeployedEntry {
                                source: entry.source_str.clone(),
                                deploy: entry.deploy_method.clone(),
                                target: result.target_path.clone(),
                                applied_at: Utc::now().to_rfc3339(),
                                sha: entry.sha.clone(),
                            },
                        );
                        println!("  ✓ {} → {}", entry.skill_name, result.target_path.display());
                        skills_applied += 1;
                    }
                    // was_collision=true: NativeBackend already printed the warning
                }
            }
            Err(e) => {
                eprintln!("  error: deploy failed: {:#}", e);
                skills_failed += deploy_entries.len();
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

/// Returns the number of Brewfiles that `brew bundle` was run against.
fn apply_brew(opts: &ApplyOptions<'_>, sorted_modules: &[String]) -> Result<usize> {
    // Collect brewfile paths: master + each module's brewfile.
    // When --module is set, only that module's brewfile is used.
    let brewfiles: Vec<PathBuf> = if let Some(module) = opts.module_filter {
        let config = ModuleConfig::load(opts.repo_root, module)?;
        config.homebrew
            .map(|hb| opts.repo_root.join(&hb.brewfile))
            .into_iter()
            .filter(|p| p.exists())
            .collect()
    } else {
        let mut paths = Vec::new();
        let master = opts.repo_root.join("brew").join("Brewfile");
        if master.exists() {
            paths.push(master);
        }
        for module_name in sorted_modules {
            if let Ok(config) = ModuleConfig::load(opts.repo_root, module_name) {
                if let Some(hb) = config.homebrew {
                    let bf = opts.repo_root.join(&hb.brewfile);
                    if bf.exists() {
                        paths.push(bf);
                    }
                }
            }
        }
        paths
    };

    if brewfiles.is_empty() {
        return Ok(0);
    }

    if opts.dry_run {
        for bf in &brewfiles {
            println!(
                "[brew] brew bundle --file {}",
                bf.strip_prefix(opts.repo_root).unwrap_or(bf).display()
            );
        }
        println!();
        return Ok(0);
    }

    let mut ran = 0usize;
    match crate::homebrew::ensure_brew(false)? {
        None => {
            println!("[brew] skipped (brew not available)");
        }
        Some(brew) => {
            for bf in &brewfiles {
                println!(
                    "Installing packages from {}…",
                    bf.strip_prefix(opts.repo_root).unwrap_or(bf).display()
                );
                crate::homebrew::bundle_install(&brew, bf)
                    .with_context(|| format!("brew bundle install failed for {}", bf.display()))?;
                println!("  ✓ brew bundle");
                ran += 1;
            }
        }
    }
    Ok(ran)
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
        // backup_file uses std::fs::copy which follows symlinks. Skip backup for
        // dangling symlinks (target gone — nothing useful to preserve).
        let b = if dest.exists() {
            Some(backup_file(dest, backup_dir)
                .with_context(|| format!("Cannot back up {}", dest.display()))?)
        } else {
            None // dangling symlink — target already missing, no backup needed
        };
        std::fs::remove_file(dest)
            .with_context(|| format!("Cannot remove {}", dest.display()))?;
        b
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

// ─── External dir helper ──────────────────────────────────────────────────────

/// Clone or update an external directory using the configured VCS backend.
///
/// - New dir: cloned via `vcs::clone_repo()` (git or jj, depth 1).
/// - Existing dir: `git pull --ff-only` (works in both git and colocated jj repos).
///   When backend is jj and `.jj/` is absent, offers `jj git init --colocate`.
fn apply_vcs_external(
    url: &str,
    ref_name: Option<&str>,
    dest: &Path,
    opts: &ApplyOptions<'_>,
    jj_migrate_all: &mut bool,
) -> Result<()> {
    if dest.exists() {
        // When backend is jj, offer migration for plain-git dirs.
        if opts.vcs_backend == VcsBackend::Jj
            && vcs::ensure_colocated(dest, *jj_migrate_all)? == MigrateOutcome::MigratedAll
        {
            *jj_migrate_all = true;
        }

        let git_dir = dest.join(".git");
        if !git_dir.exists() {
            anyhow::bail!(
                "{} already exists and is not a git repository",
                dest.display()
            );
        }
        if opts.apply_externals {
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
        vcs::clone_repo(opts.vcs_backend, url, dest, Some(1), ref_name)?;
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
        crate::homebrew::brew_uninstall(&brew, name, false, false)
            .with_context(|| format!("Failed to uninstall formula '{}'", name))?;
        println!("✓");
    }
    for name in &unreferenced_casks {
        print!("  Removing cask {}… ", name);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        crate::homebrew::brew_uninstall(&brew, name, true, opts.zap)
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
