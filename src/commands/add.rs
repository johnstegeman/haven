use anyhow::{bail, Context, Result};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::commands::security_scan::scan_single_file_content;
use crate::config::haven::HavenConfig;
use crate::config::module::expand_tilde;
use crate::fs::{is_sensitive, tilde_path};
use crate::ignore::IgnoreList;
use crate::template::TemplateContext;
use crate::source::{encode_filename, extdir_source_path};

pub fn run(repo_root: &Path, file: &Path, link: bool, apply: bool, update: bool) -> Result<()> {
    let file = resolve_path(file)?;

    if !file.exists() {
        bail!("File not found: {}", file.display());
    }

    let ctx = TemplateContext::from_env_for_repo(repo_root);
    let ignore = IgnoreList::load(repo_root, &ctx);

    if file.is_dir() {
        return run_dir(repo_root, &file, &ignore);
    }

    // Sensitive file check.
    if is_sensitive(&file) && !confirm_sensitive(&file)? {
        println!("Skipped: {}", file.display());
        return Ok(());
    }

    // The file must be under the home directory so we can encode the path.
    let home = dirs::home_dir().context("Cannot determine home directory")?;
    let rel = file
        .strip_prefix(&home)
        .with_context(|| format!("{} is not under your home directory", file.display()))?;

    // Ignore check — must happen after we have the dest_tilde.
    let dest_tilde = tilde_path(&file);
    if ignore.is_ignored(&dest_tilde) {
        println!(
            "Skipped: {} (matches a pattern in config/ignore — remove the pattern first if you want to track this file)",
            dest_tilde,
        );
        return Ok(());
    }

    // Auto-detect flags from the file's actual permissions.
    let metadata = std::fs::metadata(&file)?;
    let mode = metadata.permissions().mode();
    let private = !link && (mode & 0o077 == 0); // no group/other bits → chmod 0600
    let executable = !link && (mode & 0o111 != 0); // any execute bit set

    // Build the encoded path relative to source/.
    let encoded = encode_rel_path(&home, rel, private, executable, link)?;
    let source_dest = repo_root.join("source").join(&encoded);

    // Idempotency / update check.
    if source_dest.exists() {
        if update {
            // --update: fall through and overwrite.
        } else {
            bail!(
                "{} is already tracked (source/{}). Use --update to re-copy.",
                tilde_path(&file),
                encoded.display()
            );
        }
    }

    // Create intermediate directories in source/ if needed.
    if let Some(parent) = source_dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create {}", parent.display()))?;
    }

    // Copy the file into source/ with the encoded name, stripping any managed
    // section content so the repo always stores empty markers.
    let file_content = std::fs::read_to_string(&file).ok();
    if let Some(content) = file_content {
        let stripped = crate::config_injection::strip_managed_content(&content);
        std::fs::write(&source_dest, &stripped).with_context(|| {
            format!("Cannot write {}", source_dest.display())
        })?;
    } else {
        // Binary file — copy as-is (managed sections only apply to text files).
        std::fs::copy(&file, &source_dest).with_context(|| {
            format!(
                "Cannot copy {} → {}",
                file.display(),
                source_dest.display()
            )
        })?;
    }

    println!("Added: {} → source/{}", dest_tilde, encoded.display());

    // Content scan: warn if the newly-added file contains sensitive patterns.
    // Respect [security] allow list from haven.toml.
    let config = HavenConfig::load(repo_root).unwrap_or_default();
    let allow_list = crate::commands::security_scan::make_allow_list(&config.security.allow);
    if !allow_list.is_ignored(&dest_tilde) {
        let content_findings = scan_single_file_content(&source_dest, &dest_tilde);
        if !content_findings.is_empty() {
            println!(
                "warning: {} may contain sensitive content ({} pattern(s) found).",
                dest_tilde,
                content_findings.len()
            );
            for f in &content_findings {
                println!("  · {} ({})", f.rule, f.severity.label());
            }
            print!("Track it anyway? [y/N] ");
            io::stdout().flush()?;
            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
            if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
                std::fs::remove_file(&source_dest).with_context(|| {
                    format!("Cannot remove {}", source_dest.display())
                })?;
                println!("Removed from tracking: {}", dest_tilde);
                return Ok(());
            }
        }
    }

    // --apply: immediately replace the original file with a symlink back into source/.
    if apply {
        install_symlink(&file, &source_dest)?;
    }

    Ok(())
}

