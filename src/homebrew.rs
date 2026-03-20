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
///   apply  → `brew bundle install --file=<path> --no-lock`
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

/// Run `brew bundle install --file=<brewfile> --no-lock`.
///
/// `--no-lock` prevents Brewfile.lock.json from being created (we manage state ourselves).
pub fn bundle_install(brew: &Path, brewfile: &Path) -> Result<()> {
    let status = std::process::Command::new(brew)
        .args(["bundle", "install", "--no-lock", "--file"])
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
