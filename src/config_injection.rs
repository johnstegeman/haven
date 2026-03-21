/// Managed section injection for AI skill snippets.
///
/// Platform config files (e.g. `~/.claude/CLAUDE.md`) can contain a
/// dfiles-managed section bounded by HTML comment markers:
///
/// ```markdown
/// <!-- dfiles managed start -->
/// <!-- managed by dfiles (dfiles apply to regenerate) -->
/// ...generated content...
/// <!-- dfiles managed end -->
/// ```
///
/// On `dfiles apply`, dfiles:
///   1. Collects snippets from `ai/skills/<name>/all.md` and
///      `ai/skills/<name>/<platform-id>.md` for each deployed skill.
///   2. Assembles a managed section (skills list + snippets).
///   3. Replaces the content between the markers in the destination file.
///
/// If the destination has no markers but there are snippets pending,
/// the user is prompted to add a managed section (TTY) or shown a notice
/// (non-interactive). If the destination doesn't exist and there are
/// snippets, a minimal stub file is created.
use anyhow::{Context, Result};
use std::io::{self, Write as IoWrite};
use std::path::{Path, PathBuf};

use crate::ai_platform::PlatformPlugin;
use crate::ai_skill::SkillDeclaration;
use crate::state::State;

// ─── Constants ────────────────────────────────────────────────────────────────

pub const MARKER_START: &str = "<!-- dfiles managed start -->";
pub const MARKER_END: &str = "<!-- dfiles managed end -->";
const MANAGED_HEADER: &str =
    "<!-- managed by dfiles (dfiles apply to regenerate) -->";

// ─── Marker utilities ─────────────────────────────────────────────────────────

/// Find the byte-offset range of the managed section in `content`.
///
/// Returns `Ok(Some((start, end)))` where `start` is the byte offset of the
/// first character *after* the `MARKER_START` line's newline, and `end` is the
/// byte offset of the first character of the `MARKER_END` line.
///
/// Returns `Ok(None)` when neither marker is present (no managed section).
/// Returns `Err` when only one marker is present (mismatched markers).
pub fn find_markers(content: &str) -> Result<Option<(usize, usize)>> {
    let start_idx = content.find(MARKER_START);
    let end_idx = content.find(MARKER_END);

    match (start_idx, end_idx) {
        (None, None) => Ok(None),
        (Some(s), Some(e)) => {
            if s >= e {
                anyhow::bail!(
                    "Managed section markers are out of order: \
                     '{}' appears after '{}'",
                    MARKER_START,
                    MARKER_END
                );
            }
            // Advance past the MARKER_START line (including its trailing newline).
            let after_start = s + MARKER_START.len();
            let inner_start = if content.as_bytes().get(after_start) == Some(&b'\n') {
                after_start + 1
            } else {
                after_start
            };
            Ok(Some((inner_start, e)))
        }
        (Some(_), None) => anyhow::bail!(
            "Found '{}' but no matching '{}'",
            MARKER_START,
            MARKER_END
        ),
        (None, Some(_)) => anyhow::bail!(
            "Found '{}' but no matching '{}'",
            MARKER_END,
            MARKER_START
        ),
    }
}

/// Strip the content between managed markers, leaving the markers themselves.
///
/// If no markers are present, returns the input unchanged.
/// If markers are mismatched, returns the input unchanged (with a warning printed).
pub fn strip_managed_content(content: &str) -> String {
    match find_markers(content) {
        Ok(Some((inner_start, inner_end))) => {
            // Replace the inner content with nothing (empty line between markers).
            let before = &content[..inner_start];
            let after = &content[inner_end..];
            format!("{}{}", before, after)
        }
        Ok(None) => content.to_string(),
        Err(e) => {
            eprintln!("warning: cannot strip managed section: {}", e);
            content.to_string()
        }
    }
}

