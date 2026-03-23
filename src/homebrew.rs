/// Homebrew integration: detection, optional install, and Brewfile management.
///
/// Detection order:
///   1. `brew` in PATH
///   2. Known install locations (Apple Silicon, Intel macOS, Linux)
///
/// Install flow (when brew is absent and user consents):
///   /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
///
/// Brewfile operations:
///   apply  → `brew bundle install --file=<path>`
///   status → `brew bundle check  --file=<path>`
use anyhow::{Context, Result};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const BREW_INSTALL_URL: &str =
    "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh";

/// Well-known Homebrew binary locations checked when `brew` is not in PATH.
const BREW_LOCATIONS: &[&str] = &[
    "/opt/homebrew/bin/brew",              // macOS Apple Silicon
    "/usr/local/bin/brew",                 // macOS Intel
    "/home/linuxbrew/.linuxbrew/bin/brew", // Linux
];

/// Find the `brew` binary. Checks PATH first, then known install locations.
pub fn brew_path() -> Option<PathBuf> {
    // PATH lookup via `which`.
    if let Ok(out) = std::process::Command::new("which").arg("brew").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
    }
    // Fall back to well-known locations.
    BREW_LOCATIONS
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

/// Ensure Homebrew is available.
///
/// - If brew is already installed, returns `Ok(Some(path))` immediately.
/// - If not installed and `dry_run` is true, prints a note and returns `Ok(None)`.
/// - If not installed and interactive, prompts the user:
///     - "y" → runs the official install script with inherited stdio, then re-checks.
///     - "n" → returns `Ok(None)` (caller should skip Homebrew steps).
///
/// Returns `Err` only if the installer was invoked but failed.
pub fn ensure_brew(dry_run: bool) -> Result<Option<PathBuf>> {
    if let Some(p) = brew_path() {
        return Ok(Some(p));
    }

    if dry_run {
        println!("  [homebrew] brew not found — would offer to install");
        return Ok(None);
    }

    print!(
        "\nHomebrew is not installed.\n\
         Install it now? [y/N] "
    );
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;

    if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
        println!("Skipping Homebrew installation.");
        return Ok(None);
    }

    println!("Installing Homebrew…");
    println!("(You may be prompted for your password.)\n");

    let status = std::process::Command::new("/bin/bash")
        .arg("-c")
        .arg(format!("$(curl -fsSL {})", BREW_INSTALL_URL))
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .context("Failed to launch Homebrew installer")?;

    if !status.success() {
        anyhow::bail!("Homebrew installation failed (exit {:?})", status.code());
    }

    // Re-check after install (brew may now be in a non-PATH location).
    match brew_path() {
        Some(p) => {
            println!("\nHomebrew installed at {}", p.display());
            Ok(Some(p))
        }
        None => {
            // Installer succeeded but brew isn't on PATH yet — shell restart needed.
            println!(
                "\nHomebrew was installed, but `brew` is not yet on your PATH.\n\
                 Open a new terminal or run:\n\
                 \n  eval \"$({}/shellenv)\"\n",
                if cfg!(target_arch = "aarch64") {
                    "/opt/homebrew/bin/brew"
                } else {
                    "/usr/local/bin/brew"
                }
            );
            // Return the known path directly rather than failing.
            Ok(BREW_LOCATIONS.iter().map(PathBuf::from).find(|p| p.exists()))
        }
    }
}

/// Run `brew bundle install --file=<brewfile>`.
pub fn bundle_install(brew: &Path, brewfile: &Path) -> Result<()> {
    let status = std::process::Command::new(brew)
        .args(["bundle", "install", "--file"])
        .arg(brewfile)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .with_context(|| {
            format!(
                "Cannot run `brew bundle install` for {}",
                brewfile.display()
            )
        })?;

    if !status.success() {
        anyhow::bail!(
            "`brew bundle install` failed for {} (exit {:?})",
            brewfile.display(),
            status.code()
        );
    }
    Ok(())
}

