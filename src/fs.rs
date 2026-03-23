/// File system utilities: backup, copy, sensitive-file detection.
use anyhow::{Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};

/// Patterns that should never be auto-tracked without explicit confirmation.
/// Checked against the filename (not the full path).
const SENSITIVE_PATTERNS: &[&str] = &[
    ".env",
    ".npmrc",
    ".pypirc",
    ".netrc",
    ".htpasswd",
    "credentials",
    "secrets",
];

/// Glob-style suffix patterns for sensitive files.
const SENSITIVE_SUFFIXES: &[&str] = &[
    "_rsa",
    "_rsa.pub",
    "_ed25519",
    "_ed25519.pub",
    "_ecdsa",
    "_dsa",
    ".pem",
    ".pfx",
    ".p12",
    ".key",
    ".crt",
    ".cer",
];

/// Returns the matched rule name if the (decoded) filename matches a sensitive pattern,
/// or `None` if it does not.
///
/// `name` should be the **decoded** filename (e.g. `id_rsa`, `.env`),
/// not the encoded source path name (e.g. `private_id_rsa`, `dot_env`).
pub fn is_sensitive_with_rule(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    for &p in SENSITIVE_PATTERNS {
        if lower == p || lower.ends_with(p) {
            return Some(p);
        }
    }
    for &s in SENSITIVE_SUFFIXES {
        if lower.ends_with(s) {
            return Some(s);
        }
    }
    None
}

/// Returns true if the filename matches a sensitive pattern.
pub fn is_sensitive(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_lowercase();

    if SENSITIVE_PATTERNS.iter().any(|p| name == *p || name.ends_with(p)) {
        return true;
    }
    if SENSITIVE_SUFFIXES.iter().any(|s| name.ends_with(s)) {
        return true;
    }
    false
}

/// Back up an existing file to the backup directory before overwriting.
/// Returns the backup path.
pub fn backup_file(live_path: &Path, backup_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(backup_dir)?;

    let stem = live_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let ts = Utc::now().format("%Y%m%dT%H%M%SZ");
    let backup_name = format!("{}.{}", stem, ts);
    let backup_path = backup_dir.join(&backup_name);

    std::fs::copy(live_path, &backup_path).with_context(|| {
        format!(
            "Cannot back up {} to {}",
            live_path.display(),
            backup_path.display()
        )
    })?;

    Ok(backup_path)
}

/// Write rendered template content to `dest`, creating parent directories as needed.
pub fn write_to_dest(content: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("Cannot create directory {}", parent.display())
        })?;
    }
    std::fs::write(dest, content)
        .with_context(|| format!("Cannot write {}", dest.display()))?;
    Ok(())
}

/// Convert an absolute path to a `~/`-prefixed string for storage in TOML dest fields.
/// If the path is not under the home directory, returns the full absolute path as a string.
pub fn tilde_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rel) = path.strip_prefix(&home) {
            return format!("~/{}", rel.display());
        }
    }
    path.display().to_string()
}

/// Set file permissions on `dest` based on `private` and `executable` flags.
///
/// | private | executable | dest is dir | mode  |
/// |---------|-----------|-------------|-------|
/// | true    | true      | any         | 0700  |
/// | true    | false     | yes         | 0700  | ← directories always need +x to be traversable
/// | true    | false     | no          | 0600  |
/// | false   | true      | any         | 0755  |
/// | false   | false     | any         | (no-op) |
#[cfg(unix)]
pub fn apply_permissions(dest: &Path, private: bool, executable: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let is_dir = dest.is_dir();
    let mode = match (private, executable || is_dir) {
        (true, true) => 0o700,
        (true, false) => 0o600,
        (false, true) => 0o755,
        (false, false) => return Ok(()),
    };
    std::fs::set_permissions(dest, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("Cannot set permissions on {}", dest.display()))?;
    Ok(())
}

#[cfg(not(unix))]
pub fn apply_permissions(_dest: &Path, _private: bool, _executable: bool) -> Result<()> {
    Ok(()) // Windows: permission bits are not supported
}

/// Copy `src` to `dest`, creating parent directories as needed.
pub fn copy_to_dest(src: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("Cannot create directory {}", parent.display())
        })?;
    }
    std::fs::copy(src, dest)
        .with_context(|| format!("Cannot copy {} → {}", src.display(), dest.display()))?;
    Ok(())
}