/// Replace the content between managed markers with `new_inner`.
///
/// `inner_start` and `inner_end` are byte offsets as returned by [`find_markers`].
fn replace_managed_section(
    content: &str,
    inner_start: usize,
    inner_end: usize,
    new_inner: &str,
) -> String {
    let before = &content[..inner_start];
    let after = &content[inner_end..];
    if new_inner.is_empty() {
        format!("{}{}", before, after)
    } else {
        format!("{}{}\n{}", before, new_inner, after)
    }
}

// ─── Snippet collection ───────────────────────────────────────────────────────

/// Collect the snippet content for a skill on a given platform.
///
/// Returns the concatenation of `all.md` (if present and non-empty)
/// followed by `<platform_id>.md` (if present and non-empty), separated by
/// a blank line.
fn read_snippet(repo_root: &Path, skill_name: &str, platform_id: &str) -> String {
    let skill_dir = repo_root.join("ai").join("skills").join(skill_name);

    let all_content = read_nonempty(&skill_dir.join("all.md"));
    let platform_content = read_nonempty(&skill_dir.join(format!("{}.md", platform_id)));

    match (all_content, platform_content) {
        (Some(a), Some(p)) => format!("{}\n\n{}", a.trim_end(), p.trim_end()),
        (Some(a), None) => a.trim_end().to_string(),
        (None, Some(p)) => p.trim_end().to_string(),
        (None, None) => String::new(),
    }
}

/// Read a file if it exists and has non-whitespace content.
fn read_nonempty(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    if content.trim().is_empty() {
        None
    } else {
        Some(content)
    }
}

// ─── Skills list ──────────────────────────────────────────────────────────────

/// Build the `## Installed Skills` and `## Slash Commands` sections for a
/// platform's managed content, scanning `skills_dir` and optionally
/// `config_dir/commands/`.
fn build_skills_section(skills_dir: &Path, commands_dir: Option<&Path>) -> String {
    let skills = crate::claude_md::scan_skills(skills_dir);
    let commands = commands_dir
        .map(|d| crate::claude_md::scan_commands(d))
        .unwrap_or_default();

    let mut out = String::new();

    if !skills.is_empty() {
        out.push_str("## Installed Skills\n");
        for (name, desc) in &skills {
            out.push_str(&format!("- /{}: {}\n", name, desc));
        }
    }

    if !commands.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("## Slash Commands\n");
        for (name, desc) in &commands {
            out.push_str(&format!("- /{}: {}\n", name, desc));
        }
    }

    out
}

// ─── Managed content assembly ─────────────────────────────────────────────────

/// Assemble the full content to be placed between the managed markers.
///
/// Order: MANAGED_HEADER → skills list → per-skill snippets (all.md first,
/// then platform-specific), one per skill in declaration order.
fn assemble_managed_content(
    repo_root: &Path,
    skills: &[SkillDeclaration],
    platform: &PlatformPlugin,
) -> String {
    let commands_dir = platform.config_dir.as_deref().map(|d| d.join("commands"));
    let skills_section =
        build_skills_section(&platform.skills_dir, commands_dir.as_deref());

    // Collect per-skill snippets.
    let mut snippet_blocks: Vec<String> = Vec::new();
    for skill in skills {
        let content = read_snippet(repo_root, &skill.name, &platform.id);
        if !content.is_empty() {
            snippet_blocks.push(format!(
                "<!-- skill: {} -->\n{}\n<!-- /skill: {} -->",
                skill.name,
                content.trim_end(),
                skill.name
            ));
        }
    }

    // If there's nothing to write, return empty so the managed section stays clean.
    if skills_section.is_empty() && snippet_blocks.is_empty() {
        return String::new();
    }

    let mut parts: Vec<String> = vec![MANAGED_HEADER.to_string()];
    if !skills_section.is_empty() {
        parts.push(String::new()); // blank line
        parts.push(skills_section.trim_end().to_string());
    }
    for block in snippet_blocks {
        parts.push(String::new()); // blank line before each snippet block
        parts.push(block);
    }

    parts.join("\n")
}