/// Replace `dest` with a symlink pointing at `source_file`.
///
/// Backs up `dest` to `<dest>.bak` if it exists and is not already the
/// correct symlink. On success prints a confirmation line.
fn install_symlink(dest: &Path, source_file: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;

    // If dest already is the correct symlink, nothing to do.
    if dest.is_symlink() {
        if std::fs::read_link(dest).ok().as_deref() == Some(source_file) {
            println!("Symlink already in place: {}", dest.display());
            return Ok(());
        }
    }

    // Back up the existing file before replacing it.
    if dest.exists() || dest.is_symlink() {
        let backup_name = format!(
            "{}.bak",
            dest.file_name().unwrap_or_default().to_string_lossy()
        );
        let backup = dest.with_file_name(backup_name);
        std::fs::rename(dest, &backup)
            .with_context(|| format!("Cannot back up {} → {}", dest.display(), backup.display()))?;
        println!("  backed up {} → {}", dest.display(), backup.display());
    }

    symlink(source_file, dest)
        .with_context(|| format!("Cannot create symlink {} → {}", dest.display(), source_file.display()))?;

    println!(
        "  linked {} → {}",
        dest.display(),
        source_file.display(),
    );
    Ok(())
}

// ─── Directory handling ───────────────────────────────────────────────────────

fn run_dir(repo_root: &Path, dir: &Path, ignore: &IgnoreList) -> Result<()> {
    let remotes = get_git_remotes(dir);

    if !remotes.is_empty() {
        match prompt_dir_mode(dir, &remotes)? {
            DirMode::Extdir(idx) => {
                let (remote_name, url) = &remotes[idx];
                add_as_extdir(repo_root, dir, remote_name, url, ignore)?;
                return Ok(());
            }
            DirMode::Recursive => {} // fall through to recursive add
            DirMode::Skip => {
                println!("Skipped: {}", dir.display());
                return Ok(());
            }
        }
    }

    add_dir_recursive(repo_root, dir, ignore)
}

fn add_as_extdir(repo_root: &Path, dir: &Path, _remote_name: &str, url: &str, ignore: &IgnoreList) -> Result<()> {
    let home = dirs::home_dir().context("Cannot determine home directory")?;
    let dest_tilde = tilde_path(dir);
    let repo_source = repo_root.join("source");

    if ignore.is_ignored(&dest_tilde) {
        println!(
            "Skipped: {} (matches a pattern in config/ignore — remove the pattern first if you want to track this directory)",
            dest_tilde,
        );
        return Ok(());
    }

    let extdir_path = extdir_source_path(&repo_source, &dest_tilde);

    if extdir_path.exists() {
        println!(
            "{} is already tracked as external (source/{})",
            dest_tilde,
            extdir_path.strip_prefix(&repo_source).unwrap_or(&extdir_path).display()
        );
        return Ok(());
    }

    if !dir.strip_prefix(&home).is_ok() {
        bail!("{} is not under your home directory", dir.display());
    }

    if let Some(parent) = extdir_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create {}", parent.display()))?;
    }

    let content = format!("type = \"git\"\nurl  = {:?}\n", url);
    std::fs::write(&extdir_path, &content)
        .with_context(|| format!("Cannot write {}", extdir_path.display()))?;

    println!(
        "Added external: {} → source/{}  ({})",
        dest_tilde,
        extdir_path.strip_prefix(&repo_source).unwrap_or(&extdir_path).display(),
        url,
    );
    Ok(())
}

fn add_dir_recursive(repo_root: &Path, dir: &Path, ignore: &IgnoreList) -> Result<()> {
    let home = dirs::home_dir().context("Cannot determine home directory")?;
    let repo_source = repo_root.join("source");
    let mut count = 0;

    for dent in WalkDir::new(dir)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            // Skip hidden directories (e.g. .git) but still descend into
            // non-hidden ones. Hidden *files* are allowed — they'll get dot_ encoding.
            !e.file_type().is_dir()
                || !e.file_name().to_str().map(|s| s.starts_with('.')).unwrap_or(false)
        })
    {
        let dent = dent.with_context(|| format!("Cannot walk {}", dir.display()))?;
        if !dent.file_type().is_file() {
            continue;
        }
        let file = dent.path().to_path_buf();

        if is_sensitive(&file) && !confirm_sensitive(&file)? {
            println!("  ~ Skipped (sensitive): {}", tilde_path(&file));
            continue;
        }

        let file_tilde = tilde_path(&file);
        if ignore.is_ignored(&file_tilde) {
            println!("  ~ Skipped (ignored): {}", file_tilde);
            continue;
        }

        let rel = match file.strip_prefix(&home) {
            Ok(r) => r.to_path_buf(),
            Err(_) => {
                println!("  ~ Skipped (not under home): {}", file.display());
                continue;
            }
        };

        let metadata = std::fs::metadata(&file)?;
        let mode = metadata.permissions().mode();
        let private = mode & 0o077 == 0;
        let executable = mode & 0o111 != 0;

        let encoded = encode_rel_path(&home, &rel, private, executable, false)?;
        let source_dest = repo_source.join(&encoded);

        if source_dest.exists() {
            println!("  ~ {} already tracked", tilde_path(&file));
            continue;
        }

        if let Some(parent) = source_dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&file, &source_dest).with_context(|| {
            format!("Cannot copy {} → {}", file.display(), source_dest.display())
        })?;

        println!("  ✓  {} → source/{}", tilde_path(&file), encoded.display());
        count += 1;
    }

    println!("Added {} file(s) from {}", count, tilde_path(dir));
    Ok(())
}

