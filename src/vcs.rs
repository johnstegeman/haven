/// VCS backend selection: git or jj (Jujutsu colocated).
///
/// Resolution order (first match wins):
///   1. `--vcs` CLI flag
///   2. `DFILES_VCS` env var
///   3. `vcs.backend` in `haven.toml`
///   4. Interactive detection prompt (jj on PATH, nothing set)
///   5. Default: git
///
/// For colocated jj repos: only the initial clone uses `jj git clone --colocate`.
/// Subsequent operations (pull, remote detection, status checks) use git directly,
/// because git commands continue to work in colocated repos.
use anyhow::{bail, Context, Result};
use std::io::{self, Write};
use std::path::Path;

// ─── Public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcsBackend {
    Git,
    Jj,
}

/// How the active backend was determined (for `haven vcs` display).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcsResolutionSource {
    /// `--vcs` CLI flag
    CliFlag,
    /// `DFILES_VCS` environment variable
    EnvVar,
    /// `vcs.backend` in `haven.toml`
    Config,
    /// jj detected on PATH, user chose via interactive prompt
    Detected,
    /// Default (no jj found or nothing set)
    Default,
}

#[derive(Debug, Clone)]
pub struct ResolvedVcs {
    pub backend: VcsBackend,
    pub source: VcsResolutionSource,
    /// When true the caller should persist `backend` to `haven.toml`.
    pub save_to_config: bool,
}

/// Result of the interactive "jj detected, which VCS?" prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcsPromptResult {
    /// Use jj for this command only.
    UseJj,
    /// Use git for this command only.
    UseGit,
    /// Use jj and save `vcs.backend = "jj"` to `haven.toml`.
    SaveJj,
    /// Use git and save `vcs.backend = "git"` to `haven.toml`.
    SaveGit,
    /// Abort — user will set config manually.
    Abort,
}

// ─── Resolution ───────────────────────────────────────────────────────────────

/// Pure resolution from already-evaluated inputs. No I/O. Testable.
///
/// `prompt_result` is `Some(_)` only when the interactive prompt was already
/// shown by the caller (i.e. `jj_on_path` is true and no earlier source matched).
pub fn resolve_from_parts(
    cli_flag: Option<VcsBackend>,
    env_var: Option<VcsBackend>,
    config_backend: Option<VcsBackend>,
    jj_on_path: bool,
    prompt_result: Option<VcsPromptResult>,
) -> Option<ResolvedVcs> {
    if let Some(b) = cli_flag {
        return Some(ResolvedVcs { backend: b, source: VcsResolutionSource::CliFlag, save_to_config: false });
    }
    if let Some(b) = env_var {
        return Some(ResolvedVcs { backend: b, source: VcsResolutionSource::EnvVar, save_to_config: false });
    }
    if let Some(b) = config_backend {
        return Some(ResolvedVcs { backend: b, source: VcsResolutionSource::Config, save_to_config: false });
    }
    if jj_on_path {
        if let Some(pr) = prompt_result {
            return match pr {
                VcsPromptResult::UseJj => Some(ResolvedVcs { backend: VcsBackend::Jj, source: VcsResolutionSource::Detected, save_to_config: false }),
                VcsPromptResult::UseGit => Some(ResolvedVcs { backend: VcsBackend::Git, source: VcsResolutionSource::Detected, save_to_config: false }),
                VcsPromptResult::SaveJj => Some(ResolvedVcs { backend: VcsBackend::Jj, source: VcsResolutionSource::Detected, save_to_config: true }),
                VcsPromptResult::SaveGit => Some(ResolvedVcs { backend: VcsBackend::Git, source: VcsResolutionSource::Detected, save_to_config: true }),
                VcsPromptResult::Abort => None, // caller should exit
            };
        }
    }
    // Default: git
    Some(ResolvedVcs { backend: VcsBackend::Git, source: VcsResolutionSource::Default, save_to_config: false })
}

/// Full resolution: reads env var, config, detects jj, shows prompt if needed.
///
/// Returns `None` if the user chose to abort at the detection prompt.
pub fn resolve(
    cli_flag: Option<VcsBackend>,
    config_backend: Option<VcsBackend>,
    repo_root: Option<&Path>, // None during `haven init` (config not yet written)
) -> Result<Option<ResolvedVcs>> {
    let env_var = parse_vcs_env()?;
    let on_path = jj_on_path();

    // If nothing is set and jj is available, show detection prompt.
    let prompt_result = if cli_flag.is_none() && env_var.is_none() && config_backend.is_none() && on_path {
        let pr = prompt_vcs_backend()?;
        if let VcsPromptResult::SaveJj | VcsPromptResult::SaveGit = pr {
            // Caller handles the config save.
        }
        Some(pr)
    } else {
        None
    };

    let resolved = resolve_from_parts(cli_flag, env_var, config_backend, on_path, prompt_result);

    // If user chose to save, persist the choice to haven.toml.
    if let Some(ref r) = resolved {
        if r.save_to_config {
            if let Some(root) = repo_root {
                save_vcs_to_config(root, r.backend)?;
            }
        }
    }

    Ok(resolved)
}