/// Run `brew bundle check --file=<brewfile>`.
///
/// Returns `true` if all packages are installed, `false` otherwise.
/// Silently returns `false` on any subprocess error.
pub fn bundle_check(brew: &Path, brewfile: &Path) -> bool {
    std::process::Command::new(brew)
        .args(["bundle", "check", "--file"])
        .arg(brewfile)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run `brew install [--cask] <name>`.
pub fn brew_install(brew: &Path, name: &str, cask: bool) -> Result<()> {
    let mut cmd = std::process::Command::new(brew);
    cmd.arg("install");
    if cask {
        cmd.arg("--cask");
    }
    cmd.arg(name)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    let status = cmd.status().with_context(|| format!("Cannot run `brew install {}`", name))?;
    if !status.success() {
        anyhow::bail!("`brew install {}` failed (exit {:?})", name, status.code());
    }
    Ok(())
}

/// Run `brew uninstall [--cask] <name>`.
pub fn brew_uninstall(brew: &Path, name: &str, cask: bool) -> Result<()> {
    let mut cmd = std::process::Command::new(brew);
    cmd.arg("uninstall");
    if cask {
        cmd.arg("--cask");
    }
    cmd.arg(name)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    let status = cmd.status().with_context(|| format!("Cannot run `brew uninstall {}`", name))?;
    if !status.success() {
        anyhow::bail!("`brew uninstall {}` failed (exit {:?})", name, status.code());
    }
    Ok(())
}

// ─── Brewfile entry collection ────────────────────────────────────────────────

/// All `brew` and `cask` entries declared across one or more Brewfiles.
#[derive(Debug, Default)]
pub struct BrewfileEntries {
    pub formulas: std::collections::HashSet<String>,
    pub casks: std::collections::HashSet<String>,
}

/// Extract the quoted name from a Brewfile line like `brew "name"` or `cask 'name'`.
/// Returns `None` for comments, blank lines, taps, and malformed entries.
fn extract_entry_name(line: &str, kind: &str) -> Option<String> {
    let prefix = format!("{} ", kind);
    let rest = line.strip_prefix(&prefix)?;
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let inner = &rest[1..];
    let end = inner.find(quote)?;
    Some(inner[..end].to_string())
}

/// Parse all `brew` and `cask` entries from a single Brewfile.
/// Returns an empty set if the file does not exist.
fn parse_brewfile(path: &Path) -> Result<BrewfileEntries> {
    let mut entries = BrewfileEntries::default();
    if !path.exists() {
        return Ok(entries);
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read {}", path.display()))?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = extract_entry_name(line, "brew") {
            entries.formulas.insert(name);
        } else if let Some(name) = extract_entry_name(line, "cask") {
            entries.casks.insert(name);
        }
    }
    Ok(entries)
}

/// Collect all `brew` and `cask` entries declared across a set of Brewfiles.
/// Duplicate entries across files are unified (set semantics).
pub fn collect_brewfile_entries(paths: &[&Path]) -> Result<BrewfileEntries> {
    let mut all = BrewfileEntries::default();
    for &path in paths {
        let entries = parse_brewfile(path)?;
        all.formulas.extend(entries.formulas);
        all.casks.extend(entries.casks);
    }
    Ok(all)
}

/// The result of comparing Brewfile declarations against the live system.
///
/// Fields:
///   missing_*  — declared in Brewfiles but not installed (apply would add them)
///   extra_*    — installed but not declared   (--remove-unreferenced-brews would remove them)
#[derive(Debug, Default)]
pub struct BrewfileDiff {
    pub missing_formulas: Vec<String>,
    pub missing_casks: Vec<String>,
    pub extra_formulas: Vec<String>,
    pub extra_casks: Vec<String>,
}

impl BrewfileDiff {
    pub fn is_clean(&self) -> bool {
        self.missing_formulas.is_empty()
            && self.missing_casks.is_empty()
            && self.extra_formulas.is_empty()
            && self.extra_casks.is_empty()
    }
}

