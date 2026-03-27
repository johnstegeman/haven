/// `haven ai` subcommands: discover, add, add-local, fetch, update, remove.
///
/// These commands manage the lifecycle of AI skills declared in `ai/skills.toml`:
///
/// - `discover`   — scan for installed AI platforms, offer to update `active` list
/// - `add`        — append a new `[[skill]]` entry to `ai/skills.toml`
/// - `add-local`  — import a locally-developed skill into the haven repo (`repo:` source)
/// - `fetch`      — download `gh:` skills into the local cache (respects lock SHA)
/// - `update`     — like `fetch` but clears the lock SHA first to force re-download
/// - `remove`     — remove a skill from the config, lock file, and optionally live dirs
use anyhow::{Context, Result};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::ai_config::{AiConfig, BackendKind};
use crate::ai_platform::{platform_registry, PlatformsConfig};
use crate::ai_skill::{DeployMethod, SkillSource, SkillsConfig};
use crate::lock::LockFile;
use crate::skill_cache::SkillCache;
use crate::state::State;

// ─── discover ─────────────────────────────────────────────────────────────────

/// Options for `haven ai discover`.
pub struct DiscoverOptions<'a> {
    pub repo_root: &'a Path,
}

/// Scan for installed AI platforms and offer to update `ai/platforms.toml`.
pub fn discover(opts: &DiscoverOptions<'_>) -> Result<()> {
    let registry = platform_registry();

    // cross-client has no binary or meaningful config_dir to detect — skip it.
    let detectable: Vec<_> = registry.iter().filter(|p| p.id != "cross-client").collect();

    let mut detected: Vec<_> = Vec::new();
    let mut not_detected: Vec<_> = Vec::new();

    for platform in &detectable {
        if is_platform_installed(platform) {
            detected.push(*platform);
        } else {
            not_detected.push(*platform);
        }
    }

    // Print results table.
    println!("Platform detection results:");
    println!();
    for p in &detected {
        println!("  ✓  {} ({})", p.name, p.id);
    }
    for p in &not_detected {
        println!("  ✗  {} ({})", p.name, p.id);
    }
    println!();

    if detected.is_empty() {
        println!(
            "No AI platforms detected on this machine.\n\
             Install a platform binary and re-run `haven ai discover`."
        );
        return Ok(());
    }

    // Determine which detected platforms are already active.
    let current_active = load_active_list(opts.repo_root);
    let to_add: Vec<_> = detected
        .iter()
        .filter(|p| !current_active.contains(&p.id))
        .collect();

    if to_add.is_empty() {
        println!("All detected platforms are already in the active list. Nothing to do.");
        return Ok(());
    }

    let ids: Vec<&str> = to_add.iter().map(|p| p.id.as_str()).collect();
    println!("Detected but not yet active: {}", ids.join(", "));
    println!();
    print!("Add to ai/platforms.toml? [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if !input.trim().eq_ignore_ascii_case("y") {
        println!("Aborted — no changes made.");
        return Ok(());
    }

    let ids_to_add: Vec<String> = to_add.iter().map(|p| p.id.clone()).collect();
    update_platforms_active(opts.repo_root, &ids_to_add)?;
    println!("Updated ai/platforms.toml — active: {}", ids_to_add.join(", "));
    println!();
    println!("Run `haven apply --ai` to deploy skills to the newly-added platforms.");
    Ok(())
}

/// Check whether a platform is installed on this machine.
///
/// A platform is considered installed if its binary is found via `which`
/// or its config directory exists.
fn is_platform_installed(platform: &crate::ai_platform::PlatformPlugin) -> bool {
    if let Some(binary) = &platform.binary {
        if which_on_path(binary) {
            return true;
        }
    }
    if let Some(config_dir) = &platform.config_dir {
        if config_dir.exists() {
            return true;
        }
    }
    false
}

/// Return true if `name` can be found on PATH using `which`.
fn which_on_path(name: &str) -> bool {
    crate::util::is_on_path(name)
}

/// Load the `active` list from `ai/platforms.toml`, returning an empty vec if
/// the file is absent or has no active list.
fn load_active_list(repo_root: &Path) -> Vec<String> {
    PlatformsConfig::load(repo_root)
        .ok()
        .flatten()
        .map(|c| c.active)
        .unwrap_or_default()
}

/// Append `ids_to_add` to the `active` array in `ai/platforms.toml`.
/// Creates the file if it doesn't exist.
fn update_platforms_active(repo_root: &Path, ids_to_add: &[String]) -> Result<()> {
    let ai_dir = repo_root.join("ai");
    std::fs::create_dir_all(&ai_dir).context("Cannot create ai/ directory")?;
    let path = ai_dir.join("platforms.toml");

    let text = if path.exists() {
        std::fs::read_to_string(&path)
            .with_context(|| format!("Cannot read {}", path.display()))?
    } else {
        String::new()
    };

    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .context("ai/platforms.toml contains invalid TOML")?;

    if doc.contains_key("active") {
        // Append to the existing array.
        let arr = doc["active"]
            .as_value_mut()
            .and_then(|v| v.as_array_mut())
            .context("'active' in ai/platforms.toml is not an array")?;
        for id in ids_to_add {
            arr.push(id.as_str());
        }
    } else {
        // Create a fresh active array.
        let mut arr = toml_edit::Array::new();
        for id in ids_to_add {
            arr.push(id.as_str());
        }
        doc.insert("active", toml_edit::Item::Value(toml_edit::Value::Array(arr)));
    }

    std::fs::write(&path, doc.to_string())
        .with_context(|| format!("Cannot write {}", path.display()))
}

// ─── add ──────────────────────────────────────────────────────────────────────

