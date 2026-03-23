use anyhow::{bail, Context, Result};
use std::path::Path;

use crate::commands::apply;
use crate::config::DfilesConfig;
use crate::github::GhSource;
use crate::vcs::{self, VcsBackend};

pub struct InitOptions<'a> {
    pub repo_root: &'a Path,
    /// Remote git URL or `gh:owner/repo[@ref]` to clone. `None` = blank scaffold.
    pub source: Option<&'a str>,
    /// Branch to clone. Overrides any `@ref` in the source string.
    pub branch: Option<&'a str>,
    /// Apply the cloned repo after cloning.
    pub apply: bool,
    /// Profile to apply. `None` means not explicitly given (defaults to "default" when applying).
    pub profile: Option<&'a str>,
    // Fields required for the optional apply step.
    pub dest_root: &'a Path,
    pub backup_dir: &'a Path,
    pub state_dir: &'a Path,
    pub claude_dir: &'a Path,
    /// VCS backend to use for the initial clone.
    pub vcs_backend: VcsBackend,
}

pub fn run(opts: &InitOptions<'_>) -> Result<()> {
    // Guard: --apply and --profile both require a source.
    if (opts.apply || opts.profile.is_some()) && opts.source.is_none() {
        bail!(
            "--apply requires a source.\n\
             Use: dfiles init <source> --apply\n\
             To apply an existing local repo, run: dfiles apply"
        );
    }

    match opts.source {
        Some(source_str) => run_from_source(opts, source_str),
        None => run_scaffold(opts.repo_root),
    }
}

/// Clone a git repository (or `gh:owner/repo`) into the repo root, then
/// optionally apply it.
fn run_from_source(opts: &InitOptions<'_>, source_str: &str) -> Result<()> {
    let repo_root = opts.repo_root;

    // Validate: target must be absent or empty.
    if repo_root.exists() {
        let is_empty = repo_root
            .read_dir()
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !is_empty {
            bail!(
                "{} already exists and is not empty.\n\
                 Use --dir to specify a different location.",
                repo_root.display()
            );
        }
    }

    // Resolve the clone URL and any ref embedded in the source string.
    let (clone_url, source_ref) = if let Ok(gh) = GhSource::parse(source_str) {
        let url = format!("https://github.com/{}/{}", gh.owner, gh.repo);
        (url, gh.git_ref)
    } else {
        // Raw git URL — pass through as-is (SSH, HTTPS, local path, etc.).
        (source_str.to_string(), None)
    };

    // --branch overrides @ref in the source string.
    let branch: Option<&str> = match opts.branch {
        Some(b) => {
            if source_ref.is_some() {
                eprintln!("note: --branch '{}' overrides @ref in source", b);
            }
            Some(b)
        }
        None => source_ref.as_deref(),
    };

    match branch {
        Some(b) => println!(
            "Cloning {} (branch: {}) into {} …",
            clone_url,
            b,
            repo_root.display()
        ),
        None => println!("Cloning {} into {} …", clone_url, repo_root.display()),
    }

    vcs::clone_repo(opts.vcs_backend, &clone_url, repo_root, None, branch)
        .with_context(|| format!("Clone failed for {}", clone_url))?;

    println!("Cloned successfully.");

    // Optionally apply.
    if opts.apply {
        if !repo_root.join("dfiles.toml").exists() {
            bail!(
                "Cloned repo does not contain a dfiles.toml — this does not appear to be a \
                 dfiles repository.\n\
                 Cannot apply. Run `dfiles init` in that directory to scaffold one, or \
                 choose a different source."
            );
        }

        let profile = opts.profile.unwrap_or("default");
        println!("\nApplying profile '{}' …", profile);
        apply::run(&apply::ApplyOptions {
            repo_root,
            dest_root: opts.dest_root,
            backup_dir: opts.backup_dir,
            state_dir: opts.state_dir,
            claude_dir: opts.claude_dir,
            profile,
            module_filter: None,
            dry_run: false,
            apply_files: true,
            apply_brews: true,
            apply_ai: true,
            apply_externals: false,
            run_scripts: false,
            remove_unreferenced_brews: false,
            interactive: false,
            vcs_backend: opts.vcs_backend,
        })?;

        println!("\nNext steps:");
        println!("  dfiles status          # verify what was applied");
        println!("  dfiles add ~/.zshrc    # start tracking more files");
    } else {
        println!("\nNext steps:");
        println!("  dfiles apply           # apply config to this machine");
        println!("  dfiles add ~/.zshrc    # start tracking a new dotfile");
    }

    Ok(())
}

/// Create a blank dfiles scaffold at `repo_root`. Refuses if already initialized.
fn run_scaffold(repo_root: &Path) -> Result<()> {
    // Refuse to re-initialize an existing repo.
    if repo_root.join("dfiles.toml").exists() {
        bail!(
            "{} is already initialized (dfiles.toml exists)",
            repo_root.display()
        );
    }

    // Create the directory if it doesn't exist.
    std::fs::create_dir_all(repo_root)
        .with_context(|| format!("Cannot create {}", repo_root.display()))?;

    // Detect version control.
    let has_git = repo_root.join(".git").exists();
    let has_jj = repo_root.join(".jj").exists();
    if !has_git && !has_jj {
        // Not under version control — remind the user.
        eprintln!(
            "hint: {} is not a git/jj repository.\n\
             hint: Run `git init` or `jj git init --colocate` to track your dfiles config.",
            repo_root.display()
        );
    }

    // Scaffold directory structure.
    std::fs::create_dir_all(repo_root.join("modules"))
        .context("Cannot create modules/")?;
    std::fs::create_dir_all(repo_root.join("source"))
        .context("Cannot create source/")?;
    std::fs::create_dir_all(repo_root.join("brew"))
        .context("Cannot create brew/")?;

    // Write dfiles.toml.
    DfilesConfig::write_scaffold(repo_root)?;

    // Write a starter shell module — brew and AI config only.
    // Files are tracked by placing them in source/ with magic-name encoding,
    // so no [[files]] section is needed.
    let shell_toml = r#"# Shell module — brew packages and AI tools for this machine.
# Add Homebrew packages via: dfiles brew install <name> --module shell
# Add AI skills/commands:
#
# [ai]
# skills   = ["gh:gstack/standard-skills@v1"]
# commands = ["gh:myuser/my-commands@main"]
#
# [homebrew]
# brewfile = "brew/Brewfile.shell"
"#;
    std::fs::write(
        repo_root.join("modules").join("shell.toml"),
        shell_toml,
    )
    .context("Cannot write modules/shell.toml")?;

    // Write .gitignore (never commit state files).
    let gitignore = "# dfiles runtime files — do not commit\n.dfiles/\n";
    let gi_path = repo_root.join(".gitignore");
    if !gi_path.exists() {
        std::fs::write(&gi_path, gitignore).context("Cannot write .gitignore")?;
    }

    println!("Initialized dfiles repo at {}", repo_root.display());
    println!();
    println!("Next steps:");
    println!("  dfiles add ~/.zshrc              # start tracking a dotfile");
    println!("  dfiles brew install ripgrep      # track a Homebrew package");
    println!("  dfiles apply                     # apply config to this machine");
    Ok(())
}
