/// Show differences between the tracked source state and the live machine.
///
/// Data flow:
///
///   haven diff
///        │
///        ├── [files]   (gated: source::scan only runs when diff_files=true)
///        │     │
///        │     └── for each SourceEntry:
///        │           drift::check_drift_*()  →  DriftKind
///        │           if Modified:
///        │             template? → render → diff_util::unified_diff()
///        │             symlink?  → show target mismatch
///        │             binary?   → "binary files differ"
///        │             plain?    → diff_util::unified_diff()
///        │           if Missing       → "? path"
///        │           if SourceMissing → "! path"
///        │
///        ├── [brew]    (gated: diff_brews=true)
///        │     └── homebrew::brewfile_diff()
///        │           missing_* → "+ pkg  (in Brewfile, not installed)"
///        │           extra_*   → "- pkg  (installed, not in Brewfile)"
///        │
///        └── [ai]      (gated: diff_ai=true)
///              └── per module: check skill/command dirs exist
///                    missing → "- fetch skill: ..."
///
/// Exit code: caller sets exit 1 when run() returns Ok(true).
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::ai_skill::{SkillSource, SkillsConfig};
use crate::config::{sort_modules, HavenConfig, ModuleConfig};
use crate::config::module::expand_tilde;
use crate::diff_util::{colorize_diff, stat_line, unified_diff};
use crate::drift::{check_drift, check_drift_link, check_drift_link_template, check_drift_template, DriftKind};
use crate::ignore::IgnoreList;
use crate::lock::LockFile;
use crate::skill_cache::SkillCache;
use crate::source::scan;
use crate::template::TemplateContext;

/// Whether to emit ANSI color codes in diff output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorMode {
    /// Emit color codes when stdout is a tty; strip when piped.
    Auto,
    /// Always emit color codes.
    Always,
    /// Never emit color codes.
    Never,
}

pub struct DiffOptions<'a> {
    pub repo_root: &'a Path,
    /// Where live files reside. `/` in production; temp dir in tests.
    pub dest_root: &'a Path,
    /// `~/.haven` — used to find the skill cache for AI version drift checks.
    pub state_dir: &'a Path,
    pub profile: &'a str,
    /// When set, scope brew/AI diff to this module only.
    pub module_filter: Option<&'a str>,
    /// Include the files section.
    pub diff_files: bool,
    /// Include the brew section.
    pub diff_brews: bool,
    /// Include the AI section.
    pub diff_ai: bool,
    /// Show a stat summary instead of full diff content.
    pub stat_only: bool,
    pub color: ColorMode,
}