/// Options for `haven ai add`.
pub struct AddOptions<'a> {
    pub repo_root: &'a Path,
    /// Source string: `gh:owner/repo[/subpath][@ref]` or `dir:~/path`.
    pub source: &'a str,
    /// Explicit skill name. If None, inferred from source.
    pub name: Option<&'a str>,
    /// Target platforms string. Defaults to `"all"`.
    pub platforms: &'a str,
    /// Deploy method.
    pub deploy: &'a str,
}

/// Add a new skill to `ai/skills/<name>/skill.toml` and create a blank snippet stub.
pub fn add(opts: &AddOptions<'_>) -> Result<()> {
    // Validate source.
    let parsed = SkillSource::parse(opts.source)
        .context("Invalid source")?;

    // Infer name if not explicitly given.
    let name = match opts.name {
        Some(n) => n.to_string(),
        None => infer_name_from_source(&parsed),
    };

    // Check for name conflicts in existing skills.
    if let Some(existing) = SkillsConfig::load(opts.repo_root)? {
        if existing.skills.iter().any(|s| s.name == name) {
            anyhow::bail!(
                "A skill named '{}' already exists in ai/skills/.\n\
                 Use --name to specify a different name.",
                name
            );
        }
    }

    // Validate deploy method.
    let _ = parse_deploy_method(opts.deploy)?;

    // Write ai/skills/<name>/skill.toml and create blank all.md stub.
    write_skill_dir(opts.repo_root, &name, opts.source, opts.platforms, opts.deploy)?;

    println!("Added skill '{}'.", name);
    println!("  → snippet: ai/skills/{}/all.md (edit to add agent instructions)", name);
    println!();
    println!("Run `haven apply --ai` to deploy it, or `haven ai fetch` to pre-warm the cache.");
    Ok(())
}

/// Infer a skill name from its source string.
///
/// - `gh:anthropics/skills/pdf-processing` → `"pdf-processing"` (last subpath component)
/// - `gh:anthropics/pdf-skill` → `"pdf-skill"` (repo name)
/// - `dir:~/projects/my-skill` → `"my-skill"` (last path component)
/// - `repo:` → not applicable (name comes from the source path in add-local)
fn infer_name_from_source(source: &SkillSource) -> String {
    match source {
        SkillSource::Gh(gh) => gh.name().to_string(),
        SkillSource::Dir(path) => path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "skill".to_string()),
        SkillSource::Repo => "skill".to_string(),
    }
}

// ─── add-local ────────────────────────────────────────────────────────────────

/// Options for `haven ai add-local`.
pub struct AddLocalOptions<'a> {
    pub repo_root: &'a Path,
    /// Path to the local skill directory to import.
    pub path: &'a str,
    /// Override for the skill name. Defaults to the directory name of `path`.
    pub name: Option<&'a str>,
    /// Target platforms. Default: `"all"`.
    pub platforms: &'a str,
}

/// Import a locally-developed skill into the haven repo.
///
/// Copies the skill directory into `ai/skills/<name>/files/`, writes
/// `ai/skills/<name>/skill.toml` with `source = "repo:"`, creates a blank
/// `all.md` snippet stub, and removes the original directory.
///
/// Run `haven apply --ai` afterward to deploy the skill symlink to
/// `~/.claude/skills/<name>` (or the equivalent for your active platforms).
pub fn add_local(opts: &AddLocalOptions<'_>) -> Result<()> {
    use crate::config::module::expand_tilde;

    // Expand tilde and resolve the source path.
    let src_path = expand_tilde(opts.path)
        .with_context(|| format!("Cannot expand path '{}'", opts.path))?;

    if !src_path.exists() {
        anyhow::bail!("Path does not exist: {}", src_path.display());
    }
    if !src_path.is_dir() {
        anyhow::bail!("Path is not a directory: {}", src_path.display());
    }

    // Infer skill name from directory name (or --name override).
    let name = match opts.name {
        Some(n) => n.to_string(),
        None => src_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .with_context(|| {
                format!("Cannot determine skill name from '{}'", src_path.display())
            })?,
    };

    // Check for name conflicts.
    if let Some(existing) = SkillsConfig::load(opts.repo_root)? {
        if existing.skills.iter().any(|s| s.name == name) {
            anyhow::bail!(
                "A skill named '{}' already exists in ai/skills/.\n\
                 Use --name to specify a different name.",
                name
            );
        }
    }

    let files_dir = opts
        .repo_root
        .join("ai")
        .join("skills")
        .join(&name)
        .join("files");

    // Refuse to overwrite an existing files/ dir to avoid silent data loss.
    if files_dir.exists() {
        anyhow::bail!(
            "ai/skills/{}/files/ already exists. Remove it manually before retrying.",
            name
        );
    }

    // Copy source directory into files/. Clean up on failure.
    std::fs::create_dir_all(&files_dir)
        .with_context(|| format!("Cannot create {}", files_dir.display()))?;

    if let Err(e) = crate::skill_cache::copy_dir_excluding_git(&src_path, &files_dir) {
        // Remove partial files/ dir before bailing.
        let _ = std::fs::remove_dir_all(&files_dir);
        return Err(e.context(format!(
            "Failed to copy '{}' into repo — original directory is unchanged.",
            src_path.display()
        )));
    }

    // Write skill.toml and create all.md stub.
    write_skill_dir(opts.repo_root, &name, "repo:", opts.platforms, "symlink")?;

    // Remove the original directory — import is complete.
    std::fs::remove_dir_all(&src_path).with_context(|| {
        format!(
            "Files copied but failed to remove original '{}'. \
             You may want to remove it manually to avoid confusion.",
            src_path.display()
        )
    })?;

    println!("Added local skill '{}'.", name);
    println!("  Files:   ai/skills/{}/files/", name);
    println!("  Snippet: ai/skills/{}/all.md  (edit to add agent instructions)", name);
    println!();
    println!("Run `haven apply --ai` to deploy the skill symlink.");

    Ok(())
}

