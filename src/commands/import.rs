/// `dfiles import --from chezmoi` — migrate a chezmoi source dir into dfiles format.
///
/// Pipeline:
///
///   detect_source_dir()
///         │
///         ▼
///   scan() → (keeps, externals, skips)
///         │
///   ┌─────┴──────┐
///   │ dry_run?   │
///   │  yes → print plan + skip table, exit
///   │  no  → copy source files (encoded paths), write external module TOMLs
///   └────────────┘
///
/// Files are written to `source/` using the chezmoi-compatible magic-name encoding
/// (same path as in the chezmoi source dir). No [[files]] TOML entries are written —
/// the encoded filename is the single source of truth for dest, flags, and permissions.
use anyhow::{Context, Result};
use std::path::Path;

use crate::chezmoi::{self, ChezmoiEntry, ChezmoiExternalEntry, ChezmoiScriptEntry, ScriptMigration, ScriptWhen, SkippedEntry};
use crate::config::module::{HomebrewConfig, MiseConfig, ModuleConfig};
use crate::fs::copy_to_dest;
use crate::source::extdir_source_path;

pub struct ImportOptions<'a> {
    pub repo_root: &'a Path,
    /// Override for the chezmoi source directory (--source flag).
    pub source_dir: Option<&'a Path>,
    pub dry_run: bool,
    /// When true, import files that match `.chezmoiignore` patterns instead of skipping them.
    /// The ignore patterns are still written to `config/ignore`, so `dfiles apply/status/diff`
    /// will continue to exclude those files.
    pub include_ignored_files: bool,
}

pub fn run(opts: &ImportOptions<'_>) -> Result<()> {
    // Guard: refuse to import into a non-empty repo.
    if !opts.dry_run {
        let repo_source = opts.repo_root.join("source");
        if repo_source.exists() {
            let has_files = std::fs::read_dir(&repo_source)
                .with_context(|| format!("Cannot read {}", repo_source.display()))?
                .next()
                .is_some();
            if has_files {
                anyhow::bail!(
                    "{} is not empty.\n\
                     \n\
                     Import rewrites source/ from scratch. \
                     To re-import, remove source/ first:\n\
                     \n\
                     \trm -rf {}/source\n\
                     \n\
                     Use --dry-run to preview what would be imported without writing anything.",
                    repo_source.display(),
                    opts.repo_root.display(),
                );
            }
        }
    }

    let source_dir = chezmoi::detect_source_dir(opts.source_dir)
        .context("Cannot locate chezmoi source directory")?;

    println!("Chezmoi source: {}", source_dir.display());
    println!();

    let (keeps, externals, skips, scripts) = chezmoi::scan(&source_dir, opts.include_ignored_files)?;

    if opts.dry_run {
        print_dry_run_plan(&source_dir, &keeps, &externals, &skips, &scripts);
        return Ok(());
    }

    if keeps.is_empty() && externals.is_empty() && scripts.is_empty() {
        println!("Nothing to import.");
        print_skip_table(&skips);
        return Ok(());
    }

    execute(opts, &source_dir, &keeps, &externals, &skips, &scripts)
}

// ─── Dry-run output ───────────────────────────────────────────────────────────

fn print_dry_run_plan(chezmoi_source_dir: &std::path::Path, keeps: &[ChezmoiEntry], externals: &[ChezmoiExternalEntry], skips: &[SkippedEntry], scripts: &[ChezmoiScriptEntry]) {
    if keeps.is_empty() && externals.is_empty() {
        println!("Would import 0 files.");
    } else {
        if !keeps.is_empty() {
            println!("Would import {} file(s):", keeps.len());
            println!();
            for e in keeps {
                let mut flags: Vec<&str> = Vec::new();
                if e.private { flags.push("private"); }
                if e.executable { flags.push("executable"); }
                if e.link { flags.push("link"); }
                if e.template { flags.push("template"); }
                let annotation = if flags.is_empty() {
                    String::new()
                } else {
                    format!("  ({})", flags.join(", "))
                };
                println!(
                    "  {:40}  →  source/{:30}  dest: {}{}",
                    e.chezmoi_path.display(),
                    e.source_name,
                    e.dest_tilde,
                    annotation,
                );
            }
            println!();
        }
        if !externals.is_empty() {
            println!("Would import {} external(s):", externals.len());
            println!();
            for e in externals {
                let ref_label = e.ref_name.as_deref().unwrap_or("default branch");
                println!(
                    "  [{:6}]  {}  ({}  {})",
                    e.module, e.dest_tilde, e.url, ref_label,
                );
            }
            println!();
        }
    }
    // Show script migrations in dry-run.
    if !scripts.is_empty() {
        println!("Would emit {} script migration(s):", scripts.len());
        println!();
        for s in scripts {
            let when_label = match s.when { ScriptWhen::Once => "run_once", ScriptWhen::Always => "run_always" };
            let migration_label = match &s.migration {
                ScriptMigration::BrewBundle { brewfile_path } => format!("[homebrew] brewfile = {:?}", brewfile_path),
                ScriptMigration::MiseInstall => "[mise]".to_string(),
                ScriptMigration::Unrecognized => unreachable!("Unrecognized filtered before here"),
            };
            println!("  [{:10}]  {:50}  →  {}", when_label, s.chezmoi_path.display(), migration_label);
        }
        println!();
    }

    // Show ignore file import in dry-run.
    let ignore_src = chezmoi_source_dir.join(".chezmoiignore");
    if ignore_src.exists() {
        println!("Would import: .chezmoiignore  →  config/ignore");
    }
    print_skip_table(skips);
}