/// Run the diff command. Returns `true` if any drift was found.
pub fn run(opts: &DiffOptions<'_>) -> Result<bool> {
    let use_color = match opts.color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => {
            use is_terminal::IsTerminal;
            std::io::stdout().is_terminal()
        }
    };

    let config = HavenConfig::load(opts.repo_root).unwrap_or_default();
    let template_ctx = TemplateContext::from_env(opts.profile, opts.repo_root, config.data);
    let mut any_drift = false;

    // ── Files ─────────────────────────────────────────────────────────────────
    if opts.diff_files {
        let source_dir = opts.repo_root.join("source");
        let ignore = IgnoreList::load(opts.repo_root, &template_ctx);
        let entries = scan(&source_dir, &ignore)?;
        let mut section_lines: Vec<String> = Vec::new();

        for entry in &entries {
            let dest_expanded = expand_tilde(&entry.dest_tilde)?;
            let dest = resolve_dest(dest_expanded, opts.dest_root);
            let label = &entry.dest_tilde;

            if entry.flags.extdir {
                if !dest.exists() {
                    section_lines.push(format!("  ? {}  (extdir: not cloned)", label));
                } else if let Ok(marker) = parse_extdir_marker(&entry.src) {
                    // Check ref drift: if a ref is pinned, compare against installed HEAD.
                    if let Some(ref pinned_ref) = marker.ref_name {
                        if let Some(head) = extdir_head_sha(&dest) {
                            let expected_sha = git_rev_parse_ref(&dest, pinned_ref);
                            let at_ref = expected_sha.map_or(false, |s| s == head);
                            if !at_ref {
                                let short = &head[..head.len().min(8)];
                                section_lines.push(format!(
                                    "  M {}  (extdir: at {}, expected {})",
                                    label, short, pinned_ref
                                ));
                            }
                        }
                        // If git is unavailable or dest isn't a git repo — skip silently.
                    }
                }
                continue;
            }

            if entry.flags.extfile {
                if !dest.exists() {
                    section_lines.push(format!("  ? {}  (extfile: not downloaded)", label));
                }
                // No content-hash tracking yet — presence check only.
                continue;
            }

            if entry.flags.symlink {
                let (kind, expected_target) = if entry.flags.template {
                    let src_text = std::fs::read_to_string(&entry.src);
                    match src_text.and_then(|t| {
                        crate::template::render(&t, &template_ctx)
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                    }) {
                        Err(e) => {
                            section_lines.push(format!(
                                "  ~ {}  (template render failed: {})",
                                label, e
                            ));
                            continue;
                        }
                        Ok(rendered) => {
                            let target = std::path::PathBuf::from(rendered.trim().to_string());
                            let kind = match check_drift_link_template(&entry.src, &template_ctx, &dest) {
                                Ok(k) => k,
                                Err(e) => {
                                    section_lines.push(format!(
                                        "  ~ {}  (template render failed: {})",
                                        label, e
                                    ));
                                    continue;
                                }
                            };
                            (kind, target.display().to_string())
                        }
                    }
                } else {
                    (check_drift_link(&entry.src, &dest), entry.src.display().to_string())
                };
                match kind {
                    DriftKind::Clean => {}
                    DriftKind::Missing => {
                        section_lines.push(format!("  ? {}", label));
                    }
                    DriftKind::SourceMissing => {
                        section_lines.push(format!("  ! {}", label));
                    }
                    DriftKind::Modified => {
                        let actual = if dest.is_symlink() {
                            std::fs::read_link(&dest)
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|_| "(unreadable)".to_string())
                        } else {
                            "(not a symlink)".to_string()
                        };
                        section_lines.push(format!(
                            "  M {}  (symlink: points to {}, expected {})",
                            label,
                            actual,
                            expected_target
                        ));
                    }
                }
                continue;
            }

            if entry.flags.template {
                let kind = match check_drift_template(&entry.src, &template_ctx, &dest) {
                    Ok(k) => k,
                    Err(e) => {
                        // Non-fatal: show the ~ marker and continue.
                        section_lines.push(format!(
                            "  ~ {}  (template render failed: {})",
                            label,
                            e
                        ));
                        continue;
                    }
                };
                match kind {
                    DriftKind::Clean => {}
                    DriftKind::Missing => {
                        section_lines.push(format!("  ? {}", label));
                    }
                    DriftKind::SourceMissing => {
                        section_lines.push(format!("  ! {}", label));
                    }
                    DriftKind::Modified => {
                        let src_text = std::fs::read_to_string(&entry.src)
                            .with_context(|| format!("Cannot read {}", entry.src.display()))?;
                        let rendered = crate::template::render(&src_text, &template_ctx)
                            .with_context(|| {
                                format!("Cannot render template {}", entry.src.display())
                            })?;
                        let dest_text = read_text_or_notice(&dest, label, &mut section_lines);
                        if let Some(dest_str) = dest_text {
                            push_diff_output(
                                &dest_str,
                                &rendered,
                                label,
                                label,                             // --- dest (current)
                                &format!("{} (source)", label),   // +++ source (desired)
                                opts.stat_only,
                                use_color,
                                &mut section_lines,
                            );
                        }
                    }
                }
                continue;
            }

            // Plain file.
            let kind = check_drift(&entry.src, &dest);
            match kind {
                DriftKind::Clean => {}
                DriftKind::Missing => {
                    section_lines.push(format!("  ? {}", label));
                }
                DriftKind::SourceMissing => {
                    section_lines.push(format!("  ! {}", label));
                }
                DriftKind::Modified => {
                    let src_bytes = std::fs::read(&entry.src)
                        .with_context(|| format!("Cannot read {}", entry.src.display()))?;
                    let dest_bytes = std::fs::read(&dest)
                        .with_context(|| format!("Cannot read {}", dest.display()))?;

                    if is_binary(&src_bytes) || is_binary(&dest_bytes) {
                        section_lines.push(format!("  M {}  (binary files differ)", label));
                    } else {
                        let src_text = String::from_utf8_lossy(&src_bytes).into_owned();
                        let dest_text = String::from_utf8_lossy(&dest_bytes).into_owned();
                        push_diff_output(
                            &dest_text,                        // a = current on disk
                            &src_text,                         // b = what apply would write
                            label,
                            label,                             // --- dest (current)
                            &format!("{} (source)", label),   // +++ source (desired)
                            opts.stat_only,
                            use_color,
                            &mut section_lines,
                        );
                    }
                }
            }
        }

        if !section_lines.is_empty() {
            any_drift = true;
            println!("[files]");
            for line in &section_lines {
                println!("{}", line);
            }
            println!();
        }
    }

    // ── Brew ──────────────────────────────────────────────────────────────────
    if opts.diff_brews {
        let modules_to_check: Vec<String> = match opts.module_filter {
            Some(m) => vec![m.to_string()],
            None => HavenConfig::load(opts.repo_root)?
                .resolve_modules(opts.profile)
                .unwrap_or_default(),
        };
        let sorted = sort_modules(&modules_to_check);
        let brewfile_paths = profile_brewfile_paths(opts, &sorted);
        let path_refs: Vec<&Path> = brewfile_paths.iter().map(|p| p.as_path()).collect();

        if path_refs.is_empty() {
            // No Brewfiles for this profile — nothing to report.
        } else {
            match crate::homebrew::brew_path() {
                None => {
                    println!("[brew] skipped (brew not available)\n");
                }
                Some(brew) => {
                    match crate::homebrew::brewfile_diff(&brew, &path_refs) {
                        Err(e) => {
                            eprintln!("warning: brew diff failed: {}", e);
                        }
                        Ok(diff) if diff.is_clean() => {}
                        Ok(diff) => {
                            any_drift = true;
                            println!("[brew]");
                            for name in &diff.missing_formulas {
                                println!("  + {}  (in Brewfile, not installed)", name);
                            }
                            for name in &diff.missing_casks {
                                println!("  + {}  (in Brewfile, not installed)", name);
                            }
                            for name in &diff.extra_formulas {
                                println!("  - {}  (installed, not in Brewfile)", name);
                            }
                            for name in &diff.extra_casks {
                                println!("  - {}  (installed, not in Brewfile)", name);
                            }
                            println!();
                        }
                    }
                }
            }
        }
    }

    // ── AI skills ─────────────────────────────────────────────────────────────
    if opts.diff_ai {
        if let Ok(Some(skills_cfg)) = SkillsConfig::load(opts.repo_root) {
            let lock = LockFile::load(opts.repo_root).unwrap_or_default();
            let cache = SkillCache::new(opts.state_dir);
            let mut section_lines: Vec<String> = Vec::new();

            for skill in &skills_cfg.skills {
                let source = match SkillSource::parse(&skill.source) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let gh = match source {
                    SkillSource::Repo => {
                        // repo: skills are embedded in the haven repo.
                        // Check for uncommitted changes in the files/ subdir.
                        let files_path = opts
                            .repo_root
                            .join("ai")
                            .join("skills")
                            .join(&skill.name)
                            .join("files");
                        if !files_path.exists() {
                            section_lines.push(format!(
                                "  ? {}  (repo: files not found)",
                                skill.name
                            ));
                        } else {
                            // Run git status --short on the files/ subdir.
                            // If git is unavailable or fails (e.g. jj-only repo), skip silently.
                            let rel_path = format!("ai/skills/{}/files", skill.name);
                            let dirty = std::process::Command::new("git")
                                .args([
                                    "-C",
                                    &opts.repo_root.to_string_lossy(),
                                    "status",
                                    "--short",
                                    "--",
                                    &rel_path,
                                ])
                                .output()
                                .ok()
                                .filter(|o| o.status.success())
                                .map(|o| !o.stdout.is_empty())
                                .unwrap_or(false);
                            if dirty {
                                section_lines.push(format!(
                                    "  ~ {}  (repo: uncommitted changes)",
                                    skill.name
                                ));
                            }
                        }
                        continue;
                    }
                    SkillSource::Dir(_) => continue, // no version tracking for local skills
                    SkillSource::Gh(ref gh) => gh,
                };
                let lock_key = gh.source_key();
                let lock_sha = match lock.skill_sha(&lock_key) {
                    Some(s) => s.to_string(),
                    None => continue, // never fetched from this lock — skip
                };

                let cache_path = cache.cache_path(gh);
                let installed_sha = read_cache_sha(&cache_path);

                match installed_sha {
                    None => {
                        section_lines.push(format!("  ? {}  (not installed)", skill.name));
                    }
                    Some(ref installed) if installed != &lock_sha => {
                        section_lines.push(format!(
                            "  ~ {}  (installed: {}, pinned: {})",
                            skill.name,
                            &installed[..installed.len().min(8)],
                            &lock_sha[..lock_sha.len().min(8)],
                        ));
                    }
                    Some(_) => {} // up to date
                }
            }

            if !section_lines.is_empty() {
                any_drift = true;
                println!("[ai]");
                for line in &section_lines {
                    println!("{}", line);
                }
                println!();
            }
        }
    }

    if !any_drift {
        println!("✓ Everything up to date (profile: {})", opts.profile);
    }

    Ok(any_drift)
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Rebase a dest path onto dest_root (same as in apply.rs / status.rs).
fn resolve_dest(dest: PathBuf, dest_root: &Path) -> PathBuf {
    if dest_root == Path::new("/") {
        dest
    } else {
        let rel = dest.strip_prefix("/").unwrap_or(&dest);
        dest_root.join(rel)
    }
}