/// Parse `"symlink"` or `"copy"` into `DeployMethod`.
fn parse_deploy_method(s: &str) -> Result<DeployMethod> {
    match s {
        "symlink" => Ok(DeployMethod::Symlink),
        "copy" => Ok(DeployMethod::Copy),
        other => anyhow::bail!(
            "Unknown deploy method '{}'. Use 'symlink' or 'copy'.",
            other
        ),
    }
}

/// Write `ai/skills/<name>/skill.toml` and create a blank `all.md` snippet stub.
///
/// The skill directory is created if it does not exist.
/// If `all.md` already exists, it is left unchanged (user may have edited it).
fn write_skill_dir(
    repo_root: &Path,
    name: &str,
    source: &str,
    platforms: &str,
    deploy: &str,
) -> Result<()> {
    let skill_dir = repo_root.join("ai").join("skills").join(name);
    std::fs::create_dir_all(&skill_dir)
        .with_context(|| format!("Cannot create {}", skill_dir.display()))?;

    // Build skill.toml content.
    let platforms_toml = if platforms == "all" || platforms == "cross-client" {
        format!("\"{}\"", platforms)
    } else {
        let ids: Vec<&str> = platforms.split(',').map(str::trim).collect();
        format!(
            "[{}]",
            ids.iter().map(|id| format!("\"{}\"", id)).collect::<Vec<_>>().join(", ")
        )
    };

    let mut toml = format!("source    = \"{}\"\nplatforms = {}\n", source, platforms_toml);
    if deploy != "symlink" {
        toml.push_str(&format!("deploy    = \"{}\"\n", deploy));
    }

    let skill_toml_path = skill_dir.join("skill.toml");
    std::fs::write(&skill_toml_path, &toml)
        .with_context(|| format!("Cannot write {}", skill_toml_path.display()))?;

    // Create blank all.md stub only if it doesn't already exist.
    let all_md_path = skill_dir.join("all.md");
    if !all_md_path.exists() {
        std::fs::write(&all_md_path, "")
            .with_context(|| format!("Cannot write {}", all_md_path.display()))?;
    }

    Ok(())
}

// ─── fetch ────────────────────────────────────────────────────────────────────

/// Options for `haven ai fetch`.
pub struct FetchOptions<'a> {
    pub repo_root: &'a Path,
    pub state_dir: &'a Path,
    /// If Some, only fetch this named skill. If None, fetch all.
    pub name: Option<&'a str>,
}

/// Download `gh:` skills into the local cache without deploying.
///
/// Respects the lock file — skips skills whose cache SHA already matches.
/// `dir:` skills are always skipped (local directories are read directly on apply).
pub fn fetch(opts: &FetchOptions<'_>) -> Result<()> {
    let skills = load_skills_required(opts.repo_root)?;
    let mut lock = LockFile::load(opts.repo_root)?;
    let cache = SkillCache::new(opts.state_dir);

    let to_fetch = filter_skills(&skills.skills, opts.name);
    let mut any = false;
    let mut errors = 0usize;

    for decl in to_fetch {
        match SkillSource::parse(&decl.source)? {
            SkillSource::Gh(gh) => {
                any = true;
                print!("Fetching '{}' from {} ... ", decl.name, decl.source);
                io::stdout().flush()?;
                match cache.ensure(&gh, &mut lock) {
                    Ok(sha) => println!("done ({})", &sha[..sha.len().min(12)]),
                    Err(e) => {
                        println!("FAILED");
                        eprintln!("  error: {:#}", e);
                        errors += 1;
                    }
                }
            }
            SkillSource::Dir(_) | SkillSource::Repo => {
                // dir: and repo: skills are read directly at apply time — no cache needed.
                if opts.name.is_some() {
                    println!(
                        "Skill '{}' uses a local source ({}) — nothing to fetch.",
                        decl.name, decl.source
                    );
                }
            }
        }
    }

    if !any && opts.name.is_none() {
        println!("No gh: skills found in ai/skills.toml. Nothing to fetch.");
    }

    lock.save(opts.repo_root)?;

    if errors > 0 {
        anyhow::bail!("{} skill(s) failed to fetch — see errors above.", errors);
    }
    Ok(())
}

// ─── update ───────────────────────────────────────────────────────────────────

/// Options for `haven ai update`.
pub struct UpdateOptions<'a> {
    pub repo_root: &'a Path,
    pub state_dir: &'a Path,
    /// If Some, only update this named skill. If None, update all.
    pub name: Option<&'a str>,
}

/// Fetch the latest version of skills, ignoring the current lock SHA.
///
/// Unlike `fetch`, this clears the lock entry before fetching so that
/// `SkillCache::ensure()` always downloads from source.
pub fn update(opts: &UpdateOptions<'_>) -> Result<()> {
    let skills = load_skills_required(opts.repo_root)?;

    let mut lock = LockFile::load(opts.repo_root)?;
    let cache = SkillCache::new(opts.state_dir);

    let to_update = filter_skills(&skills.skills, opts.name);
    let mut any = false;
    let mut errors = 0usize;

    for decl in to_update {
        match SkillSource::parse(&decl.source)? {
            SkillSource::Gh(gh) => {
                any = true;
                // Clear the lock SHA so ensure() treats this as a cache miss.
                lock.remove_skill(&gh.source_key());
                print!("Updating '{}' from {} ... ", decl.name, decl.source);
                io::stdout().flush()?;
                match cache.ensure(&gh, &mut lock) {
                    Ok(sha) => println!("done ({})", &sha[..sha.len().min(12)]),
                    Err(e) => {
                        println!("FAILED");
                        eprintln!("  error: {:#}", e);
                        errors += 1;
                    }
                }
            }
            SkillSource::Dir(_) | SkillSource::Repo => {
                // dir: and repo: skills are read directly at apply time — nothing to update.
                if opts.name.is_some() {
                    println!(
                        "Skill '{}' uses a local source ({}) — nothing to update.",
                        decl.name, decl.source
                    );
                }
            }
        }
    }

    if !any && opts.name.is_none() {
        println!("No gh: skills found in ai/skills.toml. Nothing to update.");
    }

    lock.save(opts.repo_root)?;

    if errors > 0 {
        anyhow::bail!("{} skill(s) failed to update — see errors above.", errors);
    }
    Ok(())
}

