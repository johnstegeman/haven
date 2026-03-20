/// Mise (https://mise.jdx.dev) integration: detection and tool installation.
///
/// Detection order:
///   1. `mise` in PATH
///   2. `~/.local/bin/mise` (default mise self-install location)
///
/// Install flow:
///   If mise is absent, prints a one-line hint — mise install is intentionally
///   left to the user for now (unlike Homebrew, there's no single canonical
///   installer URL that works well non-interactively on all platforms).
///
/// Tool installation:
///   `mise install` (reads the config file or nearest .mise.toml / .tool-versions)
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Find the `mise` binary. Checks PATH first, then `~/.local/bin/mise`.
pub fn mise_path() -> Option<PathBuf> {
    // PATH lookup via `which`.
    if let Ok(out) = std::process::Command::new("which").arg("mise").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
    }
    // Default self-install location.
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".local").join("bin").join("mise");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Run `mise install` using the given config file (passed via `MISE_CONFIG_FILE` env).
///
/// If `config` is None, mise reads its default config from the working directory.
pub fn install_tools(mise: &Path, config: Option<&Path>) -> Result<()> {
    let mut cmd = std::process::Command::new(mise);
    cmd.arg("install");
    if let Some(cfg) = config {
        cmd.env("MISE_CONFIG_FILE", cfg);
    }
    cmd.stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    let status = cmd
        .status()
        .context("Cannot run `mise install`")?;

    if !status.success() {
        anyhow::bail!("`mise install` failed (exit {:?})", status.code());
    }
    Ok(())
}

/// Check whether all tools in the config are installed.
///
/// Runs `mise current` and checks exit code. Returns `false` on any error.
pub fn tools_installed(mise: &Path, config: Option<&Path>) -> bool {
    let mut cmd = std::process::Command::new(mise);
    cmd.arg("current");
    if let Some(cfg) = config {
        cmd.env("MISE_CONFIG_FILE", cfg);
    }
    cmd.output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