/// Collect all Brewfile paths for the active profile (master + per-module).
fn profile_brewfile_paths(opts: &DiffOptions<'_>, sorted_modules: &[String]) -> Vec<PathBuf> {
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

/// Returns `true` if the byte slice looks like binary data (contains a null byte).
fn is_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0u8)
}

/// Try to read a file as UTF-8 text. On failure (binary or unreadable), push a notice
/// into `lines` and return `None`.
fn read_text_or_notice(
    path: &Path,
    label: &str,
    lines: &mut Vec<String>,
) -> Option<String> {
    match std::fs::read(path) {
        Err(e) => {
            lines.push(format!("  M {}  (cannot read destination: {})", label, e));
            None
        }
        Ok(bytes) if is_binary(&bytes) => {
            lines.push(format!("  M {}  (binary files differ)", label));
            None
        }
        Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
    }
}

// ─── AI skill helpers ─────────────────────────────────────────────────────────

/// Read the `.haven-sha` file from a skill cache directory.
/// Returns `None` if the directory doesn't exist or the file is unreadable.
fn read_cache_sha(cache_path: &Path) -> Option<String> {
    let sha_file = cache_path.join(".haven-sha");
    std::fs::read_to_string(&sha_file)
        .ok()
        .map(|s| s.trim().to_string())
}