// ─── remove ───────────────────────────────────────────────────────────────────

/// Options for `haven ai remove`.
pub struct RemoveOptions<'a> {
    pub repo_root: &'a Path,
    pub state_dir: &'a Path,
    /// Name of the skill to remove.
    pub name: &'a str,
    /// Skip confirmation prompts.
    pub yes: bool,
}

/// Remove a skill from `ai/skills.toml`, the lock file, and optionally
/// deployed skill directories.
///
/// `opts.name` is matched first by exact skill name, then by source string
/// (useful when passing the full `gh:owner/repo/...` URL). If neither matches,
/// the error lists the available skill names.
pub fn remove(opts: &RemoveOptions<'_>) -> Result<()> {
    let skills = load_skills_required(opts.repo_root)?;

    // Match by name first, then fall back to source URL.
    let decl = skills
        .skills
        .iter()
        .find(|s| s.name == opts.name)
        .or_else(|| {
            // Normalise: strip a leading "gh:" prefix the user may have omitted,
            // then try a suffix match so both "jujutsu" and
            // "gh:owner/repo/skills/jujutsu" resolve to the same skill.
            let needle = opts.name.trim_start_matches("gh:");
            skills.skills.iter().find(|s| {
                let src = s.source.trim_start_matches("gh:");
                src == needle || src.ends_with(&format!("/{}", needle))
            })
        })
        .with_context(|| {
            let names: Vec<&str> = skills.skills.iter().map(|s| s.name.as_str()).collect();
            if names.is_empty() {
                format!("No skill named '{}' found — no skills are declared yet.", opts.name)
            } else {
                format!(
                    "No skill named '{}' found in ai/skills/.\nAvailable skills: {}",
                    opts.name,
                    names.join(", ")
                )
            }
        })?;

    let source_str = decl.source.clone();

    // Confirm removal from config.
    if !opts.yes {
        print!(
            "Remove skill '{}' ({})? [y/N] ",
            opts.name, source_str
        );
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Remove the skill directory.
    remove_skill_entry(opts.repo_root, opts.name)?;
    println!("Removed skill directory ai/skills/{}.", opts.name);

    // Remove from lock file (only for gh: sources).
    if let Ok(SkillSource::Gh(gh)) = SkillSource::parse(&source_str) {
        let mut lock = LockFile::load(opts.repo_root)?;
        lock.remove_skill(&gh.source_key());
        lock.save(opts.repo_root)?;
    }

    // Find and optionally remove deployed copies.
    let deployed = find_deployed_paths(opts.state_dir, opts.name);
    if deployed.is_empty() {
        return Ok(());
    }

    println!();
    println!("Deployed copies found:");
    for (platform_id, target) in &deployed {
        println!("  {} → {}", platform_id, target.display());
    }
    println!();

    let remove_deployed = if opts.yes {
        true
    } else {
        print!("Remove deployed copies? [y/N] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        input.trim().eq_ignore_ascii_case("y")
    };

    if remove_deployed {
        for (platform_id, target) in &deployed {
            if target.is_symlink() || target.exists() {
                if target.is_dir() && !target.is_symlink() {
                    std::fs::remove_dir_all(target).with_context(|| {
                        format!("Cannot remove {}", target.display())
                    })?;
                } else {
                    std::fs::remove_file(target).with_context(|| {
                        format!("Cannot remove {}", target.display())
                    })?;
                }
                println!("Removed {} ({})", target.display(), platform_id);
            }
        }
    }

    Ok(())
}

/// Remove the `ai/skills/<name>/` directory.
fn remove_skill_entry(repo_root: &Path, name: &str) -> Result<()> {
    let skill_dir = repo_root.join("ai").join("skills").join(name);
    if skill_dir.exists() {
        std::fs::remove_dir_all(&skill_dir)
            .with_context(|| format!("Cannot remove {}", skill_dir.display()))?;
    }
    Ok(())
}

/// Return all `(platform_id, target_path)` pairs from state.json for a given
/// skill name.
fn find_deployed_paths(state_dir: &Path, skill_name: &str) -> Vec<(String, PathBuf)> {
    let state = match State::load(state_dir) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let ai_state = match &state.ai {
        Some(a) => a,
        None => return Vec::new(),
    };
    ai_state
        .deployed_skills
        .iter()
        .filter_map(|(platform_id, platform_skills)| {
            platform_skills
                .get(skill_name)
                .map(|entry| (platform_id.clone(), entry.target.clone()))
        })
        .collect()
}

// ─── search ───────────────────────────────────────────────────────────────────

/// Options for `haven ai search`.
pub struct SearchOptions<'a> {
    pub repo_root: &'a Path,
    pub query: &'a str,
    pub limit: u8,
}

/// Search for skills using the configured backend and display matching results.
pub fn search(opts: &SearchOptions<'_>) -> Result<()> {
    let ai_config = AiConfig::load(opts.repo_root)?;
    let registry_label = match ai_config.backend {
        BackendKind::AgentSkills => "agent-skills marketplace",
        _ => "skills.sh",
    };
    print!("Searching {} for '{}' ...", registry_label, opts.query);
    io::stdout().flush()?;

    let results = dispatch_search(&ai_config, opts.query, opts.limit as usize)?;
    println!();

    if results.is_empty() {
        println!("No results found for '{}'.", opts.query);
        return Ok(());
    }

    println!("{} result(s):\n", results.len());
    for entry in &results {
        if let Some(installs) = entry.installs {
            println!("  {}  ({} installs)", entry.source, installs);
        } else {
            println!("  {}", entry.source);
        }
    }
    println!();
    println!("To add a skill:  haven ai add <gh:source>");
    Ok(())
}

// ─── scan ─────────────────────────────────────────────────────────────────────

/// Options for `haven ai scan`.
pub struct ScanOptions<'a> {
    pub repo_root: &'a Path,
    pub state_dir: &'a Path,
    /// Directory to scan for skill subdirectories.
    pub dir: &'a str,
    pub dry_run: bool,
}

