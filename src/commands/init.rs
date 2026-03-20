use anyhow::{bail, Context, Result};
use std::path::Path;

use crate::config::DfilesConfig;

pub fn run(repo_root: &Path) -> Result<()> {
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
             hint: Run `git init` or `jj init --colocate` to track your dfiles config.",
            repo_root.display()
        );
    }

    // Scaffold directory structure.
    std::fs::create_dir_all(repo_root.join("config").join("modules"))
        .context("Cannot create config/modules/")?;
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
        repo_root.join("config").join("modules").join("shell.toml"),
        shell_toml,
    )
    .context("Cannot write config/modules/shell.toml")?;

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
