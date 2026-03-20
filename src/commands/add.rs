use anyhow::{bail, Context, Result};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::config::module::expand_tilde;
use crate::fs::{is_sensitive, tilde_path};
use crate::source::encode_filename;

pub fn run(repo_root: &Path, file: &Path, link: bool) -> Result<()> {
    // Resolve and validate the source file.
    let file = resolve_path(file)?;

    if !file.exists() {
        bail!("File not found: {}", file.display());
    }
    if file.is_dir() {
        bail!("{} is a directory — only files are supported", file.display());
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

    // Auto-detect flags from the file's actual permissions.
    let metadata = std::fs::metadata(&file)?;
    let mode = metadata.permissions().mode();
    let private = !link && (mode & 0o077 == 0); // chmod 0600: no group/other bits
    let executable = !link && (mode & 0o111 != 0); // any execute bit set

    // Build the encoded path relative to source/.
    let encoded = encode_rel_path(rel, private, executable, link)?;
    let source_dest = repo_root.join("source").join(&encoded);

    // Idempotency check.
    if source_dest.exists() {
        let dest_tilde = tilde_path(&file);
        println!(
            "{} is already tracked (source/{})",
            dest_tilde,
            encoded.display()
        );
        return Ok(());
    }

    // Create intermediate directories in source/ if needed.
    if let Some(parent) = source_dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create {}", parent.display()))?;
    }

    // Copy the file into source/ with the encoded name.
    std::fs::copy(&file, &source_dest).with_context(|| {
        format!(
            "Cannot copy {} → {}",
            file.display(),
            source_dest.display()
        )
    })?;

    let dest_tilde = tilde_path(&file);
    println!("Added: {} → source/{}", dest_tilde, encoded.display());
    Ok(())
}

/// Build the encoded path inside source/ for a file relative to the home directory.
///
/// Each directory component gets `dot_` if its name starts with `.`.
/// The file component gets the full magic-name encoding via `encode_filename`.
fn encode_rel_path(rel: &Path, private: bool, executable: bool, symlink: bool) -> Result<PathBuf> {
    let components: Vec<String> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(str::to_owned))
        .collect();

    if components.is_empty() {
        bail!("Cannot determine relative path from home directory");
    }

    let n = components.len();
    let mut parts: Vec<String> = Vec::new();

    // Encode directory components (all but last): only handle `dot_` prefix.
    for comp in &components[..n - 1] {
        let encoded = if let Some(rest) = comp.strip_prefix('.') {
            format!("dot_{}", rest)
        } else {
            comp.clone()
        };
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