/// Scan a skills directory, detect gh: sources, and interactively add unmanaged
/// skills to `ai/skills.toml`.
pub fn scan(opts: &ScanOptions<'_>) -> Result<()> {
    let scan_dir = expand_dir(opts.dir)?;
    let skill_cache_dir = opts.state_dir.join("skills");
    let ai_config = AiConfig::load(opts.repo_root)?;

    // Load existing skills to skip already-managed ones.
    let existing_names: std::collections::HashSet<String> = SkillsConfig::load(opts.repo_root)?
        .map(|c| c.skills.into_iter().map(|s| s.name).collect())
        .unwrap_or_default();

    // Collect candidate skill entries: (display_name, real_path).
    let mut candidates: Vec<(String, PathBuf)> = Vec::new();

    let read_dir = std::fs::read_dir(&scan_dir)
        .with_context(|| format!("Cannot read directory {}", scan_dir.display()))?;

    for entry in read_dir {
        let entry = entry.context("Error reading directory entry")?;
        let name = entry.file_name().to_string_lossy().into_owned();

        // Skip hidden entries.
        if name.starts_with('.') {
            continue;
        }

        // Resolve symlinks to their real path.
        let real_path = std::fs::canonicalize(entry.path())
            .unwrap_or_else(|_| entry.path().to_path_buf());

        // Skip if not a directory.
        if !real_path.is_dir() {
            continue;
        }

        // Skip if already managed by haven (points into the skill cache).
        if real_path.starts_with(&skill_cache_dir) {
            continue;
        }

        // Skip if no SKILL.md — not a skill directory.
        if !real_path.join("SKILL.md").exists() {
            // Also check one level deeper (monorepo: scan_dir/gstack/browse/SKILL.md).
            // We handle monorepos by recursing one level if the dir has no SKILL.md
            // but contains subdirs that do.
            let has_subskills = std::fs::read_dir(&real_path)
                .ok()
                .map(|entries| {
                    entries.flatten().any(|e| {
                        let sub_real = std::fs::canonicalize(e.path())
                            .unwrap_or_else(|_| e.path().to_path_buf());
                        sub_real.is_dir() && sub_real.join("SKILL.md").exists()
                    })
                })
                .unwrap_or(false);

            if has_subskills {
                // Recurse one level: add each subdir as a candidate.
                if let Ok(sub_entries) = std::fs::read_dir(&real_path) {
                    for sub in sub_entries.flatten() {
                        let sub_name = sub.file_name().to_string_lossy().into_owned();
                        if sub_name.starts_with('.') { continue; }
                        let sub_real = std::fs::canonicalize(sub.path())
                            .unwrap_or_else(|_| sub.path().to_path_buf());
                        if sub_real.is_dir() && sub_real.join("SKILL.md").exists() {
                            candidates.push((sub_name, sub_real));
                        }
                    }
                }
            }
            continue;
        }

        candidates.push((name, real_path));
    }

    // Remove candidates already tracked in skills.toml.
    let candidates: Vec<_> = candidates
        .into_iter()
        .filter(|(name, _)| !existing_names.contains(name))
        .collect();

    if candidates.is_empty() {
        println!("No unmanaged skills found in {}.", scan_dir.display());
        return Ok(());
    }

    println!("Found {} unmanaged skill(s) in {}.\n", candidates.len(), scan_dir.display());

    let total = candidates.len();
    let mut added = 0usize;

    for (idx, (name, real_path)) in candidates.iter().enumerate() {
        println!("[{}/{}] {}", idx + 1, total, name);

        // Try git detection first.
        let detected = detect_gh_source(real_path);

        // For skills without a detected git source, search the configured backend once
        // (limit=10) and reuse the results for both the best-match display and the ?-prompt.
        let mut scan_search_results: Vec<SearchEntry> = Vec::new();

        let proposed = match &detected {
            Some(src) => {
                let detail = git_remote_url(real_path)
                    .unwrap_or_else(|| "git remote".to_string());
                println!("  Detected: {}  ({})", src, detail);
                src.clone()
            }
            None => {
                print!("  No git remote — searching skills.sh for '{}' ...", name);
                io::stdout().flush()?;
                scan_search_results = dispatch_search(&ai_config, name, 10).unwrap_or_default();
                println!();

                if scan_search_results.is_empty() {
                    println!("  No matches found on skills.sh.");
                    println!("  Skip with 'n' or enter a source manually with 'e'.");
                    String::new()
                } else {
                    let best = &scan_search_results[0];
                    let dup_note = if scan_search_results.len() > 1 {
                        format!("  ({} results — use '?' to see all)", scan_search_results.len())
                    } else {
                        String::new()
                    };
                    let installs_note = best.installs.map(|n| format!(" ({} installs)", n)).unwrap_or_default();
                    println!("  Best match: {}{}{}",best.source, installs_note, dup_note);
                    best.source.clone()
                }
            }
        };

        // Interactive prompt loop.
        let all_results: Option<Vec<SearchEntry>> = if detected.is_none() {
            Some(scan_search_results)
        } else {
            None
        };

        loop {
            let hint = if proposed.is_empty() {
                "[y=skip/n=skip/e=enter manually/?=search results]".to_string()
            } else {
                format!("Add {}?  [y/n/e=edit/?=more results]", proposed)
            };
            print!("  {} ", hint);
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let input = input.trim();

            match input {
                "y" | "Y" | "" if !proposed.is_empty() => {
                    if opts.dry_run {
                        println!("  (dry-run) Would add: {}", proposed);
                    } else {
                        write_skill_dir(opts.repo_root, name, &proposed, "all", "symlink")?;
                        println!("  Added {}.", proposed);
                        println!("  → snippet: ai/skills/{}/all.md (edit to add agent instructions)", name);
                    }
                    added += 1;
                    break;
                }
                "n" | "N" => {
                    println!("  Skipped.");
                    break;
                }
                "e" | "E" => {
                    print!("  Enter gh: source: ");
                    io::stdout().flush()?;
                    let mut src_input = String::new();
                    io::stdin().read_line(&mut src_input)?;
                    let src = src_input.trim().to_string();
                    if src.is_empty() {
                        println!("  Skipped.");
                        break;
                    }
                    // Validate the source string.
                    match SkillSource::parse(&src) {
                        Err(e) => {
                            println!("  Invalid source: {}. Try again.", e);
                            continue;
                        }
                        Ok(_) => {
                            if opts.dry_run {
                                println!("  (dry-run) Would add: {}", src);
                            } else {
                                write_skill_dir(opts.repo_root, name, &src, "all", "symlink")?;
                                println!("  Added {}.", src);
                                println!("  → snippet: ai/skills/{}/all.md (edit to add agent instructions)", name);
                            }
                            added += 1;
                            break;
                        }
                    }
                }
                "?" => {
                    let results = all_results.as_deref().unwrap_or(&[]);
                    if results.is_empty() {
                        println!("  No search results available.");
                    } else {
                        for (i, r) in results.iter().enumerate() {
                            let installs_note = r.installs.map(|n| format!("  ({} installs)", n)).unwrap_or_default();
                            println!("    {}. {}{}", i + 1, r.source, installs_note);
                        }
                        print!("  Pick [1-{}] or (n)ext/(e)nter manually: ", results.len());
                        io::stdout().flush()?;
                        let mut pick = String::new();
                        io::stdin().read_line(&mut pick)?;
                        let pick = pick.trim();
                        if let Ok(n) = pick.parse::<usize>() {
                            if n >= 1 && n <= results.len() {
                                let chosen = results[n - 1].source.clone();
                                if opts.dry_run {
                                    println!("  (dry-run) Would add: {}", chosen);
                                } else {
                                    write_skill_dir(opts.repo_root, name, &chosen, "all", "symlink")?;
                                    println!("  Added {}.", chosen);
                                    println!("  → snippet: ai/skills/{}/all.md (edit to add agent instructions)", name);
                                }
                                added += 1;
                                break;
                            }
                        }
                        if pick == "n" || pick == "N" {
                            println!("  Skipped.");
                            break;
                        }
                        // Otherwise loop back to the prompt.
                        continue;
                    }
                }
                _ => {
                    println!("  Unknown input. Use y/n/e/?");
                    continue;
                }
            }
        }
        println!();
    }

    if opts.dry_run {
        println!("Dry run — {} skill(s) would be added.", added);
    } else if added > 0 {
        println!("{} skill(s) added.", added);
        println!("Run `haven apply --ai` to deploy.");
    } else {
        println!("No skills added.");
    }

    Ok(())
}