/// Compare the packages declared across `brewfile_paths` against what is installed.
///
/// Uses `brew list --formula` (all installed formulas, not just leaves) to detect
/// packages that are declared but missing.  Uses `brew leaves` for the reverse
/// direction (installed but unreferenced) so that auto-installed dependencies are
/// not flagged as extra.
pub fn brewfile_diff(brew: &Path, brewfile_paths: &[&Path]) -> Result<BrewfileDiff> {
    let declared = collect_brewfile_entries(brewfile_paths)?;

    let all_installed_formulas: std::collections::HashSet<String> =
        brew_list_formula(brew)?.into_iter().collect();
    let all_installed_casks: std::collections::HashSet<String> =
        brew_list_casks(brew)?.into_iter().collect();

    // Declared but not installed.
    let mut missing_formulas: Vec<String> = declared
        .formulas
        .iter()
        .filter(|f| !all_installed_formulas.contains(*f))
        .cloned()
        .collect();
    let mut missing_casks: Vec<String> = declared
        .casks
        .iter()
        .filter(|c| !all_installed_casks.contains(*c))
        .cloned()
        .collect();

    // Installed but not declared (leaves only for formulas).
    let leaves = brew_leaves(brew)?;
    let mut extra_formulas: Vec<String> = leaves
        .into_iter()
        .filter(|f| !declared.formulas.contains(f))
        .collect();
    let mut extra_casks: Vec<String> = all_installed_casks
        .into_iter()
        .filter(|c| !declared.casks.contains(c))
        .collect();

    missing_formulas.sort();
    missing_casks.sort();
    extra_formulas.sort();
    extra_casks.sort();

    Ok(BrewfileDiff {
        missing_formulas,
        missing_casks,
        extra_formulas,
        extra_casks,
    })
}

