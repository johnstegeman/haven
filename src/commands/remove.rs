/// Remove a tracked dotfile from the source/ directory.
///
/// Scans source/ for entries whose decoded destination matches the given path,
/// then deletes the source file. The live file on disk is left unchanged.
use anyhow::{bail, Context, Result};
use std::path::Path;

use crate::config::module::expand_tilde;
use crate::ignore::IgnoreList;
use crate::template::TemplateContext;
use crate::source;

pub struct RemoveOptions<'a> {
    pub repo_root: &'a Path,
    /// Path to stop tracking — accepts `~`-prefixed or absolute paths.
    pub file: &'a Path,
    /// Print what would be removed without deleting anything.
    pub dry_run: bool,
}

pub fn run(opts: &RemoveOptions<'_>) -> Result<()> {
    let source_dir = opts.repo_root.join("source");
    let ctx = TemplateContext::from_env_for_repo(opts.repo_root);
    let ignore = IgnoreList::load(opts.repo_root, &ctx);
    let entries = source::scan(&source_dir, &ignore)?;

    // Normalise the requested path so we can compare against dest_tilde.
    let given_str = opts.file.to_string_lossy();
    let given_expanded = expand_tilde(&given_str)?;

    let matches: Vec<_> = entries
        .iter()
        .filter(|e| {
            // Match by tilde string (e.g. "~/.zshrc").
            if e.dest_tilde == *given_str {
                return true;
            }
            // Match by expanded absolute path.
            if let Ok(expanded) = expand_tilde(&e.dest_tilde) {
                if expanded == given_expanded {
                    return true;
                }
            }
            false
        })
        .collect();

    if matches.is_empty() {
        bail!(
            "No tracked file found for '{}'. \
             Run `dfiles status` to see which files are tracked.",
            opts.file.display()
        );
    }

    for entry in &matches {
        if opts.dry_run {
            println!("Would remove: {}", entry.src.display());
        } else {
            std::fs::remove_file(&entry.src)
                .with_context(|| format!("Cannot remove {}", entry.src.display()))?;
            println!("Removed: {} ({})", entry.dest_tilde, entry.src.display());
        }
    }

    if opts.dry_run {
        println!("(dry run — no files removed)");
    }

    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn setup_repo_with_file(dest_name: &str) -> (TempDir, PathBuf) {
        let repo = TempDir::new().unwrap();
        let source_dir = repo.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();

        // Encode the dest_name as a source filename (e.g. ~/.zshrc → dot_zshrc).
        let encoded = dest_name
            .trim_start_matches("~/.")
            .to_string();
        let encoded = format!("dot_{}", encoded);
        let src_file = source_dir.join(&encoded);
        fs::write(&src_file, "# content\n").unwrap();

        (repo, src_file)
    }

    #[test]
    fn removes_tracked_file_by_tilde_path() {
        let (repo, src_file) = setup_repo_with_file("zshrc");
        assert!(src_file.exists());

        run(&RemoveOptions {
            repo_root: repo.path(),
            file: Path::new("~/.zshrc"),
            dry_run: false,
        })
        .unwrap();

        assert!(!src_file.exists(), "source file should have been deleted");
    }

    #[test]
    fn dry_run_does_not_delete() {
        let (repo, src_file) = setup_repo_with_file("bashrc");
        assert!(src_file.exists());

        run(&RemoveOptions {
            repo_root: repo.path(),
            file: Path::new("~/.bashrc"),
            dry_run: true,
        })
        .unwrap();

        assert!(src_file.exists(), "dry-run must not delete the file");
    }

    #[test]
    fn errors_when_file_not_tracked() {
        let repo = TempDir::new().unwrap();
        fs::create_dir_all(repo.path().join("source")).unwrap();

        let result = run(&RemoveOptions {
            repo_root: repo.path(),
            file: Path::new("~/.not_tracked"),
            dry_run: false,
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No tracked file"));
    }
}
