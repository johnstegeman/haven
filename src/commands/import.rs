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

use crate::chezmoi::{self, ChezmoiEntry, ChezmoiExternalEntry, SkippedEntry};
use crate::config::module::{ExternalEntry, ModuleConfig};
use crate::fs::copy_to_dest;

pub struct ImportOptions<'a> {
    pub repo_root: &'a Path,
    /// Override for the chezmoi source directory (--source flag).
    pub source_dir: Option<&'a Path>,
    pub dry_run: bool,
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

    let (keeps, externals, skips) = chezmoi::scan(&source_dir)?;

    if opts.dry_run {
        print_dry_run_plan(&keeps, &externals, &skips);
        return Ok(());
    }

    if keeps.is_empty() && externals.is_empty() {
        println!("Nothing to import.");
        print_skip_table(&skips);
        return Ok(());
    }

    execute(opts, &keeps, &externals, &skips)
}

// ─── Dry-run output ───────────────────────────────────────────────────────────

fn print_dry_run_plan(keeps: &[ChezmoiEntry], externals: &[ChezmoiExternalEntry], skips: &[SkippedEntry]) {
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
    print_skip_table(skips);
}

// ─── Real run ────────────────────────────────────────────────────────────────

fn execute(opts: &ImportOptions<'_>, keeps: &[ChezmoiEntry], externals: &[ChezmoiExternalEntry], skips: &[SkippedEntry]) -> Result<()> {
    let source_dir = chezmoi::detect_source_dir(opts.source_dir)?;
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

    // ── Write external entries to module TOMLs ────────────────────────────────
    for entry in externals {
        let mut config = ModuleConfig::load(opts.repo_root, &entry.module)
            .with_context(|| format!("Cannot load module config '{}'", entry.module))?;

        if config.contains_external(&entry.dest_tilde) {
            println!(
                "  ~ [{}]  {}  (external already tracked — skipped)",
                entry.module, entry.dest_tilde,
            );
            continue;
        }

        config.externals.push(ExternalEntry {
            dest: entry.dest_tilde.clone(),
            kind: entry.kind.clone(),
            url: entry.url.clone(),
            ref_name: entry.ref_name.clone(),
        });

        config
            .save(opts.repo_root, &entry.module)
            .with_context(|| format!("Cannot write config/modules/{}.toml", entry.module))?;

        let ref_label = entry.ref_name.as_deref().unwrap_or("default branch");
        println!(
            "  ✓ [{}]  {}  →  {}  ({})",
            entry.module, entry.dest_tilde, entry.url, ref_label,
        );
    }

    println!();
    println!(
        "Imported {} file(s) and {} external(s). Skipped {} item(s).",
        keeps.len(),
        externals.len(),
        skips.iter().filter(|s| s.reason.display().is_some()).count(),
    );
    println!("Run `dfiles apply` to deploy.");

    // Profile hint: warn if any new module (from externals) isn't in dfiles.toml.
    let mut ext_modules: Vec<String> = externals.iter().map(|e| e.module.clone()).collect();
    ext_modules.sort();
    ext_modules.dedup();
    print_profile_hint(opts.repo_root, &ext_modules);

    print_skip_table(skips);

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

fn print_profile_hint(repo_root: &Path, new_modules: &[String]) {
    let dfiles_toml = repo_root.join("dfiles.toml");
    let contents = match std::fs::read_to_string(&dfiles_toml) {
        Ok(c) => c,
        Err(_) => return,
    };
    for module in new_modules {
        if !contents.contains(module.as_str()) {
            println!(
                "Hint: add '{}' to your default profile in dfiles.toml to deploy these externals.",
                module
            );
        }
    }
}
