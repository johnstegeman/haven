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

use crate::chezmoi::{self, ChezmoiBrewfileEntry, ChezmoiEntry, ChezmoiExternalEntry, ChezmoiScriptEntry, ScriptMigration, ScriptWhen, SkippedEntry};
use crate::config::dfiles::DfilesConfig;
use crate::config::module::{expand_tilde, HomebrewConfig, MiseConfig, ModuleConfig};
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

    let (keeps, externals, skips, scripts, brewfiles) = chezmoi::scan(&source_dir, opts.include_ignored_files)?;

    if opts.dry_run {
        print_dry_run_plan(&source_dir, &keeps, &externals, &skips, &scripts, &brewfiles);
        return Ok(());
    }

    if keeps.is_empty() && externals.is_empty() && scripts.is_empty() && brewfiles.is_empty() {
        println!("Nothing to import.");
        print_skip_table(&skips);
        return Ok(());
    }

    execute(opts, &source_dir, &keeps, &externals, &skips, &scripts, &brewfiles)
}

// ─── Dry-run output ───────────────────────────────────────────────────────────

fn print_dry_run_plan(chezmoi_source_dir: &std::path::Path, keeps: &[ChezmoiEntry], externals: &[ChezmoiExternalEntry], skips: &[SkippedEntry], scripts: &[ChezmoiScriptEntry], brewfiles: &[ChezmoiBrewfileEntry]) {
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
    // Show script imports in dry-run.
    if !scripts.is_empty() {
        println!("Would import {} script(s) to source/scripts/:", scripts.len());
        println!();
        for s in scripts {
            let when_label = match s.when { ScriptWhen::Once => "run_once", ScriptWhen::Always => "run_always" };
            let migration_label = match &s.migration {
                ScriptMigration::BrewBundle { brewfile_path } => format!(" + emit [homebrew] brewfile = {:?}", brewfile_path),
                ScriptMigration::MiseInstall => " + emit [mise]".to_string(),
                ScriptMigration::Unrecognized => " (no pattern detected — copy only)".to_string(),
            };
            println!(
                "  [{:10}]  {}  →  source/scripts/{}{}",
                when_label, s.chezmoi_path.display(),
                s.chezmoi_path.file_name().unwrap_or(s.chezmoi_path.as_os_str()).to_string_lossy(),
                migration_label,
            );
        }
        println!();
    }

    // Show Brewfile imports in dry-run.
    if !brewfiles.is_empty() {
        println!("Would import {} Brewfile(s) to brew/:", brewfiles.len());
        println!();
        for b in brewfiles {
            println!(
                "  {}  →  {}  (module: {})",
                b.dest_tilde, b.brew_dest, b.module_name,
            );
        }
        println!();
    }

    // Show script-referenced Brewfiles in dry-run.
    let tracked_tilde: std::collections::HashSet<&str> =
        brewfiles.iter().map(|b| b.dest_tilde.as_str()).collect();
    for script in scripts {
        if let ScriptMigration::BrewBundle { brewfile_path } = &script.migration {
            let normalized = normalize_to_tilde(brewfile_path);
            if !tracked_tilde.contains(normalized.as_str()) {
                let filename = std::path::Path::new(brewfile_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Brewfile");
                let brew_dest = crate::chezmoi::brewfile_brew_dest(filename);
                let resolved = try_expand_tilde(brewfile_path);
                if resolved.map_or(false, |p| p.exists()) {
                    println!(
                        "  Would copy {} → {}  (referenced in script: {})",
                        brewfile_path, brew_dest, script.chezmoi_path.display(),
                    );
                } else {
                    println!(
                        "  warning: {} → {} not found on disk  (referenced in script: {})",
                        brewfile_path, brew_dest, script.chezmoi_path.display(),
                    );
                }
            }
        }
    }

    // Show ignore file import in dry-run.
    let ignore_src = chezmoi_source_dir.join(".chezmoiignore");
    if ignore_src.exists() {
        println!("Would import: .chezmoiignore  →  config/ignore");
    }

    // Show data file import in dry-run.
    if let Ok(data_vars) = chezmoi::scan_data_file(chezmoi_source_dir) {
        if !data_vars.is_empty() {
            let mut keys: Vec<&String> = data_vars.keys().collect();
            keys.sort();
            println!("Would add {} [data] variable(s) to dfiles.toml:", data_vars.len());
            for k in keys {
                println!("  data.{} = {:?}", k, data_vars[k]);
            }
        }
    }

    print_skip_table(skips);
}

// ─── Real run ────────────────────────────────────────────────────────────────

fn execute(opts: &ImportOptions<'_>, source_dir: &std::path::Path, keeps: &[ChezmoiEntry], externals: &[ChezmoiExternalEntry], skips: &[SkippedEntry], scripts: &[ChezmoiScriptEntry], brewfiles: &[ChezmoiBrewfileEntry]) -> Result<()> {
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

    // ── Copy scripts to source/scripts/ for execution on apply ───────────────
    import_scripts(source_dir, opts.repo_root, scripts)?;

    // ── Import Brewfiles to brew/ and emit [homebrew] module TOML ─────────────
    import_brewfiles(opts, source_dir, brewfiles, scripts)?;

    // ── Emit module TOML for non-brew script migrations (mise) ────────────────
    emit_script_migrations(opts.repo_root, scripts)?;

    // ── Import .chezmoiignore → config/ignore ─────────────────────────────────
    let ignore_warnings = import_chezmoiignore(source_dir, opts.repo_root)?;
    for w in &ignore_warnings {
        eprintln!("    warning (ignore): {}", w);
    }

    // ── Write dfiles.toml if not already present ──────────────────────────────
    let dfiles_toml = opts.repo_root.join("dfiles.toml");
    if !dfiles_toml.exists() {
        DfilesConfig::write_scaffold(opts.repo_root)?;
        println!("  ✓  wrote dfiles.toml  (edit profiles to customise)");
    }

    // ── Import .chezmoidata.yaml / .chezmoidata.toml → [data] in dfiles.toml ──
    let data_vars = chezmoi::scan_data_file(&source_dir)?;
    if !data_vars.is_empty() {
        import_data_vars(opts.repo_root, &data_vars)?;
    }

    println!();
    println!(
        "Imported {} file(s), {} external(s), {} Brewfile(s), {} script migration(s). Skipped {} item(s).",
        keeps.len(),
        externals.len(),
        brewfiles.len(),
        scripts.len(),
        skips.iter().filter(|s| s.reason.display().is_some()).count(),
    );
    if !data_vars.is_empty() {
        println!("  [data] {} custom variable(s) added to dfiles.toml", data_vars.len());
    }
    println!("Run `dfiles apply` to deploy.");

    print_skip_table(skips);

    Ok(())
}

/// Write module TOML entries for mise script migrations.
///
/// BrewBundle migrations are handled by `import_brewfiles()` — this function
/// only emits `[mise]` sections for `mise install` scripts.
///
/// Migrations are merged into the "packages" module.
/// Existing TOML files are loaded and updated in-place (additive, never overwrites).
fn emit_script_migrations(repo_root: &Path, scripts: &[ChezmoiScriptEntry]) -> Result<()> {
    let mise_scripts: Vec<_> = scripts
        .iter()
        .filter(|s| matches!(&s.migration, ScriptMigration::MiseInstall))
        .collect();

    if mise_scripts.is_empty() {
        return Ok(());
    }

    let module_name = "packages";
    let mut module = ModuleConfig::load(repo_root, module_name)?;
    let mut changed = false;

    for script in &mise_scripts {
        if module.mise.is_none() {
            module.mise = Some(MiseConfig { config: None });
            println!(
                "  ✓  {} → modules/{}.toml  ([mise])",
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

    if changed {
        module.save(repo_root, module_name)?;
    }

    Ok(())
}

/// Copy all detected scripts into `source/scripts/` so they can be executed
/// by `dfiles apply --run-scripts`.
///
/// Each script is stored under its original filename. Existing files are skipped
/// (idempotent — same behaviour as regular source file import).
fn import_scripts(chezmoi_source_dir: &std::path::Path, repo_root: &Path, scripts: &[ChezmoiScriptEntry]) -> Result<()> {
    if scripts.is_empty() {
        return Ok(());
    }

    let scripts_dest = repo_root.join("source").join("scripts");
    std::fs::create_dir_all(&scripts_dest)?;

    for script in scripts {
        let filename = script.chezmoi_path
            .file_name()
            .expect("script path always has a filename");
        let dest = scripts_dest.join(filename);

        if dest.exists() {
            println!(
                "  ~ {}  (source/scripts/{} already exists — skipped)",
                script.chezmoi_path.display(),
                filename.to_string_lossy(),
            );
            continue;
        }

        let src = chezmoi_source_dir.join(&script.chezmoi_path);
        std::fs::copy(&src, &dest).with_context(|| {
            format!(
                "Cannot copy {} → source/scripts/{}",
                src.display(),
                filename.to_string_lossy(),
            )
        })?;

        let when_label = match script.when { ScriptWhen::Once => "run_once", ScriptWhen::Always => "run_always" };
        println!(
            "  ✓  [{}]  {}  →  source/scripts/{}",
            when_label,
            script.chezmoi_path.display(),
            filename.to_string_lossy(),
        );
    }

    Ok(())
}

/// Copy Brewfiles to `brew/` and emit `[homebrew]` module TOML entries.
///
/// Handles two sources:
/// 1. Chezmoi-tracked: copy from chezmoi source dir to `brew/`.
/// 2. Script-referenced: `brew bundle --file=<path>` in a `run_once_` script —
///    cross-correlate with tracked entries, then try to resolve on disk.
fn import_brewfiles(
    opts: &ImportOptions<'_>,
    source_dir: &Path,
    brewfiles: &[ChezmoiBrewfileEntry],
    scripts: &[ChezmoiScriptEntry],
) -> Result<()> {
    let brew_dir = opts.repo_root.join("brew");

    // Track which dest_tilde paths we've handled (for cross-correlation).
    let mut handled_dest_tilde: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    // ── Step 1: Copy chezmoi-tracked Brewfiles ─────────────────────────────────
    for entry in brewfiles {
        let brew_filename = std::path::Path::new(&entry.brew_dest)
            .file_name()
            .expect("brew_dest always has a filename");
        let dest = brew_dir.join(brew_filename);

        if dest.exists() {
            println!(
                "  ~ {}  ({} already exists — skipped)",
                entry.dest_tilde, entry.brew_dest,
            );
            handled_dest_tilde.insert(entry.dest_tilde.clone());
            continue;
        }

        std::fs::create_dir_all(&brew_dir)
            .context("Cannot create brew/ directory")?;

        let src = source_dir.join(&entry.chezmoi_path);
        std::fs::copy(&src, &dest).with_context(|| {
            format!("Cannot copy {} → {}", src.display(), dest.display())
        })?;

        println!("  ✓  {}  →  {}", entry.dest_tilde, entry.brew_dest);

        // Emit module TOML.
        emit_brewfile_module_toml(opts.repo_root, &entry.brew_dest, &entry.module_name)?;

        handled_dest_tilde.insert(entry.dest_tilde.clone());
    }

    // ── Step 2: Script-referenced Brewfiles (Case C) ───────────────────────────
    for script in scripts {
        let ScriptMigration::BrewBundle { brewfile_path } = &script.migration else {
            continue;
        };

        // Normalize path to tilde form for deduplication.
        let normalized_tilde = normalize_to_tilde(brewfile_path);

        // Skip if already covered by a chezmoi-tracked entry.
        if handled_dest_tilde.contains(&normalized_tilde) {
            continue;
        }

        let filename = std::path::Path::new(brewfile_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Brewfile");
        let brew_dest = crate::chezmoi::brewfile_brew_dest(filename);
        let module_name = crate::chezmoi::brewfile_module_name(filename);
        let brew_filename = std::path::Path::new(&brew_dest)
            .file_name()
            .expect("brew_dest always has a filename");
        let dest = brew_dir.join(brew_filename);

        // Try to resolve the path on disk.
        match try_expand_tilde(brewfile_path).filter(|p| p.exists()) {
            Some(src) => {
                // Found on disk — copy to brew/.
                std::fs::create_dir_all(&brew_dir)
                    .context("Cannot create brew/ directory")?;
                std::fs::copy(&src, &dest).with_context(|| {
                    format!("Cannot copy {} → {}", src.display(), dest.display())
                })?;

                println!(
                    "  ✓  {}  →  {}  (from script: {})",
                    brewfile_path, brew_dest,
                    script.chezmoi_path.display(),
                );

                emit_brewfile_module_toml(opts.repo_root, &brew_dest, &module_name)?;

                // Remind user to stage the new file.
                println!(
                    "  hint: Don't forget to stage this file:\n        git add {}   (or: jj file track {})",
                    brew_dest, brew_dest,
                );

                handled_dest_tilde.insert(normalized_tilde);
            }
            None => {
                // Cannot resolve — warn with manual copy instructions.
                eprintln!(
                    "  warning: script '{}' references Brewfile at '{}' which was not found on disk.",
                    script.chezmoi_path.display(),
                    brewfile_path,
                );
                eprintln!(
                    "    Copy it manually:  mkdir -p {bdir}  &&  cp {path} {bdir}/{fname}",
                    bdir = brew_dir.display(),
                    path = brewfile_path,
                    fname = brew_filename.to_string_lossy(),
                );
                eprintln!(
                    "    Then add to modules/{}.toml:\n      [homebrew]\n      brewfile = {:?}",
                    module_name, brew_dest,
                );
            }
        }
    }

    Ok(())
}

/// Emit (or skip if already set) a `[homebrew] brewfile = <path>` entry in a module TOML.
fn emit_brewfile_module_toml(repo_root: &Path, brew_dest: &str, module_name: &str) -> Result<()> {
    let mut module = ModuleConfig::load(repo_root, module_name)?;
    if module.homebrew.is_none() {
        module.homebrew = Some(HomebrewConfig { brewfile: brew_dest.to_string() });
        module.save(repo_root, module_name)?;
        println!(
            "  ✓  → modules/{}.toml  ([homebrew] brewfile = {:?})",
            module_name, brew_dest,
        );
    } else {
        println!(
            "  ~ [homebrew] already set in modules/{}.toml — skipped",
            module_name,
        );
    }
    Ok(())
}

/// Expand a path that may start with `~` to a `PathBuf`, returning `None` on failure.
fn try_expand_tilde(path: &str) -> Option<std::path::PathBuf> {
    expand_tilde(path).ok()
}

/// Normalize a path (possibly with `~`) to a tilde-prefixed string for comparison.
fn normalize_to_tilde(path: &str) -> String {
    if let Ok(expanded) = expand_tilde(path) {
        crate::fs::tilde_path(&expanded)
    } else {
        path.to_string()
    }
}

/// Read `.chezmoiignore` from `chezmoi_source_dir`, convert Go template syntax
/// to Tera, and write the result to `<repo_root>/config/ignore`.
///
/// The resulting `config/ignore` file is a Tera template — it is rendered at
/// runtime against the current machine context whenever `dfiles` loads it.
/// This preserves conditional ignore patterns (e.g. OS-specific patterns).
///
/// Returns a list of warnings for any expressions that could not be converted.
/// Does nothing (returns empty warnings) if `.chezmoiignore` does not exist.
fn import_chezmoiignore(chezmoi_source_dir: &std::path::Path, repo_root: &std::path::Path) -> Result<Vec<String>> {
    let src = chezmoi_source_dir.join(".chezmoiignore");
    if !src.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&src)
        .with_context(|| format!("Cannot read {}", src.display()))?;

    let dest = repo_root.join("config").join("ignore");
    if dest.exists() {
        println!("  ~ config/ignore already exists — skipped");
        return Ok(Vec::new());
    }

    let (tera_content, warnings) = crate::chezmoi::convert_chezmoiignore_to_tera(&content);

    // Count non-blank, non-comment, non-directive lines as "patterns".
    let pattern_count = tera_content.lines().filter(|l| {
        let t = l.trim();
        !t.is_empty() && !t.starts_with('#') && !t.starts_with("{%")
    }).count();

    if !tera_content.trim().is_empty() {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Cannot create {}", parent.display()))?;
        }
        let mut file_content = tera_content;
        if !file_content.ends_with('\n') {
            file_content.push('\n');
        }
        std::fs::write(&dest, &file_content)
            .with_context(|| format!("Cannot write {}", dest.display()))?;

        println!("  ✓  .chezmoiignore  →  config/ignore  ({} pattern(s))", pattern_count);
    }

    Ok(warnings)
}

/// Write custom data variables into the `[data]` section of `dfiles.toml` using
/// `toml_edit` so existing formatting and comments are preserved.
///
/// Keys that already exist are skipped (idempotent).
fn import_data_vars(
    repo_root: &Path,
    data_vars: &std::collections::HashMap<String, String>,
) -> Result<()> {
    let path = repo_root.join("dfiles.toml");
    let text = if path.exists() {
        std::fs::read_to_string(&path)
            .with_context(|| format!("Cannot read {}", path.display()))?
    } else {
        String::new()
    };

    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .context("dfiles.toml contains invalid TOML")?;

    let mut added = 0usize;
    let mut skipped = 0usize;

    let mut keys: Vec<&String> = data_vars.keys().collect();
    keys.sort();

    for key in keys {
        let val = &data_vars[key];
        // Skip keys that already exist in [data].
        if doc["data"][key.as_str()].is_str() {
            skipped += 1;
            continue;
        }
        doc["data"][key.as_str()] = toml_edit::value(val.clone());
        added += 1;
    }

    if added > 0 {
        std::fs::write(&path, doc.to_string())
            .with_context(|| format!("Cannot write {}", path.display()))?;
        println!(
            "  ✓  .chezmoidata  →  [data] in dfiles.toml  ({} variable(s) added{})",
            added,
            if skipped > 0 { format!(", {} already present", skipped) } else { String::new() },
        );
    } else if skipped > 0 {
        println!(
            "  ~ [data] variables already present in dfiles.toml — skipped ({} key(s))",
            skipped,
        );
    }

    Ok(())
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