// ─── Real run ────────────────────────────────────────────────────────────────

fn execute(opts: &ImportOptions<'_>, source_dir: &std::path::Path, keeps: &[ChezmoiEntry], externals: &[ChezmoiExternalEntry], skips: &[SkippedEntry], scripts: &[ChezmoiScriptEntry]) -> Result<()> {
    let repo_source = opts.repo_root.join("source");
    std::fs::create_dir_all(&repo_source)?;

    // ── Copy source files using the encoded (chezmoi-compatible) path ─────────
    for entry in keeps {
        let dest = repo_source.join(&entry.source_name);

        if dest.exists() {
            println!(
                "  ~ {}  (source/{} already exists — skipped)",
                entry.chezmoi_path.display(),
                entry.source_name,
            );
            continue;
        }

        // Create intermediate directories.
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        if entry.template {
            // Write the converted Tera template text instead of copying the raw file.
            let converted = entry.converted_content.as_deref().unwrap_or("");
            std::fs::write(&dest, converted).with_context(|| {
                format!(
                    "Cannot write converted template to source/{}",
                    entry.source_name,
                )
            })?;
        } else {
            // For symlink_ entries, copy_from points to the resolved target file;
            // for regular entries, copy from the chezmoi source path.
            let src = entry.copy_from.clone()
                .unwrap_or_else(|| source_dir.join(&entry.chezmoi_path));
            copy_to_dest(&src, &dest).with_context(|| {
                format!(
                    "Cannot import {} → source/{}",
                    entry.chezmoi_path.display(),
                    entry.source_name,
                )
            })?;
        }

        let mut flags: Vec<&str> = Vec::new();
        if entry.private { flags.push("private"); }
        if entry.executable { flags.push("executable"); }
        if entry.link { flags.push("link"); }
        if entry.template { flags.push("template"); }
        let annotation = if flags.is_empty() {
            String::new()
        } else {
            format!("  ({})", flags.join(", "))
        };
        println!(
            "  ✓  {}  →  source/{}  dest: {}{}",
            entry.chezmoi_path.display(),
            entry.source_name,
            entry.dest_tilde,
            annotation,
        );

        // Emit any partial-conversion warnings.
        for w in &entry.template_warnings {
            eprintln!(
                "    warning (template): {}  [{}]",
                w,
                entry.source_name,
            );
        }
    }

    // ── Write extdir_ marker files into source/ ───────────────────────────────
    for entry in externals {
        let extdir_path = extdir_source_path(&repo_source, &entry.dest_tilde);

        if extdir_path.exists() {
            println!(
                "  ~ {}  (extdir marker already exists — skipped)",
                entry.dest_tilde,
            );
            continue;
        }

        if let Some(parent) = extdir_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Cannot create parent dir {}", parent.display())
            })?;
        }

        let mut content = format!("type = {:?}\nurl  = {:?}\n", entry.kind, entry.url);
        if let Some(ref_name) = &entry.ref_name {
            content.push_str(&format!("ref  = {:?}\n", ref_name));
        }
        std::fs::write(&extdir_path, &content)
            .with_context(|| format!("Cannot write extdir marker {}", extdir_path.display()))?;

        let ref_label = entry.ref_name.as_deref().unwrap_or("default branch");
        println!(
            "  ✓  {}  →  source/{}  ({}  {})",
            entry.dest_tilde,
            extdir_path
                .strip_prefix(&repo_source)
                .unwrap_or(&extdir_path)
                .display(),
            entry.url,
            ref_label,
        );
    }

    // ── Emit module TOML for detected script migrations ───────────────────────
    emit_script_migrations(opts.repo_root, scripts)?;

    // ── Import .chezmoiignore → config/ignore ─────────────────────────────────
    let ignore_warnings = import_chezmoiignore(source_dir, opts.repo_root)?;
    for w in &ignore_warnings {
        eprintln!("    warning (ignore): {}", w);
    }

    println!();
    println!(
        "Imported {} file(s), {} external(s), {} script migration(s). Skipped {} item(s).",
        keeps.len(),
        externals.len(),
        scripts.len(),
        skips.iter().filter(|s| s.reason.display().is_some()).count(),
    );
    println!("Run `dfiles apply` to deploy.");

    print_skip_table(skips);

    Ok(())
}