/// Returns true if any skills have a non-empty snippet for `platform_id`.
fn has_snippets(repo_root: &Path, skills: &[SkillDeclaration], platform_id: &str) -> bool {
    skills
        .iter()
        .any(|s| !read_snippet(repo_root, &s.name, platform_id).is_empty())
}

// ─── Stub creation ────────────────────────────────────────────────────────────

/// Create a minimal config stub with empty markers.
///
/// ```markdown
/// # <Platform Name>
///
/// <!-- dfiles managed start -->
/// <!-- dfiles managed end -->
/// ```
fn create_stub(dest: &Path, platform_name: &str) -> Result<()> {
    let content = format!(
        "# {}\n\n{}\n{}\n",
        platform_name, MARKER_START, MARKER_END
    );
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create directory {}", parent.display()))?;
    }
    std::fs::write(dest, content)
        .with_context(|| format!("Cannot write stub {}", dest.display()))
}

// ─── Marker insertion ─────────────────────────────────────────────────────────

/// Where to insert the managed section markers when adding to an existing file.
#[derive(Debug, Clone, PartialEq)]
pub enum MarkerPosition {
    Beginning,
    End,
}

/// Insert empty managed-section markers into an existing source file.
pub fn insert_markers(path: &Path, position: MarkerPosition) -> Result<()> {
    let existing = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read {}", path.display()))?;

    let marker_block = format!("{}\n{}\n", MARKER_START, MARKER_END);
    let new_content = match position {
        MarkerPosition::Beginning => format!("{}\n{}", marker_block, existing),
        MarkerPosition::End => {
            let trimmed = existing.trim_end();
            format!("{}\n\n{}", trimmed, marker_block)
        }
    };

    std::fs::write(path, new_content)
        .with_context(|| format!("Cannot write {}", path.display()))
}

// ─── Interactive prompt ───────────────────────────────────────────────────────