// ─── Git remote detection ─────────────────────────────────────────────────────

/// Return `(name, url)` pairs for all fetch remotes of the git repo at `dir`.
/// Returns an empty vec if `dir` is not a git repo or has no remotes.
fn get_git_remotes(dir: &Path) -> Vec<(String, String)> {
    // Quick check: is there a .git entry?
    if !dir.join(".git").exists() {
        return Vec::new();
    }

    let output = std::process::Command::new("git")
        .args(["-C"])
        .arg(dir)
        .args(["remote", "-v"])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut remotes: Vec<(String, String)> = Vec::new();

    for line in stdout.lines() {
        // Lines look like: "origin\thttps://github.com/foo/bar (fetch)"
        if !line.ends_with("(fetch)") {
            continue;
        }
        let mut parts = line.splitn(2, '\t');
        let name = match parts.next() {
            Some(n) => n.to_string(),
            None => continue,
        };
        let rest = match parts.next() {
            Some(r) => r,
            None => continue,
        };
        // Strip " (fetch)" suffix.
        let url = rest.trim_end_matches(" (fetch)").trim().to_string();
        if !remotes.iter().any(|(n, _)| n == &name) {
            remotes.push((name, url));
        }
    }

    remotes
}

// ─── Interactive prompt ───────────────────────────────────────────────────────

enum DirMode {
    Extdir(usize), // index into remotes vec
    Recursive,
    Skip,
}

fn prompt_dir_mode(dir: &Path, remotes: &[(String, String)]) -> Result<DirMode> {
    let dest_tilde = tilde_path(dir);
    println!(
        "{} is a git repository with {} remote(s):\n",
        dest_tilde,
        remotes.len()
    );
    for (i, (name, url)) in remotes.iter().enumerate() {
        println!("  {}) {:<12} {}", i + 1, name, url);
    }
    println!();

    // Build the prompt options.
    let extdir_options: String = (1..=remotes.len()).map(|i| i.to_string()).collect::<Vec<_>>().join("/");
    print!(
        "How to add?\n  {options}  Add as external (cloned on apply)\n  f) Add all files recursively\n  q) Skip\n[{options}/f/q]: ",
        options = extdir_options,
    );
    io::stdout().flush()?;

    loop {
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let input = line.trim().to_lowercase();
        match input.as_str() {
            "f" => return Ok(DirMode::Recursive),
            "q" | "" => return Ok(DirMode::Skip),
            s => {
                if let Ok(n) = s.parse::<usize>() {
                    if n >= 1 && n <= remotes.len() {
                        return Ok(DirMode::Extdir(n - 1));
                    }
                }
                print!("Please enter {}/f/q: ", extdir_options);
                io::stdout().flush()?;
            }
        }
    }
}

// ─── Path encoding ────────────────────────────────────────────────────────────

/// Build the encoded path inside `source/` for a file relative to the home directory.
///
/// Each directory component is encoded with [`encode_filename`], checking the
/// component's actual on-disk permissions to set the `private_` prefix where
/// appropriate (e.g. `~/.ssh` → `private_dot_ssh`).
///
/// The final file component gets full magic-name encoding via [`encode_filename`].
fn encode_rel_path(home: &Path, rel: &Path, private: bool, executable: bool, symlink: bool) -> Result<PathBuf> {
    let components: Vec<String> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(str::to_owned))
        .collect();

    if components.is_empty() {
        bail!("Cannot determine relative path from home directory");
    }

    let n = components.len();
    let mut parts: Vec<String> = Vec::new();

    // Encode directory components (all but last).
    // Check each directory's actual permissions for private_ encoding.
    for i in 0..n - 1 {
        let comp = &components[i];
        let dir_path: PathBuf = home.join(components[..=i].iter().collect::<PathBuf>());
        let dir_private = std::fs::metadata(&dir_path)
            .map(|m| m.permissions().mode() & 0o077 == 0)
            .unwrap_or(false);
        let encoded = encode_filename(comp, dir_private, false, false, false);
        parts.push(encoded);
    }

    // Encode the file component (last) with full flag encoding.
    let file_name = &components[n - 1];
    let encoded_file = encode_filename(file_name, private, executable, symlink, false);
    parts.push(encoded_file);

    Ok(parts.iter().collect::<PathBuf>())
}

/// Resolve a path, expanding `~` if needed.
fn resolve_path(path: &Path) -> Result<PathBuf> {
    let s = path.to_string_lossy();
    if s.starts_with("~/") || s == "~" {
        expand_tilde(&s)
    } else {
        Ok(path.to_path_buf())
    }
}

/// Prompt the user to confirm tracking a sensitive file.
fn confirm_sensitive(path: &Path) -> Result<bool> {
    print!(
        "Warning: '{}' matches a sensitive file pattern.\n\
         It may contain secrets or credentials.\n\
         Track it anyway? [y/N] ",
        path.display()
    );
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}