/// Parse the `DFILES_VCS` environment variable.
fn parse_vcs_env() -> Result<Option<VcsBackend>> {
    match std::env::var("HAVEN_VCS") {
        Ok(v) => match v.to_lowercase().as_str() {
            "jj" => Ok(Some(VcsBackend::Jj)),
            "git" => Ok(Some(VcsBackend::Git)),
            other => bail!("HAVEN_VCS: unknown value '{}'; use 'git' or 'jj'", other),
        },
        Err(_) => Ok(None),
    }
}

/// Returns true if `jj` is found on PATH (no subprocess — PATH check only).
pub fn jj_on_path() -> bool {
    std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path).any(|dir| {
                dir.join("jj").exists() || dir.join("jj.exe").exists()
            })
        })
        .unwrap_or(false)
}

/// Validate that jj is actually installed when the backend is jj.
/// Returns an error with a clear message if it isn't.
pub fn check_jj_installed() -> Result<()> {
    if !jj_on_path() {
        bail!(
            "vcs backend is set to 'jj' but jj is not installed or not on PATH.\n\
             Install jj (https://jj-vcs.github.io/jj/latest/install-and-setup/) or \
             set vcs.backend = \"git\" in haven.toml."
        );
    }
    Ok(())
}

/// Interactive prompt: jj is installed but no backend is configured.
///
///   j) jj  (use once)
///   g) git (use once)
///   J) jj  (save vcs.backend = "jj" to haven.toml)
///   G) git (save vcs.backend = "git" to haven.toml)
///   q) abort
fn prompt_vcs_backend() -> Result<VcsPromptResult> {
    println!("jj (Jujutsu) is installed but no VCS backend is configured.");
    println!();
    println!("Which VCS should haven use for new repos and clones?");
    println!();
    println!("  j) Use jj this time");
    println!("  g) Use git this time");
    println!("  J) Use jj and save to haven.toml  (vcs.backend = \"jj\")");
    println!("  G) Use git and save to haven.toml (vcs.backend = \"git\")");
    println!("  q) Abort — I'll set vcs.backend in haven.toml manually");
    println!();

    loop {
        print!("[j/g/J/G/q]: ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        match line.trim() {
            "j" => return Ok(VcsPromptResult::UseJj),
            "g" => return Ok(VcsPromptResult::UseGit),
            "J" => return Ok(VcsPromptResult::SaveJj),
            "G" => return Ok(VcsPromptResult::SaveGit),
            "q" | "" => return Ok(VcsPromptResult::Abort),
            _ => {
                print!("Please enter j, g, J, G, or q: ");
                io::stdout().flush()?;
            }
        }
    }
}