fn prompt_add_markers(config_path: &Path) -> Result<Option<MarkerPosition>> {
    let display = tilde_str(&config_path.to_string_lossy());
    println!(
        "dfiles: {} has no managed section.",
        display
    );
    print!(
        "Skills have snippets to inject — where should the section go?\n  \
         [B] beginning of file\n  \
         [E] end of file\n  \
         [S] skip (don't ask again for this file)\n> "
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    match input.trim().to_uppercase().as_str() {
        "B" => Ok(Some(MarkerPosition::Beginning)),
        "E" => Ok(Some(MarkerPosition::End)),
        _ => Ok(None), // S or anything else → skip
    }
}

// ─── Dry-run diff ─────────────────────────────────────────────────────────────

fn print_injection_diff(
    config_path: &Path,
    current: &str,
    inner_start: usize,
    inner_end: usize,
    new_inner: &str,
) {
    let display = tilde_str(&config_path.to_string_lossy());
    println!("~ {}  (managed section)", display);

    let current_inner = &current[inner_start..inner_end];

    // Simple unified-diff style output for the managed section.
    for line in current_inner.lines() {
        println!("  - {}", line);
    }
    for line in new_inner.lines() {
        println!("  + {}", line);
    }
    println!();
}

// ─── Main injection entry point ───────────────────────────────────────────────

/// Inject managed sections into platform config files for all active platforms.
///
/// Called after AI skills are deployed. For each platform with a `config_file`:
///
/// 1. Destination exists + has markers → inject
/// 2. Destination exists + no markers + snippets exist:
///    - TTY: prompt to add markers to source/ file
///    - No TTY: print a notice
/// 3. Destination absent + no source file + snippets exist → create stub + inject
/// 4. Destination absent + source file exists → skip (files phase owns the copy)
/// 5. No snippets → skip
pub fn inject_managed_sections(
    repo_root: &Path,
    skills: &[SkillDeclaration],
    active_platforms: &[PlatformPlugin],
    state: &mut State,
    interactive: bool,
    dry_run: bool,
) -> Result<()> {
    for platform in active_platforms {
        let config_file = match &platform.config_file {
            Some(f) => f.clone(),
            None => continue,
        };

        let managed_content = assemble_managed_content(repo_root, skills, platform);
        let any_snippets = has_snippets(repo_root, skills, &platform.id);

        // Determine whether the source file exists in the dfiles repo
        // (to avoid creating a stub that conflicts with the user's source file).
        let source_exists = source_file_exists_for(repo_root, &config_file);

        let config_path = &config_file;

        if config_path.exists() {
            // Check for symlink — injecting into a symlink target would modify the repo.
            if config_path.is_symlink() {
                eprintln!(
                    "warning: {} is a symlink — skipping managed section injection. \
                     Use copy deployment for config files with managed sections.",
                    tilde_str(&config_path.to_string_lossy())
                );
                continue;
            }

            let current = std::fs::read_to_string(config_path)
                .with_context(|| format!("Cannot read {}", config_path.display()))?;

            match find_markers(&current) {
                Ok(Some((inner_start, inner_end))) => {
                    if dry_run {
                        print_injection_diff(
                            config_path,
                            &current,
                            inner_start,
                            inner_end,
                            &managed_content,
                        );
                        continue;
                    }
                    let new_content = replace_managed_section(
                        &current,
                        inner_start,
                        inner_end,
                        &managed_content,
                    );
                    if new_content != current {
                        std::fs::write(config_path, &new_content).with_context(|| {
                            format!("Cannot write {}", config_path.display())
                        })?;
                        println!(
                            "✓ {}  (managed section updated: {} skill(s))",
                            tilde_str(&config_path.to_string_lossy()),
                            skills.len()
                        );
                    }
                    // Silent when unchanged (idempotent re-apply).
                }
                Ok(None) => {
                    // No markers. Only act if there are snippets to inject.
                    if !any_snippets {
                        continue;
                    }

                    let path_str = config_path.to_string_lossy().to_string();
                    if state.skipped_managed_files.contains(&path_str) {
                        continue;
                    }

                    if dry_run {
                        println!(
                            "note: {} has no managed section ({} snippet(s) pending)",
                            tilde_str(&config_path.to_string_lossy()),
                            skills.len()
                        );
                        continue;
                    }

                    if interactive {
                        match prompt_add_markers(config_path)? {
                            Some(position) => {
                                // Find the corresponding source file and add markers there.
                                let source_path = find_source_path(repo_root, config_path);
                                if let Some(sp) = source_path {
                                    insert_markers(&sp, position.clone())?;
                                    println!(
                                        "Added managed section markers to source/{}",
                                        sp.strip_prefix(repo_root.join("source"))
                                            .unwrap_or(&sp)
                                            .display()
                                    );
                                    // Also insert markers into the live destination so
                                    // injection takes effect on this run (not just next apply).
                                    insert_markers(config_path, position)?;
                                    let updated = std::fs::read_to_string(config_path)?;
                                    if let Ok(Some((is, ie))) = find_markers(&updated) {
                                        let new_content = replace_managed_section(
                                            &updated, is, ie, &managed_content,
                                        );
                                        std::fs::write(config_path, &new_content)?;
                                    }
                                } else {
                                    // No source file — add markers directly to destination.
                                    insert_markers(config_path, position)?;
                                    let updated = std::fs::read_to_string(config_path)?;
                                    if let Ok(Some((is, ie))) = find_markers(&updated) {
                                        let new_content = replace_managed_section(
                                            &updated, is, ie, &managed_content,
                                        );
                                        std::fs::write(config_path, &new_content)?;
                                    }
                                }
                            }
                            None => {
                                // User chose skip.
                                state.skipped_managed_files.push(path_str);
                            }
                        }
                    } else {
                        println!(
                            "note: {} skill snippet(s) pending for {} (no managed section) \
                             — run dfiles apply in a terminal to configure",
                            skills.len(),
                            tilde_str(&config_path.to_string_lossy())
                        );
                    }
                }
                Err(e) => {
                    eprintln!(
                        "warning: {}: mismatched managed section markers — skipping injection: {}",
                        tilde_str(&config_path.to_string_lossy()),
                        e
                    );
                }
            }
        } else if !source_exists && any_snippets {
            // Destination absent, no source file, snippets exist → create stub.
            if dry_run {
                println!(
                    "+ {}  (new stub with managed section)",
                    tilde_str(&config_path.to_string_lossy())
                );
                continue;
            }
            create_stub(config_path, &platform.name)?;
            // Now inject into the freshly-created stub.
            let stub_content = std::fs::read_to_string(config_path)?;
            if let Ok(Some((is, ie))) = find_markers(&stub_content) {
                let new_content =
                    replace_managed_section(&stub_content, is, ie, &managed_content);
                std::fs::write(config_path, &new_content)?;
            }
            println!(
                "✓ {}  (created with managed section: {} skill(s))",
                tilde_str(&config_path.to_string_lossy()),
                skills.len()
            );
        }
        // If source_exists but dest is absent: files phase owns this — skip.
    }
    Ok(())
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Return true if a source file exists in `repo_root/source/` that would
/// map to `config_path` after apply.
///
/// Uses a simple heuristic: look for a file in `source/` whose decoded
/// destination tilde path resolves to `config_path`.
fn source_file_exists_for(repo_root: &Path, config_path: &Path) -> bool {
    let source_dir = repo_root.join("source");
    if !source_dir.exists() {
        return false;
    }
    let ignore = crate::ignore::IgnoreList::load(repo_root);
    if let Ok(entries) = crate::source::scan(&source_dir, &ignore) {
        return entries.iter().any(|e| dest_matches(&e.dest_tilde, config_path));
    }
    false
}

/// Find the source/ path that maps to `config_path`, if one exists.
fn find_source_path(repo_root: &Path, config_path: &Path) -> Option<PathBuf> {
    let source_dir = repo_root.join("source");
    if !source_dir.exists() {
        return None;
    }
    let ignore = crate::ignore::IgnoreList::load(repo_root);
    if let Ok(entries) = crate::source::scan(&source_dir, &ignore) {
        return entries
            .iter()
            .find(|e| dest_matches(&e.dest_tilde, config_path))
            .map(|e| e.src.clone());
    }
    None
}

/// Return true if `dest_tilde` (e.g. `~/.claude/CLAUDE.md`) resolves to `path`.
fn dest_matches(dest_tilde: &str, path: &Path) -> bool {
    if let Some(home) = dirs::home_dir() {
        if let Some(rel) = dest_tilde.strip_prefix("~/") {
            return home.join(rel) == path;
        } else if dest_tilde == "~" {
            return home == path;
        }
    }
    // Fallback: direct string comparison.
    Path::new(dest_tilde) == path
}

/// Replace `~` prefix for display purposes.
fn tilde_str(s: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if s.starts_with(home_str.as_ref()) {
            return format!("~{}", &s[home_str.len()..]);
        }
    }
    s.to_string()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    // ── find_markers ──────────────────────────────────────────────────────────

    #[test]
    fn find_markers_returns_none_when_absent() {
        let content = "# My config\nSome content\n";
        assert!(find_markers(content).unwrap().is_none());
    }

    #[test]
    fn find_markers_returns_positions() {
        let content = format!(
            "before\n{}\ninner line\n{}\nafter\n",
            MARKER_START, MARKER_END
        );
        let (start, end) = find_markers(&content).unwrap().unwrap();
        let inner = &content[start..end];
        assert_eq!(inner, "inner line\n");
    }

    #[test]
    fn find_markers_empty_inner() {
        let content = format!("{}\n{}\n", MARKER_START, MARKER_END);
        let (start, end) = find_markers(&content).unwrap().unwrap();
        assert_eq!(start, end, "inner should be empty");
    }

    #[test]
    fn find_markers_errors_on_start_only() {
        let content = format!("text\n{}\nno end\n", MARKER_START);
        assert!(find_markers(&content).is_err());
    }

    #[test]
    fn find_markers_errors_on_end_only() {
        let content = format!("text\n{}\n", MARKER_END);
        assert!(find_markers(&content).is_err());
    }

    #[test]
    fn find_markers_errors_when_out_of_order() {
        let content = format!("{}\nbefore\n{}\n", MARKER_END, MARKER_START);
        assert!(find_markers(&content).is_err());
    }

    // ── strip_managed_content ─────────────────────────────────────────────────

    #[test]
    fn strip_removes_content_between_markers() {
        let content = format!(
            "before\n{}\ngenerated stuff\n{}\nafter\n",
            MARKER_START, MARKER_END
        );
        let stripped = strip_managed_content(&content);
        assert!(stripped.contains(MARKER_START));
        assert!(stripped.contains(MARKER_END));
        assert!(!stripped.contains("generated stuff"));
        assert!(stripped.contains("before"));
        assert!(stripped.contains("after"));
    }

    #[test]
    fn strip_no_markers_returns_unchanged() {
        let content = "no markers here\n";
        assert_eq!(strip_managed_content(content), content);
    }

    // ── insert_markers ────────────────────────────────────────────────────────

    #[test]
    fn insert_at_end_appends_markers() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("file.md");
        write_file(&path, "# My Config\n\nSome content.\n");

        insert_markers(&path, MarkerPosition::End).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# My Config"));
        assert!(content.contains(MARKER_START));
        assert!(content.contains(MARKER_END));
        // Markers should appear after the content.
        let marker_pos = content.find(MARKER_START).unwrap();
        let content_pos = content.find("Some content").unwrap();
        assert!(marker_pos > content_pos);
    }

    #[test]
    fn insert_at_beginning_prepends_markers() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("file.md");
        write_file(&path, "# My Config\n\nSome content.\n");

        insert_markers(&path, MarkerPosition::Beginning).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let marker_pos = content.find(MARKER_START).unwrap();
        let content_pos = content.find("Some content").unwrap();
        assert!(marker_pos < content_pos);
    }

    // ── inject_managed_sections ───────────────────────────────────────────────

    #[test]
    fn injection_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let config = tmp.path().join("CLAUDE.md");
        write_file(
            &config,
            &format!("# Claude\n\n{}\n{}\n\nUser content.\n", MARKER_START, MARKER_END),
        );

        // Write a snippet stub for a skill.
        let snippet = tmp.path().join("ai").join("skills").join("my-skill").join("all.md");
        write_file(&snippet, "## My Skill\nDo the thing.\n");

        let skills = vec![crate::ai_skill::SkillDeclaration {
            name: "my-skill".to_string(),
            source: "dir:/tmp/fake".to_string(),
            platforms: crate::ai_skill::SkillPlatforms::Named("all".to_string()),
            deploy: crate::ai_skill::DeployMethod::Symlink,
        }];

        let platform = PlatformPlugin {
            id: "claude-code".to_string(),
            name: "Claude Code".to_string(),
            config_dir: None,
            skills_dir: tmp.path().join("skills"),
            config_file: Some(config.clone()),
            binary: None,
            agentskills_compliant: false,
        };

        let mut state = State::default();

        inject_managed_sections(tmp.path(), &skills, &[platform.clone()], &mut state, false, false)
            .unwrap();

        let first = std::fs::read_to_string(&config).unwrap();
        assert!(first.contains("Do the thing"));

        // Second apply should produce identical output.
        inject_managed_sections(tmp.path(), &skills, &[platform], &mut state, false, false)
            .unwrap();
        let second = std::fs::read_to_string(&config).unwrap();
        assert_eq!(first, second, "injection must be idempotent");
    }

    #[test]
    fn no_injection_when_no_snippets() {
        let tmp = TempDir::new().unwrap();
        let config = tmp.path().join("CLAUDE.md");
        write_file(
            &config,
            &format!("# Claude\n\n{}\n{}\n", MARKER_START, MARKER_END),
        );

        // Skill with blank all.md — should not inject anything.
        let snippet = tmp.path().join("ai").join("skills").join("my-skill").join("all.md");
        write_file(&snippet, "   \n");

        let skills = vec![crate::ai_skill::SkillDeclaration {
            name: "my-skill".to_string(),
            source: "dir:/tmp/fake".to_string(),
            platforms: crate::ai_skill::SkillPlatforms::Named("all".to_string()),
            deploy: crate::ai_skill::DeployMethod::Symlink,
        }];

        let platform = PlatformPlugin {
            id: "claude-code".to_string(),
            name: "Claude Code".to_string(),
            config_dir: None,
            skills_dir: tmp.path().join("skills"),
            config_file: Some(config.clone()),
            binary: None,
            agentskills_compliant: false,
        };

        let original = std::fs::read_to_string(&config).unwrap();
        let mut state = State::default();
        inject_managed_sections(tmp.path(), &skills, &[platform], &mut state, false, false)
            .unwrap();
        let after = std::fs::read_to_string(&config).unwrap();
        assert_eq!(original, after, "blank snippet should not modify file");
    }

    #[test]
    fn stub_created_when_dest_absent_and_snippets_exist() {
        let tmp = TempDir::new().unwrap();
        let config = tmp.path().join("subdir").join("CLAUDE.md");

        let snippet = tmp.path().join("ai").join("skills").join("my-skill").join("all.md");
        write_file(&snippet, "## My Skill\nDo the thing.\n");

        let skills = vec![crate::ai_skill::SkillDeclaration {
            name: "my-skill".to_string(),
            source: "dir:/tmp/fake".to_string(),
            platforms: crate::ai_skill::SkillPlatforms::Named("all".to_string()),
            deploy: crate::ai_skill::DeployMethod::Symlink,
        }];

        let platform = PlatformPlugin {
            id: "test-platform".to_string(),
            name: "Test Platform".to_string(),
            config_dir: None,
            skills_dir: tmp.path().join("skills"),
            config_file: Some(config.clone()),
            binary: None,
            agentskills_compliant: false,
        };

        let mut state = State::default();
        inject_managed_sections(tmp.path(), &skills, &[platform], &mut state, false, false)
            .unwrap();

        assert!(config.exists(), "stub file should be created");
        let content = std::fs::read_to_string(&config).unwrap();
        assert!(content.contains("# Test Platform"));
        assert!(content.contains(MARKER_START));
        assert!(content.contains("Do the thing"));
    }

    #[test]
    fn no_stub_when_no_snippets() {
        let tmp = TempDir::new().unwrap();
        let config = tmp.path().join("CLAUDE.md");
        // No snippet files at all.

        let skills: Vec<crate::ai_skill::SkillDeclaration> = vec![];
        let platform = PlatformPlugin {
            id: "test-platform".to_string(),
            name: "Test Platform".to_string(),
            config_dir: None,
            skills_dir: tmp.path().join("skills"),
            config_file: Some(config.clone()),
            binary: None,
            agentskills_compliant: false,
        };

        let mut state = State::default();
        inject_managed_sections(tmp.path(), &skills, &[platform], &mut state, false, false)
            .unwrap();

        assert!(!config.exists(), "no stub should be created when no snippets");
    }

    #[test]
    fn skip_persisted_in_state() {
        // Non-interactive mode with no-TTY should print notice but not crash.
        let tmp = TempDir::new().unwrap();
        let config = tmp.path().join("CLAUDE.md");
        write_file(&config, "# My Config\n\nNo markers here.\n");

        let snippet = tmp.path().join("ai").join("skills").join("my-skill").join("all.md");
        write_file(&snippet, "Some instructions.\n");

        let skills = vec![crate::ai_skill::SkillDeclaration {
            name: "my-skill".to_string(),
            source: "dir:/tmp/fake".to_string(),
            platforms: crate::ai_skill::SkillPlatforms::Named("all".to_string()),
            deploy: crate::ai_skill::DeployMethod::Symlink,
        }];

        let platform = PlatformPlugin {
            id: "test-platform".to_string(),
            name: "Test Platform".to_string(),
            config_dir: None,
            skills_dir: tmp.path().join("skills"),
            config_file: Some(config.clone()),
            binary: None,
            agentskills_compliant: false,
        };

        // Pre-populate skip list.
        let mut state = State::default();
        state
            .skipped_managed_files
            .push(config.to_string_lossy().to_string());

        // Should silently skip (no prompt, no notice) because it's in the skip list.
        inject_managed_sections(tmp.path(), &skills, &[platform], &mut state, false, false)
            .unwrap();
        let content = std::fs::read_to_string(&config).unwrap();
        assert!(!content.contains(MARKER_START), "skip list should prevent injection");
    }

    #[test]
    fn all_md_before_platform_specific() {
        let tmp = TempDir::new().unwrap();
        let config = tmp.path().join("CLAUDE.md");
        write_file(
            &config,
            &format!("# Claude\n\n{}\n{}\n", MARKER_START, MARKER_END),
        );

        let skill_dir = tmp.path().join("ai").join("skills").join("my-skill");
        write_file(&skill_dir.join("all.md"), "Global instructions.\n");
        write_file(&skill_dir.join("claude-code.md"), "Claude-specific instructions.\n");

        let skills = vec![crate::ai_skill::SkillDeclaration {
            name: "my-skill".to_string(),
            source: "dir:/tmp/fake".to_string(),
            platforms: crate::ai_skill::SkillPlatforms::Named("all".to_string()),
            deploy: crate::ai_skill::DeployMethod::Symlink,
        }];

        let platform = PlatformPlugin {
            id: "claude-code".to_string(),
            name: "Claude Code".to_string(),
            config_dir: None,
            skills_dir: tmp.path().join("skills"),
            config_file: Some(config.clone()),
            binary: None,
            agentskills_compliant: false,
        };

        let mut state = State::default();
        inject_managed_sections(tmp.path(), &skills, &[platform], &mut state, false, false)
            .unwrap();

        let content = std::fs::read_to_string(&config).unwrap();
        let global_pos = content.find("Global instructions").unwrap();
        let specific_pos = content.find("Claude-specific instructions").unwrap();
        assert!(global_pos < specific_pos, "all.md must appear before platform-specific");
    }

    #[test]
    fn mismatched_markers_skips_with_warning() {
        // Only MARKER_START present — find_markers returns Err.
        // inject_managed_sections should warn and skip, not panic.
        let tmp = TempDir::new().unwrap();
        let config = tmp.path().join("CLAUDE.md");
        write_file(
            &config,
            &format!("# Claude\n\n{}\nno end marker\n", MARKER_START),
        );

        let snippet = tmp.path().join("ai").join("skills").join("s").join("all.md");
        write_file(&snippet, "Something.\n");

        let skills = vec![crate::ai_skill::SkillDeclaration {
            name: "s".to_string(),
            source: "dir:/tmp/fake".to_string(),
            platforms: crate::ai_skill::SkillPlatforms::Named("all".to_string()),
            deploy: crate::ai_skill::DeployMethod::Symlink,
        }];

        let platform = PlatformPlugin {
            id: "test".to_string(),
            name: "Test".to_string(),
            config_dir: None,
            skills_dir: tmp.path().join("skills"),
            config_file: Some(config.clone()),
            binary: None,
            agentskills_compliant: false,
        };

        let original = std::fs::read_to_string(&config).unwrap();
        let mut state = State::default();
        inject_managed_sections(tmp.path(), &skills, &[platform], &mut state, false, false)
            .unwrap(); // must not error
        let after = std::fs::read_to_string(&config).unwrap();
        assert_eq!(original, after, "mismatched markers must leave file unchanged");
    }
}
