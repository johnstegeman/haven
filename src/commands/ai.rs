/// `dfiles ai` subcommands: discover, add, fetch, update, remove.
///
/// These commands manage the lifecycle of AI skills declared in `ai/skills.toml`:
///
/// - `discover`  — scan for installed AI platforms, offer to update `active` list
/// - `add`       — append a new `[[skill]]` entry to `ai/skills.toml`
/// - `fetch`     — download `gh:` skills into the local cache (respects lock SHA)
/// - `update`    — like `fetch` but clears the lock SHA first to force re-download
/// - `remove`    — remove a skill from the config, lock file, and optionally live dirs
use anyhow::{Context, Result};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::ai_platform::{platform_registry, PlatformsConfig};
use crate::ai_skill::{DeployMethod, SkillSource, SkillsConfig};
use crate::lock::LockFile;
use crate::skill_cache::SkillCache;
use crate::state::State;

// ─── discover ─────────────────────────────────────────────────────────────────

/// Options for `dfiles ai discover`.
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
             Install a platform binary and re-run `dfiles ai discover`."
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
    println!("Run `dfiles apply --ai` to deploy skills to the newly-added platforms.");
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
    std::process::Command::new("which")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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

/// Options for `dfiles ai add`.
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

/// Add a new `[[skill]]` entry to `ai/skills.toml`.
pub fn add(opts: &AddOptions<'_>) -> Result<()> {
    // Validate source.
    let parsed = SkillSource::parse(opts.source)
        .context("Invalid source")?;

    // Infer name if not explicitly given.
    let name = match opts.name {
        Some(n) => n.to_string(),
        None => infer_name_from_source(&parsed),
    };

    // Check for name conflicts in existing skills.toml.
    if let Some(existing) = SkillsConfig::load(opts.repo_root)? {
        if existing.skills.iter().any(|s| s.name == name) {
            anyhow::bail!(
                "A skill named '{}' already exists in ai/skills.toml.\n\
                 Use --name to specify a different name.",
                name
            );
        }
    }

    // Validate deploy method.
    let _ = parse_deploy_method(opts.deploy)?;

    // Build the TOML entry and append it.
    append_skill_entry(opts.repo_root, &name, opts.source, opts.platforms, opts.deploy)?;

    println!("Added skill '{}' to ai/skills.toml.", name);
    println!();
    println!("Run `dfiles apply --ai` to deploy it, or `dfiles ai fetch` to pre-warm the cache.");
    Ok(())
}

/// Infer a skill name from its source string.
///
/// - `gh:anthropics/skills/pdf-processing` → `"pdf-processing"` (last subpath component)
/// - `gh:anthropics/pdf-skill` → `"pdf-skill"` (repo name)
/// - `dir:~/projects/my-skill` → `"my-skill"` (last path component)
fn infer_name_from_source(source: &SkillSource) -> String {
    match source {
        SkillSource::Gh(gh) => gh.name().to_string(),
        SkillSource::Dir(path) => path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "skill".to_string()),
    }
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

/// Append a `[[skill]]` entry to `ai/skills.toml` using toml_edit
/// (preserves existing content and formatting).
fn append_skill_entry(
    repo_root: &Path,
    name: &str,
    source: &str,
    platforms: &str,
    deploy: &str,
) -> Result<()> {
    let ai_dir = repo_root.join("ai");
    std::fs::create_dir_all(&ai_dir).context("Cannot create ai/ directory")?;
    let path = ai_dir.join("skills.toml");

    let text = if path.exists() {
        std::fs::read_to_string(&path)
            .with_context(|| format!("Cannot read {}", path.display()))?
    } else {
        String::new()
    };

    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .context("ai/skills.toml contains invalid TOML")?;

    // Build the new [[skill]] table.
    let mut table = toml_edit::Table::new();
    table.insert("name", toml_edit::value(name));
    table.insert("source", toml_edit::value(source));

    // `platforms` is either a named string ("all", "cross-client") or a
    // comma-separated list of IDs ("claude-code,codex").
    if platforms == "all" || platforms == "cross-client" {
        table.insert("platforms", toml_edit::value(platforms));
    } else {
        let ids: Vec<&str> = platforms.split(',').map(str::trim).collect();
        if ids.len() == 1 {
            // Single platform ID — still store as an array for consistency.
            let mut arr = toml_edit::Array::new();
            arr.push(ids[0]);
            table.insert("platforms", toml_edit::value(arr));
        } else {
            let mut arr = toml_edit::Array::new();
            for id in &ids {
                arr.push(*id);
            }
            table.insert("platforms", toml_edit::value(arr));
        }
    }

    // Only write `deploy` when it's not the default ("symlink").
    if deploy != "symlink" {
        table.insert("deploy", toml_edit::value(deploy));
    }

    // Get or create the `[[skill]]` array of tables.
    if !doc.contains_key("skill") {
        doc.insert(
            "skill",
            toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()),
        );
    }
    doc["skill"]
        .as_array_of_tables_mut()
        .context("'skill' in ai/skills.toml is not an array of tables")?
        .push(table);

    std::fs::write(&path, doc.to_string())
        .with_context(|| format!("Cannot write {}", path.display()))
}

