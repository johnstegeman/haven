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
///
/// Stdout is filtered: lines starting with "Using " (already-installed packages)
/// are suppressed. Everything else (Installing, Upgrading, Tapping, summaries,
/// errors) is printed so the user only sees meaningful output.
pub fn bundle_install(brew: &Path, brewfile: &Path) -> Result<()> {
    use std::io::BufRead;

    let mut child = std::process::Command::new(brew)
        .args(["bundle", "install", "--file"])
        .arg(brewfile)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .with_context(|| {
            format!(
                "Cannot run `brew bundle install` for {}",
                brewfile.display()
            )
        })?;

    if let Some(stdout) = child.stdout.take() {
        for line in std::io::BufReader::new(stdout).lines() {
            let line = line.unwrap_or_default();
            if !line.starts_with("Using ") {
                println!("{}", line);
            }
        }
    }

    let status = child.wait().with_context(|| {
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
pub fn brew_uninstall(brew: &Path, name: &str, cask: bool, zap: bool) -> Result<()> {
    let mut cmd = std::process::Command::new(brew);
    cmd.arg("uninstall");
    if cask {
        cmd.arg("--cask");
        if zap {
            cmd.arg("--zap");
        }
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
    //
    // `brew list --formula` returns short names for tap formulae (e.g. `qmk`
    // rather than `qmk/qmk/qmk`).  A declared formula is considered installed
    // if either its full name or its short name appears in the installed set.
    let mut missing_formulas: Vec<String> = declared
        .formulas
        .iter()
        .filter(|f| {
            if all_installed_formulas.contains(*f) {
                return false;
            }
            let short = f.rsplit('/').next().unwrap_or(f.as_str());
            !all_installed_formulas.contains(short)
        })
        .cloned()
        .collect();
    let mut missing_casks: Vec<String> = declared
        .casks
        .iter()
        .filter(|c| !all_installed_casks.contains(*c))
        .cloned()
        .collect();

    // Installed but not declared (leaves only for formulas).
    //
    // `brew leaves` returns tap-qualified names for non-core formulae
    // (e.g. `qmk/qmk/qmk`) while Brewfiles often use the short form (`qmk`).
    // A leaf is considered declared if either the full name or the short name
    // (segment after the last `/`) appears in the declared set.
    let leaves = brew_leaves(brew)?;
    let mut extra_formulas: Vec<String> = leaves
        .into_iter()
        .filter(|f| {
            if declared.formulas.contains(f) {
                return false;
            }
            // Also check the short name (e.g. "qmk" for "qmk/qmk/qmk").
            let short = f.rsplit('/').next().unwrap_or(f.as_str());
            !declared.formulas.contains(short)
        })
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
///
/// The new entry is inserted immediately after the last existing line of the
/// same kind (tap/brew/cask), so it stays within its natural section rather
/// than being appended to the end of the file (which would place it after
/// entries of other kinds and break section-aware sorting).
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

    let new_entry = format!("{} \"{}\"\n", kind, name);
    let new_content = insert_after_last_of_kind(existing, kind, &new_entry);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &new_content)
        .with_context(|| format!("Cannot write {}", path.display()))?;

    Ok(true)
}

/// Insert `new_entry` immediately after the last line in `content` whose
/// trimmed form starts with `"{kind} "`.  If no such line exists, append at
/// the end (with a newline separator when needed).
fn insert_after_last_of_kind(content: String, kind: &str, new_entry: &str) -> String {
    let prefix = format!("{} ", kind);

    // Walk the '\n'-split chunks and track where each kind-line ends.
    // `split('\n')` yields chunks without the '\n' separator; the '\n'
    // itself lives at byte offset `byte_offset + chunk.len()`.
    let mut last_insert_at: Option<usize> = None;
    let mut byte_offset: usize = 0;

    for chunk in content.split('\n') {
        if chunk.trim_start().starts_with(&prefix) {
            // Insert point is right after the '\n' that follows this chunk.
            last_insert_at = Some(byte_offset + chunk.len() + 1);
        }
        byte_offset += chunk.len() + 1;
    }

    if let Some(insert_at) = last_insert_at {
        // Clamp to content length (handles files that don't end with '\n').
        let insert_at = insert_at.min(content.len());
        let mut result = String::with_capacity(content.len() + new_entry.len() + 1);
        result.push_str(&content[..insert_at]);
        // If we landed exactly at EOF and there is no trailing newline, add one.
        if insert_at == content.len() && !content.ends_with('\n') {
            result.push('\n');
        }
        result.push_str(new_entry);
        result.push_str(&content[insert_at..]);
        result
    } else {
        // No existing line of this kind — fall back to appending.
        let mut result = content;
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str(new_entry);
        result
    }
}

/// Sort the entries in a Brewfile alphabetically by name.
///
/// Each kind (tap, brew, cask) is sorted independently, so existing section
/// groupings (e.g. taps first, then formulas, then casks) are preserved.
/// Comments, blank lines, and any other non-formula lines are not moved.
/// The file is only written if the content would actually change.
///
/// Sort key: the short name (last `/`-separated component), lowercased.
/// E.g. `tap "homebrew/cask-fonts"` sorts under `"cask-fonts"`.
pub fn sort_brewfile(path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read {}", path.display()))?;

    // Collect formula lines per kind for independent sorting.
    let mut tap_lines: Vec<&str> = Vec::new();
    let mut brew_lines: Vec<&str> = Vec::new();
    let mut cask_lines: Vec<&str> = Vec::new();

    for line in text.split('\n') {
        let t = line.trim();
        if t.starts_with("tap ") {
            tap_lines.push(line);
        } else if t.starts_with("brew ") {
            brew_lines.push(line);
        } else if t.starts_with("cask ") {
            cask_lines.push(line);
        }
    }

    tap_lines.sort_by(|a, b| brewfile_entry_sort_key(a, "tap").cmp(&brewfile_entry_sort_key(b, "tap")));
    brew_lines.sort_by(|a, b| brewfile_entry_sort_key(a, "brew").cmp(&brewfile_entry_sort_key(b, "brew")));
    cask_lines.sort_by(|a, b| brewfile_entry_sort_key(a, "cask").cmp(&brewfile_entry_sort_key(b, "cask")));

    let mut tap_iter = tap_lines.iter();
    let mut brew_iter = brew_lines.iter();
    let mut cask_iter = cask_lines.iter();

    let parts: Vec<&str> = text
        .split('\n')
        .map(|line| {
            let t = line.trim();
            if t.starts_with("tap ") {
                *tap_iter.next().unwrap()
            } else if t.starts_with("brew ") {
                *brew_iter.next().unwrap()
            } else if t.starts_with("cask ") {
                *cask_iter.next().unwrap()
            } else {
                line
            }
        })
        .collect();

    let new_content = parts.join("\n");
    if new_content != text {
        std::fs::write(path, &new_content)
            .with_context(|| format!("Cannot write {}", path.display()))?;
    }

    Ok(())
}

/// Sort key for a Brewfile entry line: the short name (last `/`-component), lowercased.
fn brewfile_entry_sort_key(line: &str, kind: &str) -> String {
    extract_entry_name(line.trim(), kind)
        .map(|name| name.split('/').last().unwrap_or(&name).to_lowercase())
        .unwrap_or_else(|| line.to_lowercase())
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

    // Split on '\n' rather than .lines() so the trailing "" from a terminal newline
    // is preserved in the output when we rejoin: ["a", ""].join("\n") == "a\n".
    let mut removed = 0usize;
    let mut kept: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        if brewfile_line_matches(line, kind, name) {
            removed += 1;
        } else {
            kept.push(line);
        }
    }

    if removed > 0 {
        let new_content = kept.join("\n");
        if new_content.is_empty() {
            std::fs::remove_file(path)
                .with_context(|| format!("Cannot remove {}", path.display()))?;
        } else {
            std::fs::write(path, new_content)
                .with_context(|| format!("Cannot write {}", path.display()))?;
        }
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

    // sort_brewfile tests

    #[test]
    fn test_sort_brewfile_already_sorted_is_noop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");
        let content = "brew \"bat\"\nbrew \"fd\"\nbrew \"ripgrep\"\n";
        std::fs::write(&path, content).unwrap();
        let mtime_before = std::fs::metadata(&path).unwrap().modified().unwrap();

        sort_brewfile(&path).unwrap();

        // File should not have been rewritten (mtime unchanged)
        let mtime_after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
    }

    #[test]
    fn test_sort_brewfile_sorts_brew_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");
        std::fs::write(&path, "brew \"zsh\"\nbrew \"bat\"\nbrew \"fd\"\n").unwrap();

        sort_brewfile(&path).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "brew \"bat\"\nbrew \"fd\"\nbrew \"zsh\"\n"
        );
    }

    #[test]
    fn test_sort_brewfile_sorts_cask_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");
        std::fs::write(&path, "cask \"visual-studio-code\"\ncask \"1password\"\ncask \"iterm2\"\n").unwrap();

        sort_brewfile(&path).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "cask \"1password\"\ncask \"iterm2\"\ncask \"visual-studio-code\"\n"
        );
    }

    #[test]
    fn test_sort_brewfile_sorts_tap_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");
        std::fs::write(&path, "tap \"homebrew/cask\"\ntap \"homebrew/core\"\ntap \"apple/apple\"\n").unwrap();

        sort_brewfile(&path).unwrap();

        // Sorted by short name: apple, cask, core
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "tap \"apple/apple\"\ntap \"homebrew/cask\"\ntap \"homebrew/core\"\n"
        );
    }

    #[test]
    fn test_sort_brewfile_preserves_blank_lines_and_comments() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");
        let content = "# Taps\ntap \"homebrew/cask\"\ntap \"apple/apple\"\n\n# Formulas\nbrew \"zsh\"\nbrew \"bat\"\n";
        std::fs::write(&path, content).unwrap();

        sort_brewfile(&path).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# Taps\ntap \"apple/apple\"\ntap \"homebrew/cask\"\n\n# Formulas\nbrew \"bat\"\nbrew \"zsh\"\n"
        );
    }

    #[test]
    fn test_sort_brewfile_sorts_by_short_name_not_tap_prefix() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");
        // "homebrew/cask-fonts" short name is "cask-fonts", sorts before "core"
        std::fs::write(&path, "tap \"homebrew/core\"\ntap \"homebrew/cask-fonts\"\n").unwrap();

        sort_brewfile(&path).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "tap \"homebrew/cask-fonts\"\ntap \"homebrew/core\"\n"
        );
    }

    #[test]
    fn test_sort_brewfile_case_insensitive() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");
        std::fs::write(&path, "brew \"Zsh\"\nbrew \"bat\"\nbrew \"Bat\"\n").unwrap();

        sort_brewfile(&path).unwrap();

        // "Bat", "bat", "Zsh" — case-insensitive, stable for ties
        let result = std::fs::read_to_string(&path).unwrap();
        let first = result.lines().next().unwrap();
        assert!(first.contains("Bat") || first.contains("bat"), "first entry should be a 'bat' variant, got: {first}");
    }

    #[test]
    fn test_sort_brewfile_each_kind_sorted_independently() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");
        // Mixed file: brew positions stay brew, cask positions stay cask
        let content = "brew \"zsh\"\nbrew \"bat\"\ncask \"notion\"\ncask \"1password\"\n";
        std::fs::write(&path, content).unwrap();

        sort_brewfile(&path).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "brew \"bat\"\nbrew \"zsh\"\ncask \"1password\"\ncask \"notion\"\n"
        );
    }

    // add_to_brewfile tests

    #[test]
    fn test_add_to_brewfile_creates_new_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        // File doesn't exist, should create it
        let added = add_to_brewfile(&path, "brew", "neovim").unwrap();

        assert!(added);
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "brew \"neovim\"\n");
    }

    #[test]
    fn test_add_to_brewfile_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        // First add
        let added1 = add_to_brewfile(&path, "brew", "neovim").unwrap();
        assert!(added1);

        // Second add should be idempotent
        let added2 = add_to_brewfile(&path, "brew", "neovim").unwrap();

        assert!(!added2); // Already exists

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "brew \"neovim\"\n");
    }

    #[test]
    fn test_add_to_brewfile_preserves_existing_content() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        // Create initial file
        std::fs::write(&path, "brew \"existing\"\n").unwrap();

        // Add new entry
        let added = add_to_brewfile(&path, "brew", "neovim").unwrap();
        assert!(added);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "brew \"existing\"\nbrew \"neovim\"\n");
    }

    #[test]
    fn test_add_to_brewfile_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("subdir/nested/Brewfile");

        // File doesn't exist, should create file and parent directories
        let added = add_to_brewfile(&path, "brew", "neovim").unwrap();
        assert!(added);

        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "brew \"neovim\"\n");
    }

    #[test]
    fn test_add_to_brewfile_with_cask() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        let added = add_to_brewfile(&path, "cask", "notion").unwrap();
        assert!(added);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "cask \"notion\"\n");
    }

    #[test]
    fn test_add_to_brewfile_with_tap() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        let added = add_to_brewfile(&path, "tap", "homebrew/brew").unwrap();
        assert!(added);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "tap \"homebrew/brew\"\n");
    }

    #[test]
    fn test_add_to_brewfile_no_trailing_newline_fixes_it() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        // Create file without trailing newline
        std::fs::write(&path, "brew \"existing\"").unwrap();

        // Add new entry - should ensure trailing newline
        let added = add_to_brewfile(&path, "brew", "neovim").unwrap();
        assert!(added);

        let content = std::fs::read_to_string(&path).unwrap();
        // Should have trailing newline after existing content and new entry
        assert_eq!(content, "brew \"existing\"\nbrew \"neovim\"\n");
    }

    #[test]
    fn test_add_to_brewfile_already_has_trailing_newline() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        // Create file with trailing newline
        std::fs::write(&path, "brew \"existing\"\n").unwrap();

        // Add new entry
        let added = add_to_brewfile(&path, "brew", "neovim").unwrap();
        assert!(added);

        let content = std::fs::read_to_string(&path).unwrap();
        // Should not duplicate trailing newlines
        assert_eq!(content, "brew \"existing\"\nbrew \"neovim\"\n");
    }

    #[test]
    fn test_add_to_brewfile_inserts_after_last_of_same_kind_not_at_eof() {
        // Regression: adding a cask to a file that already has casks followed by brew
        // lines must insert within the cask section, not at EOF (which would put it
        // after the brew lines and break sort_brewfile's in-place algorithm).
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");
        let content = "cask \"arc\"\ncask \"zed\"\nbrew \"bat\"\n";
        std::fs::write(&path, content).unwrap();

        let added = add_to_brewfile(&path, "cask", "notion").unwrap();
        assert!(added);

        let result = std::fs::read_to_string(&path).unwrap();
        // "notion" must appear before "brew \"bat\"", not after it
        let lines: Vec<&str> = result.lines().collect();
        let notion_pos = lines.iter().position(|l| l.contains("notion")).unwrap();
        let brew_pos = lines.iter().position(|l| l.starts_with("brew ")).unwrap();
        assert!(
            notion_pos < brew_pos,
            "cask \"notion\" (line {notion_pos}) should be before brew \"bat\" (line {brew_pos})"
        );
    }

    #[test]
    fn test_add_to_brewfile_new_kind_falls_back_to_append() {
        // When no existing line of the kind exists, fall back to appending.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");
        std::fs::write(&path, "brew \"bat\"\n").unwrap();

        let added = add_to_brewfile(&path, "cask", "notion").unwrap();
        assert!(added);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "brew \"bat\"\ncask \"notion\"\n");
    }

    // remove_from_brewfile tests

    #[test]
    fn test_remove_from_brewfile_no_file_returns_zero() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.txt");

        let removed = remove_from_brewfile(&path, "brew", "neovim").unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_remove_from_brewfile_removes_matching_entry() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        std::fs::write(&path, "brew \"neovim\"\n").unwrap();

        let removed = remove_from_brewfile(&path, "brew", "neovim").unwrap();
        assert_eq!(removed, 1);

        assert!(!path.exists());
    }

    #[test]
    fn test_remove_from_brewfile_partial_match() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        std::fs::write(&path, "brew \"neovim\"\nbrew \"nvim\"\n").unwrap();

        let removed = remove_from_brewfile(&path, "brew", "neovim").unwrap();
        assert_eq!(removed, 1);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "brew \"nvim\"\n");
    }

    #[test]
    fn test_remove_from_brewfile_no_match() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        std::fs::write(&path, "brew \"neovim\"\n").unwrap();

        let removed = remove_from_brewfile(&path, "brew", "nvim").unwrap();
        assert_eq!(removed, 0);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "brew \"neovim\"\n");
    }

    #[test]
    fn test_remove_from_brewfile_removes_cask() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        std::fs::write(&path, "cask \"notion\"\n").unwrap();

        let removed = remove_from_brewfile(&path, "cask", "notion").unwrap();
        assert_eq!(removed, 1);

        assert!(!path.exists());
    }

    #[test]
    fn test_remove_from_brewfile_removes_tap() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        std::fs::write(&path, "tap \"homebrew/core\"\n").unwrap();

        let removed = remove_from_brewfile(&path, "tap", "homebrew/core").unwrap();
        assert_eq!(removed, 1);

        assert!(!path.exists());
    }

    #[test]
    fn test_remove_from_brewfile_mixed_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        let content = "brew \"neovim\"\n\ncask \"notion\"\n\ntap \"homebrew/brew\"\n";
        std::fs::write(&path, content).unwrap();

        let removed = remove_from_brewfile(&path, "brew", "neovim").unwrap();
        assert_eq!(removed, 1);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "\ncask \"notion\"\n\ntap \"homebrew/brew\"\n");
    }

    #[test]
    fn test_remove_from_brewfile_multiple_brews_same_name() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        // Duplicate entries (shouldn't happen normally but test idempotency)
        std::fs::write(&path, "brew \"neovim\"\nbrew \"neovim\"\n").unwrap();

        let removed = remove_from_brewfile(&path, "brew", "neovim").unwrap();
        // Removes all matching lines
        assert_eq!(removed, 2);

        assert!(!path.exists());
    }

    #[test]
    fn test_remove_from_brewfile_removes_only_matching_kind() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        let content = "brew \"neovim\"\ncask \"notion\"\n";
        std::fs::write(&path, content).unwrap();

        // Try to remove as brew - should only remove brew entry
        let removed = remove_from_brewfile(&path, "brew", "neovim").unwrap();
        assert_eq!(removed, 1);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "cask \"notion\"\n");
    }

    #[test]
    fn test_remove_from_brewfile_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        std::fs::write(&path, "").unwrap();

        let removed = remove_from_brewfile(&path, "brew", "neovim").unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_remove_from_brewfile_preserves_empty_lines() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        let content = "brew \"neovim\"\n\n\n";
        std::fs::write(&path, content).unwrap();

        let removed = remove_from_brewfile(&path, "brew", "neovim").unwrap();
        assert_eq!(removed, 1);

        let result = std::fs::read_to_string(&path).unwrap();
        // split('\n') on "brew\n\n\n" → ["brew", "", "", ""], remove "brew" → ["", "", ""]
        // joined: "\n\n" (2 separators between 3 empty strings)
        assert_eq!(result, "\n\n");
    }

    #[test]
    fn test_remove_from_brewfile_preserves_comments() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        let content = "# This is a comment\nbrew \"neovim\"\n# Another comment\n";
        std::fs::write(&path, content).unwrap();

        let removed = remove_from_brewfile(&path, "brew", "neovim").unwrap();
        assert_eq!(removed, 1);

        let result = std::fs::read_to_string(&path).unwrap();
        assert_eq!(result, "# This is a comment\n# Another comment\n");
    }

    #[test]
    fn test_remove_from_brewfile_preserves_other_kinds() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Brewfile");

        let content = "brew \"neovim\"\n\n# Comment\nbrew \"nvim\"\ncask \"notion\"\n";
        std::fs::write(&path, content).unwrap();

        let removed = remove_from_brewfile(&path, "brew", "neovim").unwrap();
        assert_eq!(removed, 1);

        let result = std::fs::read_to_string(&path).unwrap();
        assert_eq!(result, "\n# Comment\nbrew \"nvim\"\ncask \"notion\"\n");
    }

    // brewfile_line_matches tests

    #[test]
    fn test_brewfile_line_matches_brew() {
        assert!(brewfile_line_matches("brew \"neovim\"", "brew", "neovim"));
        assert!(!brewfile_line_matches("brew \"nvim\"", "brew", "neovim"));
        assert!(!brewfile_line_matches("cask \"neovim\"", "brew", "neovim"));
        assert!(!brewfile_line_matches("tap \"neovim\"", "brew", "neovim"));
    }

    #[test]
    fn test_brewfile_line_matches_cask() {
        assert!(brewfile_line_matches("cask \"notion\"", "cask", "notion"));
        assert!(!brewfile_line_matches("brew \"notion\"", "cask", "notion"));
    }

    #[test]
    fn test_brewfile_line_matches_tap() {
        assert!(brewfile_line_matches("tap \"homebrew/brew\"", "tap", "homebrew/brew"));
        assert!(!brewfile_line_matches("brew \"homebrew/brew\"", "tap", "homebrew/brew"));
    }

    #[test]
    fn test_brewfile_line_matches_ignores_leading_whitespace() {
        assert!(brewfile_line_matches("  brew \"neovim\"", "brew", "neovim"));
        assert!(brewfile_line_matches("\tbrew \"neovim\"", "brew", "neovim"));
    }

    #[test]
    fn test_brewfile_line_matches_ignores_trailing_whitespace() {
        assert!(brewfile_line_matches("brew \"neovim\"  ", "brew", "neovim"));
        assert!(brewfile_line_matches("brew \"neovim\"\t", "brew", "neovim"));
    }

    #[test]
    fn test_brewfile_line_matches_single_quotes() {
        // Brewfile supports single-quoted strings
        assert!(brewfile_line_matches("brew 'neovim'", "brew", "neovim"));
        assert!(!brewfile_line_matches("brew 'notion'", "cask", "notion"));
    }

    #[test]
    fn test_brewfile_line_matches_no_quotes_returns_false() {
        // Lines without quotes should not match
        assert!(!brewfile_line_matches("brew neovim", "brew", "neovim"));
        assert!(!brewfile_line_matches("brew neovim", "brew", "nvim"));
    }

    #[test]
    fn test_brewfile_line_matches_different_kinds() {
        assert!(!brewfile_line_matches("brew \"neovim\"", "cask", "neovim"));
        assert!(!brewfile_line_matches("cask \"notion\"", "brew", "notion"));
        assert!(!brewfile_line_matches("tap \"homebrew\"", "brew", "homebrew"));
    }
}