// ─── search dispatch ──────────────────────────────────────────────────────────

/// Unified search result returned by all backends.
pub struct SearchEntry {
    /// Full `gh:owner/repo/skill` source string.
    pub source: String,
    /// Install count from the registry, if available.
    pub installs: Option<u64>,
}

/// Route a search query to the appropriate backend based on `AiConfig`.
fn dispatch_search(config: &AiConfig, query: &str, limit: usize) -> Result<Vec<SearchEntry>> {
    match config.backend {
        BackendKind::AgentSkills => agentskills_search(&config.runner, query, limit),
        _ => skillssh_search(query, limit),
    }
}

// ─── skills.sh backend ────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct SkillsShResponse {
    skills: Vec<SkillsShEntry>,
}

#[derive(serde::Deserialize)]
struct SkillsShEntry {
    /// Full path: "owner/repo/skillName" — maps to gh:owner/repo/skillName.
    id: String,
    installs: u64,
}

fn skillssh_search(query: &str, limit: usize) -> Result<Vec<SearchEntry>> {
    let response = ureq::get("https://skills.sh/api/search")
        .query("q", query)
        .query("limit", &limit.to_string())
        .set("User-Agent", "haven/0.1 (+https://github.com/johnstegeman/haven)")
        .call()
        .context("skills.sh request failed")?
        .into_string()
        .context("skills.sh response was not UTF-8")?;

    let parsed: SkillsShResponse = serde_json::from_str(&response)
        .context("skills.sh response could not be parsed")?;

    Ok(parsed.skills.into_iter().map(|e| SearchEntry {
        source: format!("gh:{}", e.id),
        installs: Some(e.installs),
    }).collect())
}

// ─── agent-skills backend ─────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct AgentSkillsSearchResponse {
    skills: Vec<AgentSkillsSearchEntry>,
}

#[derive(serde::Deserialize)]
struct AgentSkillsSearchEntry {
    /// Full path: "owner/repo/skillName" — maps to gh:owner/repo/skillName.
    path: String,
    #[serde(default)]
    stars: Option<u64>,
}