// ─── fetch ────────────────────────────────────────────────────────────────────

/// Options for `dfiles ai fetch`.
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
            SkillSource::Dir(_) => {
                // dir: skills are read directly at apply time — no cache needed.
                if opts.name.is_some() {
                    println!(
                        "Skill '{}' uses a dir: source — nothing to fetch.",
                        decl.name
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

/// Options for `dfiles ai update`.
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
            SkillSource::Dir(_) => {
                if opts.name.is_some() {
                    println!(
                        "Skill '{}' uses a dir: source — nothing to update.",
                        decl.name
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

/// Options for `dfiles ai remove`.
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
pub fn remove(opts: &RemoveOptions<'_>) -> Result<()> {
    let skills = load_skills_required(opts.repo_root)?;

    // Verify the skill exists.
    let decl = skills
        .skills
        .iter()
        .find(|s| s.name == opts.name)
        .with_context(|| {
            format!(
                "No skill named '{}' found in ai/skills.toml.",
                opts.name
            )
        })?;

    let source_str = decl.source.clone();

    // Confirm removal from config.
    if !opts.yes {
        print!(
            "Remove skill '{}' ({}) from ai/skills.toml? [y/N] ",
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

    // Remove from ai/skills.toml.
    remove_skill_entry(opts.repo_root, opts.name)?;
    println!("Removed '{}' from ai/skills.toml.", opts.name);

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

/// Remove the `[[skill]]` entry with the given `name` from `ai/skills.toml`.
fn remove_skill_entry(repo_root: &Path, name: &str) -> Result<()> {
    let path = repo_root.join("ai").join("skills.toml");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Cannot read {}", path.display()))?;

    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .context("ai/skills.toml contains invalid TOML")?;

    let aot = doc["skill"]
        .as_array_of_tables_mut()
        .context("'skill' in ai/skills.toml is not an array of tables")?;

    let idx = aot
        .iter()
        .position(|t| t.get("name").and_then(|v| v.as_str()) == Some(name))
        .with_context(|| format!("Skill '{}' not found in ai/skills.toml", name))?;

    aot.remove(idx);

    std::fs::write(&path, doc.to_string())
        .with_context(|| format!("Cannot write {}", path.display()))
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

/// Options for `dfiles ai search`.
pub struct SearchOptions<'a> {
    pub query: &'a str,
    pub limit: u8,
}

/// Search skills.sh and display matching skills with their gh: sources.
pub fn search(opts: &SearchOptions<'_>) -> Result<()> {
    print!("Searching skills.sh for '{}' ...", opts.query);
    io::stdout().flush()?;

    let results = skillssh_search(opts.query, opts.limit as usize)?;
    println!();

    if results.is_empty() {
        println!("No results found for '{}'.", opts.query);
        return Ok(());
    }

    println!("{} result(s):\n", results.len());
    for entry in &results {
        println!("  {}  ({} installs)", entry.gh_source(), entry.installs);
    }
    println!();
    println!("To add a skill:  dfiles ai add <gh:source>");
    Ok(())
}

// ─── scan ─────────────────────────────────────────────────────────────────────

/// Options for `dfiles ai scan`.
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

        // Skip if already managed by dfiles (points into the skill cache).
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

        let proposed = match &detected {
            Some(src) => {
                let detail = git_remote_url(real_path)
                    .unwrap_or_else(|| "git remote".to_string());
                println!("  Detected: {}  ({})", src, detail);
                src.clone()
            }
            None => {
                // Fall back to skills.sh search.
                print!("  No git remote — searching skills.sh for '{}' ...", name);
                io::stdout().flush()?;
                let results = skillssh_search(name, 5).unwrap_or_default();
                println!();

                if results.is_empty() {
                    println!("  No matches found on skills.sh.");
                    println!("  Skip with 'n' or enter a source manually with 'e'.");
                    String::new()
                } else {
                    let best = &results[0];
                    let dup_note = if results.len() > 1 {
                        format!("  ({} results — use '?' to see all)", results.len())
                    } else {
                        String::new()
                    };
                    println!(
                        "  Best match: {}  ({} installs){}",
                        best.gh_source(), best.installs, dup_note
                    );
                    best.gh_source()
                }
            }
        };

        // Interactive prompt loop.
        let all_results: Option<Vec<SkillsShEntry>> = if detected.is_none() {
            Some(skillssh_search(name, 10).unwrap_or_default())
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
                        append_skill_entry(opts.repo_root, name, &proposed, "all", "symlink")?;
                        println!("  Added {}.", proposed);
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
                                append_skill_entry(opts.repo_root, name, &src, "all", "symlink")?;
                                println!("  Added {}.", src);
                            }
                            added += 1;
                            break;
                        }
                    }
                }
                "?" => {
                    let results = all_results.as_deref().unwrap_or(&[]);
                    if results.is_empty() {
                        println!("  No skills.sh results available.");
                    } else {
                        for (i, r) in results.iter().enumerate() {
                            println!("    {}. {}  ({} installs)", i + 1, r.gh_source(), r.installs);
                        }
                        print!("  Pick [1-{}] or (n)ext/(e)nter manually: ", results.len());
                        io::stdout().flush()?;
                        let mut pick = String::new();
                        io::stdin().read_line(&mut pick)?;
                        let pick = pick.trim();
                        if let Ok(n) = pick.parse::<usize>() {
                            if n >= 1 && n <= results.len() {
                                let chosen = results[n - 1].gh_source();
                                if opts.dry_run {
                                    println!("  (dry-run) Would add: {}", chosen);
                                } else {
                                    append_skill_entry(opts.repo_root, name, &chosen, "all", "symlink")?;
                                    println!("  Added {}.", chosen);
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
        println!("{} skill(s) added to ai/skills.toml.", added);
        println!("Run `dfiles apply --ai` to deploy.");
    } else {
        println!("No skills added.");
    }

    Ok(())
}

// ─── skills.sh API ────────────────────────────────────────────────────────────

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

impl SkillsShEntry {
    fn gh_source(&self) -> String {
        format!("gh:{}", self.id)
    }
}

fn skillssh_search(query: &str, limit: usize) -> Result<Vec<SkillsShEntry>> {
    let response = ureq::get("https://skills.sh/api/search")
        .query("q", query)
        .query("limit", &limit.to_string())
        .set("User-Agent", "dfiles/0.1 (+https://github.com/dfiles-sh/dfiles)")
        .call()
        .context("skills.sh request failed")?
        .into_string()
        .context("skills.sh response was not UTF-8")?;

    let parsed: SkillsShResponse = serde_json::from_str(&response)
        .context("skills.sh response could not be parsed")?;

    Ok(parsed.skills)
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

/// Load `ai/skills.toml` and return an error if it doesn't exist.
fn load_skills_required(repo_root: &Path) -> Result<SkillsConfig> {
    SkillsConfig::load(repo_root)?
        .context("ai/skills.toml not found. Use `dfiles ai add` to declare skills first.")
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

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_skills(dir: &TempDir, content: &str) {
        let ai = dir.path().join("ai");
        std::fs::create_dir_all(&ai).unwrap();
        std::fs::write(ai.join("skills.toml"), content).unwrap();
    }

    // ── add ──────────────────────────────────────────────────────────────────

    #[test]
    fn add_creates_skills_toml_when_absent() {
        let dir = TempDir::new().unwrap();
        add(&AddOptions {
            repo_root: dir.path(),
            source: "gh:anthropics/skills/pdf-processing",
            name: None,
            platforms: "all",
            deploy: "symlink",
        })
        .unwrap();

        let text = std::fs::read_to_string(dir.path().join("ai/skills.toml")).unwrap();
        assert!(text.contains("pdf-processing"));
        assert!(text.contains("gh:anthropics/skills/pdf-processing"));
        assert!(text.contains(r#"platforms = "all""#));
        // Default deploy method (symlink) is omitted.
        assert!(!text.contains("deploy"));
    }

    #[test]
    fn add_appends_to_existing_skills_toml() {
        let dir = TempDir::new().unwrap();
        write_skills(
            &dir,
            r#"
[[skill]]
name = "existing"
source = "gh:owner/existing"
platforms = "all"
"#,
        );

        add(&AddOptions {
            repo_root: dir.path(),
            source: "gh:owner/new-skill",
            name: Some("new-skill"),
            platforms: "claude-code,codex",
            deploy: "copy",
        })
        .unwrap();

        let text = std::fs::read_to_string(dir.path().join("ai/skills.toml")).unwrap();
        assert!(text.contains("existing"));
        assert!(text.contains("new-skill"));
        assert!(text.contains("deploy = \"copy\""));
    }

    #[test]
    fn add_rejects_duplicate_name() {
        let dir = TempDir::new().unwrap();
        write_skills(
            &dir,
            r#"
[[skill]]
name = "pdf-processing"
source = "gh:owner/pdf"
platforms = "all"
"#,
        );

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
        let text = std::fs::read_to_string(dir.path().join("ai/skills.toml")).unwrap();
        assert!(text.contains(r#"name = "pdf-processing""#));
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
        let text = std::fs::read_to_string(dir.path().join("ai/skills.toml")).unwrap();
        assert!(text.contains(r#"name = "my-skill-repo""#));
    }

    #[test]
    fn add_platforms_list_stored_as_array() {
        let dir = TempDir::new().unwrap();
        add(&AddOptions {
            repo_root: dir.path(),
            source: "gh:owner/skill",
            name: Some("my-skill"),
            platforms: "claude-code,codex",
            deploy: "symlink",
        })
        .unwrap();
        let text = std::fs::read_to_string(dir.path().join("ai/skills.toml")).unwrap();
        // Should be an array, not a string.
        assert!(text.contains("claude-code"));
        assert!(text.contains("codex"));
        assert!(!text.contains(r#"platforms = "claude-code"#));
    }

    // ── remove_skill_entry ───────────────────────────────────────────────────

    #[test]
    fn remove_entry_deletes_named_skill() {
        let dir = TempDir::new().unwrap();
        write_skills(
            &dir,
            r#"
[[skill]]
name = "keep-me"
source = "gh:owner/keep"
platforms = "all"

[[skill]]
name = "delete-me"
source = "gh:owner/delete"
platforms = "all"
"#,
        );

        remove_skill_entry(dir.path(), "delete-me").unwrap();

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
        write_skills(
            &dir,
            r#"
[[skill]]
name = "a"
source = "gh:owner/a"
platforms = "all"

[[skill]]
name = "b"
source = "gh:owner/b"
platforms = "all"
"#,
        );
        let cfg = SkillsConfig::load(dir.path()).unwrap().unwrap();
        let filtered = filter_skills(&cfg.skills, None);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_skills_returns_named_only() {
        let dir = TempDir::new().unwrap();
        write_skills(
            &dir,
            r#"
[[skill]]
name = "a"
source = "gh:owner/a"
platforms = "all"

[[skill]]
name = "b"
source = "gh:owner/b"
platforms = "all"
"#,
        );
        let cfg = SkillsConfig::load(dir.path()).unwrap().unwrap();
        let filtered = filter_skills(&cfg.skills, Some("a"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "a");
    }
}
