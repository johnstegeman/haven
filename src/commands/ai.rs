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