/// Write `vcs.backend = "<backend>"` into `haven.toml`.
/// Appends a `[vcs]` section if one doesn't already exist; updates in-place if it does.
fn save_vcs_to_config(repo_root: &Path, backend: VcsBackend) -> Result<()> {
    let path = repo_root.join("haven.toml");
    if !path.exists() {
        // Config doesn't exist yet (init case) — nothing to save.
        return Ok(());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Cannot read {}", path.display()))?;

    let value = match backend { VcsBackend::Git => "git", VcsBackend::Jj => "jj" };

    let updated = if content.contains("[vcs]") {
        // Replace existing backend line, or insert after [vcs].
        let mut lines: Vec<&str> = content.lines().collect();
        let mut found = false;
        let mut in_vcs = false;
        for line in &mut lines {
            if line.trim() == "[vcs]" { in_vcs = true; continue; }
            if in_vcs && line.trim_start().starts_with('[') { in_vcs = false; }
            if in_vcs && line.trim_start().starts_with("backend") {
                // We need to replace this line — use a sentinel approach below.
                found = true;
            }
        }
        if found {
            // Replace the backend line.
            content.lines()
                .map(|l| {
                    if in_vcs_section_backend_line(l) {
                        format!("backend = \"{}\"", value)
                    } else {
                        l.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n") + if content.ends_with('\n') { "\n" } else { "" }
        } else {
            // [vcs] exists but no backend line — append after [vcs].
            insert_after_vcs_header(&content, value)
        }
    } else {
        // Append [vcs] section.
        let sep = if content.ends_with('\n') { "" } else { "\n" };
        format!("{}{}[vcs]\nbackend = \"{}\"\n", content, sep, value)
    };

    std::fs::write(&path, updated)
        .with_context(|| format!("Cannot write {}", path.display()))?;

    println!("  Saved vcs.backend = \"{}\" to haven.toml", value);
    Ok(())
}

fn in_vcs_section_backend_line(line: &str) -> bool {
    // This is a simple heuristic — good enough for the well-structured toml haven writes.
    line.trim_start().starts_with("backend")
}

fn insert_after_vcs_header(content: &str, value: &str) -> String {
    let mut result = String::new();
    let mut inserted = false;
    for line in content.lines() {
        result.push_str(line);
        result.push('\n');
        if !inserted && line.trim() == "[vcs]" {
            result.push_str(&format!("backend = \"{}\"\n", value));
            inserted = true;
        }
    }
    result
}

// ─── Clone ────────────────────────────────────────────────────────────────────

/// Clone a repository using the given VCS backend.
///
/// For jj: `jj git clone --colocate [--depth N] [--branch b] url dest`
/// For git: `git clone [--depth N] [--branch b] url dest`
pub fn clone_repo(
    backend: VcsBackend,
    url: &str,
    dest: &Path,
    depth: Option<u32>,
    branch: Option<&str>,
) -> Result<()> {
    match backend {
        VcsBackend::Git => {
            let mut cmd = std::process::Command::new("git");
            cmd.arg("clone");
            if let Some(d) = depth {
                cmd.args(["--depth", &d.to_string()]);
            }
            if let Some(b) = branch {
                cmd.args(["--branch", b]);
            }
            cmd.arg(url).arg(dest);
            let status = cmd
                .status()
                .context("Failed to run `git clone`. Is git installed and in your PATH?")?;
            if !status.success() {
                bail!("git clone failed for {}", url);
            }
        }
        VcsBackend::Jj => {
            check_jj_installed()?;
            let mut cmd = std::process::Command::new("jj");
            cmd.args(["git", "clone", "--colocate"]);
            if let Some(d) = depth {
                // jj git clone supports --depth for shallow clones.
                cmd.args(["--depth", &d.to_string()]);
            }
            if let Some(b) = branch {
                cmd.args(["--branch", b]);
            }
            cmd.arg(url).arg(dest);
            let status = cmd
                .status()
                .context("Failed to run `jj git clone`. Is jj installed and in your PATH?")?;
            if !status.success() {
                bail!("jj git clone failed for {}", url);
            }
        }
    }
    Ok(())
}

// ─── Migration ────────────────────────────────────────────────────────────────

/// Outcome returned by [`ensure_colocated`] so callers can track session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrateOutcome {
    /// `.jj/` was already present — nothing done.
    AlreadyColocated,
    /// Migrated this directory (user chose "y").
    Migrated,
    /// Migrated this directory *and* user chose "always" — caller should pass
    /// `migrate_all = true` for all subsequent calls this session.
    MigratedAll,
    /// User declined migration for this directory.
    Skipped,
}

/// Ensure a directory is a jj colocated repo when backend is jj.
///
/// If `.jj/` is already present, returns [`MigrateOutcome::AlreadyColocated`].
/// Otherwise, prompts the user to run `jj git init --colocate`.
///
/// When `migrate_all` is true the prompt is skipped and migration is applied
/// immediately (the caller previously chose "always" for this session).
pub fn ensure_colocated(dir: &Path, migrate_all: bool) -> Result<MigrateOutcome> {
    if dir.join(".jj").exists() {
        return Ok(MigrateOutcome::AlreadyColocated);
    }

    let dest_display = crate::fs::tilde_path(dir);

    let mut said_always = false;

    if !migrate_all {
        println!(
            "{} is a plain git repo (no .jj/). \
             Run `jj git init --colocate` to enable jj here?",
            dest_display,
        );
        println!();
        println!("  y)      Migrate this directory");
        println!("  always) Migrate all such directories this session");
        println!("  N)      Skip — keep using git for this directory");
        println!();
        print!("[y/always/N]: ");
        io::stdout().flush()?;

        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        match line.trim().to_lowercase().as_str() {
            "y" | "yes" => {} // fall through to migrate
            "always" => { said_always = true; } // migrate this one, caller sets flag
            _ => {
                println!("  Skipping jj migration for {}", dest_display);
                return Ok(MigrateOutcome::Skipped);
            }
        }
    }

    println!("  Running `jj git init --colocate` in {}…", dest_display);
    let status = std::process::Command::new("jj")
        .args(["git", "init", "--colocate"])
        .current_dir(dir)
        .status()
        .context("Failed to run `jj git init --colocate`. Is jj installed?")?;
    if !status.success() {
        bail!("jj git init --colocate failed in {}", dir.display());
    }
    println!("  ✓  {} is now a jj colocated repo", dest_display);
    Ok(if said_always { MigrateOutcome::MigratedAll } else { MigrateOutcome::Migrated })
}

// ─── Status display ───────────────────────────────────────────────────────────

/// Print a summary of the active VCS backend for `haven vcs`.
pub fn print_status(resolved: &ResolvedVcs, repo_root: &Path) {
    let backend_str = match resolved.backend {
        VcsBackend::Git => "git",
        VcsBackend::Jj  => "jj (colocated)",
    };
    let source_str = match resolved.source {
        VcsResolutionSource::CliFlag  => "  (set via --vcs flag)",
        VcsResolutionSource::EnvVar   => "  (set via HAVEN_VCS env var)",
        VcsResolutionSource::Config   => "  (set in haven.toml [vcs])",
        VcsResolutionSource::Detected => "  (detected — jj on PATH)",
        VcsResolutionSource::Default  => "  (default)",
    };
    println!("VCS backend: {}{}", backend_str, source_str);

    let jj_str = if jj_on_path() { "installed" } else { "not found" };
    println!("jj:          {}", jj_str);

    let config_path = repo_root.join("haven.toml");
    if config_path.exists() {
        println!("haven.toml: {}", config_path.display());
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn git() -> Option<VcsBackend> { Some(VcsBackend::Git) }
    fn jj() -> Option<VcsBackend> { Some(VcsBackend::Jj) }

    #[test]
    fn cli_flag_overrides_all() {
        let r = resolve_from_parts(git(), jj(), jj(), true, Some(VcsPromptResult::UseJj)).unwrap();
        assert_eq!(r.backend, VcsBackend::Git);
        assert_eq!(r.source, VcsResolutionSource::CliFlag);
    }

    #[test]
    fn env_var_overrides_config_and_prompt() {
        let r = resolve_from_parts(None, jj(), git(), true, Some(VcsPromptResult::UseGit)).unwrap();
        assert_eq!(r.backend, VcsBackend::Jj);
        assert_eq!(r.source, VcsResolutionSource::EnvVar);
    }

    #[test]
    fn config_used_when_no_flag_or_env() {
        let r = resolve_from_parts(None, None, jj(), false, None).unwrap();
        assert_eq!(r.backend, VcsBackend::Jj);
        assert_eq!(r.source, VcsResolutionSource::Config);
        assert!(!r.save_to_config);
    }

    #[test]
    fn prompt_use_jj_once() {
        let r = resolve_from_parts(None, None, None, true, Some(VcsPromptResult::UseJj)).unwrap();
        assert_eq!(r.backend, VcsBackend::Jj);
        assert!(!r.save_to_config);
    }

    #[test]
    fn prompt_save_jj_sets_flag() {
        let r = resolve_from_parts(None, None, None, true, Some(VcsPromptResult::SaveJj)).unwrap();
        assert_eq!(r.backend, VcsBackend::Jj);
        assert!(r.save_to_config);
    }

    #[test]
    fn prompt_save_git_sets_flag() {
        let r = resolve_from_parts(None, None, None, true, Some(VcsPromptResult::SaveGit)).unwrap();
        assert_eq!(r.backend, VcsBackend::Git);
        assert!(r.save_to_config);
    }

    #[test]
    fn prompt_abort_returns_none() {
        let r = resolve_from_parts(None, None, None, true, Some(VcsPromptResult::Abort));
        assert!(r.is_none());
    }

    #[test]
    fn no_jj_on_path_defaults_to_git() {
        let r = resolve_from_parts(None, None, None, false, None).unwrap();
        assert_eq!(r.backend, VcsBackend::Git);
        assert_eq!(r.source, VcsResolutionSource::Default);
    }

    #[test]
    fn jj_on_path_but_no_prompt_given_defaults_to_git() {
        // When jj is on PATH but no prompt_result provided (caller chose not to prompt),
        // we still fall through to default.
        let r = resolve_from_parts(None, None, None, true, None).unwrap();
        assert_eq!(r.backend, VcsBackend::Git);
        assert_eq!(r.source, VcsResolutionSource::Default);
    }

    #[test]
    fn cli_flag_git_overrides_env_jj() {
        let r = resolve_from_parts(git(), jj(), None, false, None).unwrap();
        assert_eq!(r.backend, VcsBackend::Git);
        assert_eq!(r.source, VcsResolutionSource::CliFlag);
    }

    #[test]
    fn config_git_when_no_flag_env() {
        let r = resolve_from_parts(None, None, git(), true, Some(VcsPromptResult::UseJj)).unwrap();
        assert_eq!(r.backend, VcsBackend::Git);
        assert_eq!(r.source, VcsResolutionSource::Config);
    }
}