// ─── extdir_ helpers ──────────────────────────────────────────────────────────

/// Minimal read of an `extdir_` marker file — only the `ref` field is needed.
#[derive(serde::Deserialize)]
struct ExtdirMarker {
    #[serde(rename = "ref")]
    ref_name: Option<String>,
}

fn parse_extdir_marker(src: &Path) -> Result<ExtdirMarker> {
    let text = std::fs::read_to_string(src)
        .with_context(|| format!("Cannot read extdir marker {}", src.display()))?;
    toml::from_str(&text)
        .with_context(|| format!("Invalid TOML in extdir marker {}", src.display()))
}

/// Return the full HEAD commit SHA of a git repo at `dest`, or `None` if git
/// is unavailable or `dest` is not a git repository.
fn extdir_head_sha(dest: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["-C", &dest.to_string_lossy(), "rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok().map(|s| s.trim().to_string())
}

/// Resolve a ref name (branch, tag, commit SHA) to its full SHA inside `dest`.
/// Returns `None` if git is unavailable or the ref cannot be resolved.
fn git_rev_parse_ref(dest: &Path, ref_name: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["-C", &dest.to_string_lossy(), "rev-parse", ref_name])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok().map(|s| s.trim().to_string())
}

/// Generate and push diff or stat output into `lines`.
///
/// Convention: `a` = dest (current on-disk state), `b` = src (what apply would write).
/// This means `-` lines show what would be removed and `+` lines what would be added,
/// matching `git diff` semantics.
#[allow(clippy::too_many_arguments)]
fn push_diff_output(
    dest_text: &str,   // a = old = current on disk   → shown as "-" lines
    src_text: &str,    // b = new = what apply writes → shown as "+" lines
    label: &str,
    label_a: &str,
    label_b: &str,
    stat_only: bool,
    use_color: bool,
    lines: &mut Vec<String>,
) {
    const CONTEXT: usize = 3;
    if let Some(diff) = unified_diff(dest_text, src_text, label_a, label_b, CONTEXT) {
        if stat_only {
            lines.push(format!("  {}", stat_line(label, &diff)));
        } else {
            // Indent each diff line by two spaces for visual alignment.
            let formatted = if use_color {
                colorize_diff(&diff)
            } else {
                diff
            };
            for line in formatted.lines() {
                lines.push(format!("  {}", line));
            }
        }
    }
}