/// Write module TOML entries for each recognised script migration.
///
/// Each migration is written into `config/modules/<module>.toml`:
/// - `BrewBundle` → `[homebrew]\nbrewfile = "<path>"`
/// - `MiseInstall` → `[mise]`
///
/// Migrations are merged into the "packages" module by default.
/// Existing TOML files are loaded and updated in-place (additive, never overwrites).
fn emit_script_migrations(repo_root: &Path, scripts: &[ChezmoiScriptEntry]) -> Result<()> {
    if scripts.is_empty() {
        return Ok(());
    }

    // Collect all changes for the "packages" module (brew/mise always go here).
    let module_name = "packages";
    let mut module = ModuleConfig::load(repo_root, module_name)?;
    let mut changed = false;

    for script in scripts {
        match &script.migration {
            ScriptMigration::BrewBundle { brewfile_path } => {
                if module.homebrew.is_none() {
                    // Use a normalised path: if the Brewfile path looks absolute or uses ~,
                    // use it as-is; otherwise store it relative to the repo root.
                    let stored_path = normalise_brewfile_path(brewfile_path);
                    module.homebrew = Some(HomebrewConfig { brewfile: stored_path.clone() });
                    println!(
                        "  ✓  {} → config/modules/{}.toml  ([homebrew] brewfile = {:?})",
                        script.chezmoi_path.display(), module_name, stored_path,
                    );
                    changed = true;
                } else {
                    println!(
                        "  ~ {} → [homebrew] already set in {} — skipped",
                        script.chezmoi_path.display(), module_name,
                    );
                }
            }
            ScriptMigration::MiseInstall => {
                if module.mise.is_none() {
                    module.mise = Some(MiseConfig { config: None });
                    println!(
                        "  ✓  {} → config/modules/{}.toml  ([mise])",
                        script.chezmoi_path.display(), module_name,
                    );
                    changed = true;
                } else {
                    println!(
                        "  ~ {} → [mise] already set in {} — skipped",
                        script.chezmoi_path.display(), module_name,
                    );
                }
            }
            ScriptMigration::Unrecognized => unreachable!("Unrecognized filtered before here"),
        }
    }

    if changed {
        module.save(repo_root, module_name)?;
    }

    Ok(())
}

/// Normalise a Brewfile path for storage in module TOML.
///
/// If the path contains `~` or an absolute path, keep it as-is so the user
/// can adjust it. Otherwise, default to `"brew/Brewfile.packages"`.
fn normalise_brewfile_path(path: &str) -> String {
    if path == "Brewfile" {
        // Plain `brew bundle` with no --file arg — use the dfiles convention.
        "brew/Brewfile.packages".to_string()
    } else {
        path.to_string()
    }
}

/// Read `.chezmoiignore` from `chezmoi_source_dir`, strip Go template directives,
/// and write the resulting plain patterns to `<repo_root>/config/ignore`.
///
/// Returns a list of warnings for any lines that were skipped due to template syntax.
/// Does nothing (returns empty warnings) if `.chezmoiignore` does not exist.
fn import_chezmoiignore(chezmoi_source_dir: &std::path::Path, repo_root: &std::path::Path) -> Result<Vec<String>> {
    let src = chezmoi_source_dir.join(".chezmoiignore");
    if !src.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&src)
        .with_context(|| format!("Cannot read {}", src.display()))?;

    let mut out_lines: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        // Blank lines and comments pass through unchanged.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            out_lines.push(line.to_string());
            continue;
        }
        // Lines containing Go template directives ({{ ... }}) are skipped with a warning.
        if trimmed.contains("{{") {
            warnings.push(format!(
                "skipped Go template line in .chezmoiignore (not supported): {:?}",
                trimmed,
            ));
            continue;
        }
        out_lines.push(line.to_string());
    }

    // Only write the file if there are actual patterns (non-blank, non-comment lines).
    let has_patterns = out_lines.iter().any(|l| {
        let t = l.trim();
        !t.is_empty() && !t.starts_with('#')
    });

    let dest = repo_root.join("config").join("ignore");
    if dest.exists() {
        println!("  ~ config/ignore already exists — skipped");
        return Ok(warnings);
    }

    if has_patterns || !out_lines.is_empty() {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Cannot create {}", parent.display()))?;
        }
        let mut file_content = out_lines.join("\n");
        if !file_content.ends_with('\n') {
            file_content.push('\n');
        }
        std::fs::write(&dest, &file_content)
            .with_context(|| format!("Cannot write {}", dest.display()))?;

        let pattern_count = out_lines.iter().filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#')
        }).count();
        println!("  ✓  .chezmoiignore  →  config/ignore  ({} pattern(s))", pattern_count);
    }

    Ok(warnings)
}

// ─── Shared output helpers ────────────────────────────────────────────────────

fn print_skip_table(skips: &[SkippedEntry]) {
    let visible: Vec<&SkippedEntry> = skips
        .iter()
        .filter(|s| s.reason.display().is_some())
        .collect();

    if visible.is_empty() {
        return;
    }

    println!(
        "Skipped {} item(s) (not supported in v1 — see TODOS.md for P1 follow-ons):",
        visible.len()
    );
    for s in visible {
        println!(
            "  {:<45}  {}",
            s.chezmoi_path.display(),
            s.reason.display().unwrap_or(""),
        );
    }
    println!();
}

