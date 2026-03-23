/// `dfiles unmanaged` — find files in ~ that are not tracked by dfiles.
///
/// Walks the home directory (or a specified path) and reports files that have
/// no corresponding entry in `source/`. Useful for discovering dotfiles that
/// have not been added to dfiles yet.
///
/// At the home root, only dotfiles and dotdirs (starting with `.`) are examined —
/// `Documents/`, `Downloads/`, `Projects/` etc. are always skipped.
///
/// Specific high-noise directories are also skipped at any depth:
///   VCS: `.git`, `.jj`, `.hg`
///   Caches: `.cache`, `.npm`, `.cargo`, `node_modules`, etc.
///   App state: `.Trash`, `Library` (macOS), etc.
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::config::module::expand_tilde;
use crate::fs::tilde_path;
use crate::ignore::IgnoreList;
use crate::template::TemplateContext;
use crate::source;

pub struct UnmanagedOptions<'a> {
    pub repo_root: &'a Path,
    /// Root to walk. Defaults to `~`.
    pub root: Option<&'a Path>,
    /// Maximum directory depth below the root (0 = unlimited).
    pub depth: usize,
}

pub fn run(opts: &UnmanagedOptions<'_>) -> Result<()> {
    let home = dirs::home_dir().context("Cannot determine home directory")?;
    let walk_root = opts.root.unwrap_or(&home);

    // Build the set of tracked destination paths (expanded absolute paths).
    let source_dir = opts.repo_root.join("source");
    let ctx = TemplateContext::from_env_for_repo(opts.repo_root);
    let ignore = IgnoreList::load(opts.repo_root, &ctx);
    let entries = source::scan(&source_dir, &ignore)?;

    let mut tracked: HashSet<PathBuf> = HashSet::new();
    for entry in &entries {
        if let Ok(expanded) = expand_tilde(&entry.dest_tilde) {
            tracked.insert(expanded);
        }
    }

    let max_depth = if opts.depth == 0 { usize::MAX } else { opts.depth };
    let at_home_root = walk_root == home;
    let mut found = 0usize;

    walk_dir(walk_root, &tracked, max_depth, 0, at_home_root, &mut found);

    if found == 0 {
        println!("All files in {} are tracked by dfiles.", tilde_path(walk_root));
    }

    Ok(())
}

fn walk_dir(
    dir: &Path,
    tracked: &HashSet<PathBuf>,
    max_depth: usize,
    depth: usize,
    home_root_level: bool,
    found: &mut usize,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(it) => it,
        Err(_) => return, // skip unreadable dirs silently
    };

    let mut children: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    children.sort();

    for path in children {
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        // At the home root, only look at dotfiles/dotdirs — skip `Documents/`, etc.
        if home_root_level && !name.starts_with('.') {
            continue;
        }

        // Always skip high-noise directories.
        if path.is_dir() && is_noisy_dir(&name) {
            continue;
        }

        if path.is_file() {
            if !tracked.contains(&path) {
                println!("{}", tilde_path(&path));
                *found += 1;
            }
        } else if path.is_dir() && depth < max_depth {
            // When descending from home root, we're no longer at home root level.
            walk_dir(&path, tracked, max_depth, depth + 1, false, found);
        }
    }
}

/// Directories that are high-noise or contain no useful dotfiles.
///
/// Skipped at every depth level.
fn is_noisy_dir(name: &str) -> bool {
    matches!(
        name,
        // VCS internals (not the repos themselves).
        ".git" | ".jj" | ".hg" | ".svn"
        // Package manager / build caches.
        | "node_modules" | "target" | "__pycache__" | ".gradle" | ".m2" | ".ivy2"
        | "dist" | "build" | ".next" | ".nuxt" | ".eggs"
        // Tool caches — large, volatile, not tracked.
        | ".cache" | ".npm" | ".cargo" | ".rustup" | ".pyenv" | ".rbenv" | ".nvm"
        | ".volta" | ".asdf" | ".mise"
        // dfiles own state dir.
        | ".dfiles"
        // macOS system dirs.
        | "Library" | ".Trash" | ".Spotlight-V100" | ".DocumentRevisions-V100"
        | ".fseventsd" | ".TemporaryItems"
        // App-managed dirs rarely worth tracking manually.
        | ".android" | ".minikube" | ".kube" | ".docker" | ".vscode-server"
        | ".cocoapods" | ".bundle"
    )
}
