/// Bootstrap this machine from scratch, optionally from a remote environment package.
///
/// Bootstrap flow:
///
///   dfiles bootstrap
///       │
///       ├── read dfiles-manifest.json (if present) → print banner
///       ├── run apply (all modules in canonical order)
///       └── run status  ← confirm what was applied
///
///   dfiles bootstrap gh:alice/my-env
///       │
///       ├── fetch tarball → extract to ~/.dfiles/envs/my-env/
///       ├── read dfiles-manifest.json from extracted dir → print banner
///       ├── run apply from extracted dir
///       └── run status
///
///   dfiles bootstrap --dry-run
///       → full apply plan, no writes
///
///   dfiles bootstrap gh:alice/my-env --dry-run
///       → print "Would fetch …", no network hit, no writes
///
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::commands::{apply, status};
use crate::manifest::DfilesManifest;

pub struct BootstrapOptions<'a> {
    /// Remote environment source (`gh:owner/repo[@ref]`). `None` for local repo.
    pub source: Option<&'a str>,
    pub repo_root: &'a Path,
    pub dest_root: &'a Path,
    pub backup_dir: &'a Path,
    pub state_dir: &'a Path,
    pub claude_dir: &'a Path,
    /// Where remote environment packages are extracted: `~/.dfiles/envs/`.
    pub envs_dir: &'a Path,
    pub profile: &'a str,
    pub dry_run: bool,
}

pub fn run(opts: &BootstrapOptions<'_>) -> Result<()> {
    // ── Remote source: fetch before applying ─────────────────────────────────
    let apply_root: PathBuf = if let Some(source_str) = opts.source {
        let source = crate::github::GhSource::parse(source_str)
            .with_context(|| format!("Invalid bootstrap source: {}", source_str))?;

        if opts.dry_run {
            println!(
                "Dry run — would fetch {} and apply profile '{}'.",
                source_str, opts.profile
            );
            return Ok(());
        }

        println!("Fetching {} …", source_str);
        std::fs::create_dir_all(opts.envs_dir)
            .context("Cannot create envs directory")?;
        crate::github::fetch_to_dir(&source, opts.envs_dir)
            .with_context(|| format!("Failed to fetch {}", source_str))?;

        opts.envs_dir.join(source.name())
    } else {
        opts.repo_root.to_path_buf()
    };

    // ── Print banner from manifest (if present) ───────────────────────────────
    print_banner(&apply_root, opts.profile);

    // ── Apply all modules ─────────────────────────────────────────────────────
    apply::run(&apply::ApplyOptions {
        repo_root: &apply_root,
        dest_root: opts.dest_root,
        backup_dir: opts.backup_dir,
        state_dir: opts.state_dir,
        claude_dir: opts.claude_dir,
        profile: opts.profile,
        module_filter: None,
        dry_run: opts.dry_run,
    })?;

    // ── Status summary (non-dry-run only) ─────────────────────────────────────
    if !opts.dry_run {
        println!("\n─── Environment status ───────────────────────────────────");
        status::run(&status::StatusOptions {
            repo_root: &apply_root,
            dest_root: opts.dest_root,
            claude_dir: opts.claude_dir,
            profile: opts.profile,
        })?;
    }

    Ok(())
}

/// Print a one-line banner from `dfiles-manifest.json` if the file exists.
fn print_banner(repo_root: &Path, profile: &str) {
    match DfilesManifest::load(repo_root) {
        Ok(m) => {
            let author_part = m
                .author
                .as_deref()
                .map(|a| format!(" by {}", a))
                .unwrap_or_default();
            println!(
                "Bootstrapping: {} {} (profile: {}){}",
                m.name, m.version, profile, author_part
            );
        }
        Err(_) => {
            // No manifest present (local repo without publish) — just show the profile.
            println!("Bootstrapping profile: {}", profile);
        }
    }
}