/// Run `brew list --formula` and return all installed formula names (including deps).
pub fn brew_list_formula(brew: &Path) -> Result<Vec<String>> {
    let out = std::process::Command::new(brew)
        .args(["list", "--formula"])
        .output()
        .context("Failed to run `brew list --formula`")?;
    if !out.status.success() {
        anyhow::bail!("`brew list --formula` failed (exit {:?})", out.status.code());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// Run `brew leaves` and return the list of leaf formula names.
///
/// Leaf formulas are installed but not required as a dependency by any other
/// installed formula — i.e., packages the user explicitly requested.
pub fn brew_leaves(brew: &Path) -> Result<Vec<String>> {
    let out = std::process::Command::new(brew)
        .arg("leaves")
        .output()
        .context("Failed to run `brew leaves`")?;
    if !out.status.success() {
        anyhow::bail!("`brew leaves` failed (exit {:?})", out.status.code());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// Run `brew list --cask` and return all installed cask names.
pub fn brew_list_casks(brew: &Path) -> Result<Vec<String>> {
    let out = std::process::Command::new(brew)
        .args(["list", "--cask"])
        .output()
        .context("Failed to run `brew list --cask`")?;
    if !out.status.success() {
        anyhow::bail!("`brew list --cask` failed (exit {:?})", out.status.code());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

// ─── Brewfile line matching (used by add/remove helpers) ──────────────────────

/// Check whether a single line in a Brewfile matches `kind "name"` (or single-quoted).
fn brewfile_line_matches(line: &str, kind: &str, name: &str) -> bool {
    let line = line.trim();
    let prefix = format!("{} ", kind);
    if !line.starts_with(&prefix) {
        return false;
    }
    let rest = &line[prefix.len()..];
    let quote = if rest.starts_with('"') {
        '"'
    } else if rest.starts_with('\'') {
        '\''
    } else {
        return false;
    };
    let inner = &rest[1..];
    if let Some(end) = inner.find(quote) {
        &inner[..end] == name
    } else {
        false
    }
}

/// Add a `brew`/`cask`/`tap` entry to a Brewfile.
///
/// Creates the file (and any parent directories) if it does not exist.
/// Returns `true` if the entry was added, `false` if it was already present.
pub fn add_to_brewfile(path: &Path, kind: &str, name: &str) -> Result<bool> {
    let existing = if path.exists() {
        std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read {}", path.display()))?
    } else {
        String::new()
    };

    if existing.lines().any(|line| brewfile_line_matches(line, kind, name)) {
        return Ok(false);
    }

    // Build updated content: ensure a trailing newline before appending.
    let mut new_content = existing;
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    new_content.push_str(&format!("{} \"{}\"\n", kind, name));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &new_content)
        .with_context(|| format!("Cannot write {}", path.display()))?;

    Ok(true)
}

/// Remove all `brew`/`cask`/`tap` lines matching `name` from a Brewfile.
///
/// Returns the number of lines removed (0 if none matched or the file does not exist).
pub fn remove_from_brewfile(path: &Path, kind: &str, name: &str) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }

    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read {}", path.display()))?;

    let mut removed = 0usize;
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| {
            if brewfile_line_matches(line, kind, name) {
                removed += 1;
                false
            } else {
                true
            }
        })
        .collect();

    if removed > 0 {
        let mut new_content = kept.join("\n");
        if !new_content.is_empty() {
            new_content.push('\n');
        }
        std::fs::write(path, new_content)
            .with_context(|| format!("Cannot write {}", path.display()))?;
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_brewfile(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn extract_double_quoted_formula() {
        assert_eq!(
            extract_entry_name(r#"brew "ripgrep""#, "brew"),
            Some("ripgrep".into())
        );
    }

    #[test]
    fn extract_single_quoted_formula() {
        assert_eq!(
            extract_entry_name("brew 'ripgrep'", "brew"),
            Some("ripgrep".into())
        );
    }

    #[test]
    fn extract_cask_name() {
        assert_eq!(
            extract_entry_name(r#"cask "iterm2""#, "cask"),
            Some("iterm2".into())
        );
    }

    #[test]
    fn extract_wrong_kind_returns_none() {
        assert_eq!(extract_entry_name(r#"cask "iterm2""#, "brew"), None);
    }

    #[test]
    fn extract_tap_returns_none_for_brew_kind() {
        assert_eq!(extract_entry_name(r#"tap "homebrew/core""#, "brew"), None);
    }

    #[test]
    fn parse_brewfile_reads_formulas_and_casks() {
        let dir = TempDir::new().unwrap();
        let path = write_brewfile(
            &dir,
            "Brewfile",
            "brew \"ripgrep\"\nbrew \"fd\"\ncask \"iterm2\"\ntap \"homebrew/core\"\n",
        );
        let entries = parse_brewfile(&path).unwrap();
        assert!(entries.formulas.contains("ripgrep"));
        assert!(entries.formulas.contains("fd"));
        assert!(entries.casks.contains("iterm2"));
        // tap should not be collected
        assert!(!entries.formulas.contains("homebrew/core"));
        assert!(!entries.casks.contains("homebrew/core"));
    }

    #[test]
    fn parse_brewfile_skips_comments_and_blank_lines() {
        let dir = TempDir::new().unwrap();
        let path = write_brewfile(
            &dir,
            "Brewfile",
            "# this is a comment\n\nbrew \"ripgrep\"\n",
        );
        let entries = parse_brewfile(&path).unwrap();
        assert_eq!(entries.formulas.len(), 1);
        assert!(entries.formulas.contains("ripgrep"));
    }

    #[test]
    fn parse_brewfile_returns_empty_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile.nonexistent");
        let entries = parse_brewfile(&path).unwrap();
        assert!(entries.formulas.is_empty());
        assert!(entries.casks.is_empty());
    }

    #[test]
    fn collect_brewfile_entries_unions_multiple_files() {
        let dir = TempDir::new().unwrap();
        let a = write_brewfile(&dir, "Brewfile", "brew \"ripgrep\"\ncask \"iterm2\"\n");
        let b = write_brewfile(&dir, "Brewfile.work", "brew \"gh\"\ncask \"slack\"\n");
        let entries = collect_brewfile_entries(&[a.as_path(), b.as_path()]).unwrap();
        assert!(entries.formulas.contains("ripgrep"));
        assert!(entries.formulas.contains("gh"));
        assert!(entries.casks.contains("iterm2"));
        assert!(entries.casks.contains("slack"));
    }

    #[test]
    fn collect_brewfile_entries_deduplicates_across_files() {
        let dir = TempDir::new().unwrap();
        let a = write_brewfile(&dir, "Brewfile", "brew \"ripgrep\"\n");
        let b = write_brewfile(&dir, "Brewfile.extra", "brew \"ripgrep\"\nbrew \"fd\"\n");
        let entries = collect_brewfile_entries(&[a.as_path(), b.as_path()]).unwrap();
        assert_eq!(entries.formulas.len(), 2);
    }
}