fn agentskills_search(runner: &[String], query: &str, limit: usize) -> Result<Vec<SearchEntry>> {
    let runner_display = runner.join(" ");
    let output = std::process::Command::new(&runner[0])
        .args(&runner[1..])
        .args(["search", query, "--json", "--limit", &limit.to_string()])
        .output()
        .with_context(|| format!("Failed to run '{}' — is agent-skills-cli installed?", runner_display))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("'{}' search failed: {}", runner_display, stderr.trim());
    }

    let text = String::from_utf8(output.stdout).context("agent-skills-cli output was not UTF-8")?;
    let parsed: AgentSkillsSearchResponse =
        serde_json::from_str(&text).context("agent-skills-cli search output could not be parsed")?;

    Ok(parsed.skills.into_iter().map(|e| SearchEntry {
        source: format!("gh:{}", e.path),
        installs: e.stars,
    }).collect())
}

// ─── git source detection ─────────────────────────────────────────────────────

/// Try to derive a `gh:owner/repo[/subpath]` source from a skill directory
/// by reading its git remote and computing the subpath from the repo root.
fn detect_gh_source(skill_dir: &Path) -> Option<String> {
    // Find the git repo root containing this directory.
    let root_out = std::process::Command::new("git")
        .args(["-C", &skill_dir.to_string_lossy(), "rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !root_out.status.success() {
        return None;
    }
    let git_root = PathBuf::from(
        String::from_utf8(root_out.stdout).ok()?.trim().to_string()
    );

    // Get the origin remote URL.
    let remote_out = std::process::Command::new("git")
        .args(["-C", &skill_dir.to_string_lossy(), "remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !remote_out.status.success() {
        return None;
    }
    let remote_url = String::from_utf8(remote_out.stdout).ok()?;
    let (owner, repo) = parse_github_url(remote_url.trim())?;

    // Compute the subpath: skill_dir relative to git root.
    let subpath = skill_dir.strip_prefix(&git_root).ok()?;
    let subpath_str = subpath.to_string_lossy();

    if subpath_str.is_empty() || subpath_str == "." {
        Some(format!("gh:{}/{}", owner, repo))
    } else {
        Some(format!("gh:{}/{}/{}", owner, repo, subpath_str))
    }
}

/// Return just the remote URL string (for display), without parsing it.
fn git_remote_url(skill_dir: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["-C", &skill_dir.to_string_lossy(), "remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}

/// Parse `https://github.com/owner/repo[.git]` or `git@github.com:owner/repo[.git]`
/// into `(owner, repo)`. Returns `None` for non-GitHub remotes.
fn parse_github_url(url: &str) -> Option<(String, String)> {
    let url = url.trim_end_matches(".git");
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        let mut parts = rest.splitn(2, '/');
        return Some((parts.next()?.to_string(), parts.next()?.to_string()));
    }
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let mut parts = rest.splitn(2, '/');
        return Some((parts.next()?.to_string(), parts.next()?.to_string()));
    }
    None
}

/// Expand `~` in a directory path and return the canonical `PathBuf`.
fn expand_dir(dir: &str) -> Result<PathBuf> {
    let expanded = if let Some(rest) = dir.strip_prefix("~/") {
        dirs::home_dir()
            .context("Cannot determine home directory")?
            .join(rest)
    } else if dir == "~" {
        dirs::home_dir().context("Cannot determine home directory")?
    } else {
        PathBuf::from(dir)
    };
    Ok(expanded)
}

// ─── Shared helpers ───────────────────────────────────────────────────────────

/// Load skills and return an error if no skills are declared.
fn load_skills_required(repo_root: &Path) -> Result<SkillsConfig> {
    SkillsConfig::load(repo_root)?
        .context("No skills found in ai/skills/. Use `haven ai add` to declare skills first.")
}

/// Filter skill declarations to just the named skill (if specified) or all.
fn filter_skills<'a>(
    skills: &'a [crate::ai_skill::SkillDeclaration],
    name: Option<&str>,
) -> Vec<&'a crate::ai_skill::SkillDeclaration> {
    match name {
        None => skills.iter().collect(),
        Some(n) => skills.iter().filter(|s| s.name == n).collect(),
    }
}

// ─── backends ─────────────────────────────────────────────────────────────────

pub struct BackendsOptions<'a> {
    pub repo_root: &'a std::path::Path,
}

/// List all known skill backends and their availability on this machine.
pub fn backends(opts: &BackendsOptions<'_>) -> Result<()> {
    let config = crate::ai_config::AiConfig::load(opts.repo_root)?;
    let active = config.backend.as_str();
    let infos = crate::skill_backend_factory::list_backends();

    println!("Skill backends:");
    println!();
    for info in &infos {
        let marker = if info.name == active { " *" } else { "  " };
        let status = if info.available { "✓" } else { "✗" };
        println!("{} {}  {} — {}", marker, status, info.name, info.note);
    }
    println!();
    println!("  * = active backend (from ai/config.toml, or default)");
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper to create a skill directory with skill.toml (new per-dir format).
    fn write_skill_dir_test(dir: &TempDir, name: &str, source: &str) {
        let skill_dir = dir.path().join("ai").join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            format!("source    = \"{}\"\nplatforms = \"all\"\n", source),
        )
        .unwrap();
    }

    // ── add ──────────────────────────────────────────────────────────────────

    #[test]
    fn add_creates_skill_dir_when_absent() {
        let dir = TempDir::new().unwrap();
        add(&AddOptions {
            repo_root: dir.path(),
            source: "gh:anthropics/skills/pdf-processing",
            name: None,
            platforms: "all",
            deploy: "symlink",
        })
        .unwrap();

        let skill_toml = dir.path().join("ai/skills/pdf-processing/skill.toml");
        let text = std::fs::read_to_string(&skill_toml).unwrap();
        assert!(text.contains("gh:anthropics/skills/pdf-processing"));
        assert!(text.contains(r#"platforms = "all""#));
        // Default deploy method (symlink) is omitted.
        assert!(!text.contains("deploy"));
        // Blank all.md stub should also be created.
        assert!(dir.path().join("ai/skills/pdf-processing/all.md").exists());
    }

    #[test]
    fn add_second_skill_creates_separate_dir() {
        let dir = TempDir::new().unwrap();
        write_skill_dir_test(&dir, "existing", "gh:owner/existing");

        add(&AddOptions {
            repo_root: dir.path(),
            source: "gh:owner/new-skill",
            name: Some("new-skill"),
            platforms: "claude-code,codex",
            deploy: "copy",
        })
        .unwrap();

        // Both skill dirs should exist.
        assert!(dir.path().join("ai/skills/existing/skill.toml").exists());
        let text = std::fs::read_to_string(
            dir.path().join("ai/skills/new-skill/skill.toml"),
        )
        .unwrap();
        assert!(text.contains("gh:owner/new-skill"));
        assert!(text.contains("deploy"));
    }

    #[test]
    fn add_rejects_duplicate_name() {
        let dir = TempDir::new().unwrap();
        write_skill_dir_test(&dir, "pdf-processing", "gh:owner/pdf");

        let err = add(&AddOptions {
            repo_root: dir.path(),
            source: "gh:another/pdf-processing",
            name: None,
            platforms: "all",
            deploy: "symlink",
        })
        .unwrap_err();
        assert!(format!("{:#}", err).contains("pdf-processing"));
    }

    #[test]
    fn add_infers_name_from_gh_subpath() {
        let dir = TempDir::new().unwrap();
        add(&AddOptions {
            repo_root: dir.path(),
            source: "gh:anthropics/skills/pdf-processing",
            name: None,
            platforms: "all",
            deploy: "symlink",
        })
        .unwrap();
        assert!(dir.path().join("ai/skills/pdf-processing/skill.toml").exists());
    }

    #[test]
    fn add_infers_name_from_gh_repo() {
        let dir = TempDir::new().unwrap();
        add(&AddOptions {
            repo_root: dir.path(),
            source: "gh:owner/my-skill-repo",
            name: None,
            platforms: "all",
            deploy: "symlink",
        })
        .unwrap();
        assert!(dir.path().join("ai/skills/my-skill-repo/skill.toml").exists());
    }

    #[test]
    fn add_platforms_list_stored_in_skill_toml() {
        let dir = TempDir::new().unwrap();
        add(&AddOptions {
            repo_root: dir.path(),
            source: "gh:owner/skill",
            name: Some("my-skill"),
            platforms: "claude-code,codex",
            deploy: "symlink",
        })
        .unwrap();
        let text = std::fs::read_to_string(
            dir.path().join("ai/skills/my-skill/skill.toml"),
        )
        .unwrap();
        assert!(text.contains("claude-code"));
        assert!(text.contains("codex"));
    }

    // ── remove_skill_entry ───────────────────────────────────────────────────

    #[test]
    fn remove_entry_deletes_named_skill_dir() {
        let dir = TempDir::new().unwrap();
        write_skill_dir_test(&dir, "keep-me", "gh:owner/keep");
        write_skill_dir_test(&dir, "delete-me", "gh:owner/delete");

        remove_skill_entry(dir.path(), "delete-me").unwrap();

        // delete-me dir should be gone.
        assert!(!dir.path().join("ai/skills/delete-me").exists());
        // keep-me dir should remain.
        assert!(dir.path().join("ai/skills/keep-me/skill.toml").exists());

        let cfg = SkillsConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(cfg.skills.len(), 1);
        assert_eq!(cfg.skills[0].name, "keep-me");
    }

    // ── discover helpers ─────────────────────────────────────────────────────

    #[test]
    fn update_platforms_active_creates_file() {
        let dir = TempDir::new().unwrap();
        update_platforms_active(dir.path(), &["claude-code".to_string(), "codex".to_string()])
            .unwrap();

        let cfg = crate::ai_platform::PlatformsConfig::load(dir.path())
            .unwrap()
            .unwrap();
        assert_eq!(cfg.active, ["claude-code", "codex"]);
    }

    #[test]
    fn update_platforms_active_appends_to_existing() {
        let dir = TempDir::new().unwrap();
        let ai = dir.path().join("ai");
        std::fs::create_dir_all(&ai).unwrap();
        std::fs::write(
            ai.join("platforms.toml"),
            r#"active = ["claude-code"]
"#,
        )
        .unwrap();

        update_platforms_active(dir.path(), &["codex".to_string()]).unwrap();

        let cfg = crate::ai_platform::PlatformsConfig::load(dir.path())
            .unwrap()
            .unwrap();
        assert!(cfg.active.contains(&"claude-code".to_string()));
        assert!(cfg.active.contains(&"codex".to_string()));
    }

    // ── fetch / update helpers (unit — no network) ───────────────────────────

    #[test]
    fn filter_skills_returns_all_when_name_is_none() {
        let dir = TempDir::new().unwrap();
        write_skill_dir_test(&dir, "a", "gh:owner/a");
        write_skill_dir_test(&dir, "b", "gh:owner/b");

        let cfg = SkillsConfig::load(dir.path()).unwrap().unwrap();
        let filtered = filter_skills(&cfg.skills, None);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_skills_returns_named_only() {
        let dir = TempDir::new().unwrap();
        write_skill_dir_test(&dir, "a", "gh:owner/a");
        write_skill_dir_test(&dir, "b", "gh:owner/b");

        let cfg = SkillsConfig::load(dir.path()).unwrap().unwrap();
        let filtered = filter_skills(&cfg.skills, Some("a"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "a");
    }
}
