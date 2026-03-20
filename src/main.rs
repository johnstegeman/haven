mod chezmoi;
mod chezmoi_template;
mod claude_md;
mod commands;
mod config;
mod fs;
mod github;
mod homebrew;
mod lock;
mod manifest;
mod mise;
mod onepassword;
mod source;
mod state;
mod template;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Subcommand)]
enum BrewAction {
    /// Install a formula and record it in a Brewfile in your dfiles repo.
    ///
    /// Examples:
    ///   dfiles brew install ripgrep
    ///   dfiles brew install iterm2 --cask
    ///   dfiles brew install ripgrep --module packages
    Install {
        /// Formula or cask name (e.g. `ripgrep`, `iterm2`).
        name: String,

        /// Install as a cask (GUI apps, fonts, etc.).
        #[arg(long)]
        cask: bool,

        /// Module whose Brewfile to update. Auto-detected when there is exactly
        /// one Brewfile in the repo; required when there are several.
        #[arg(long)]
        module: Option<String>,
    },

    /// Uninstall a formula and remove it from all Brewfiles in your dfiles repo.
    ///
    /// Examples:
    ///   dfiles brew uninstall ripgrep
    ///   dfiles brew uninstall iterm2 --cask
    Uninstall {
        /// Formula or cask name.
        name: String,

        /// Uninstall as a cask.
        #[arg(long)]
        cask: bool,
    },
}

use config::dfiles::repo_root;

#[derive(Parser)]
#[command(
    name = "dfiles",
    version,
    about = "AI-first dotfiles & environment manager",
    long_about = "dfiles tracks dotfiles, packages, and AI tools across machines.\n\
                  \n\
                  Repo directory: ~/dfiles  (override: --dir or DFILES_DIR)\n\
                  State directory: ~/.dfiles (backups, lock file, applied state)\n\
                  Claude directory: ~/.claude (skills, commands, CLAUDE.md)",
)]
struct Cli {
    /// dfiles repo directory. Defaults to ~/dfiles; overridden by DFILES_DIR env var.
    #[arg(long, global = true, env = "DFILES_DIR")]
    dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Bootstrap this machine from a local or remote environment package.
    ///
    /// Without a source, applies the local repo (equivalent to `dfiles apply` + `dfiles status`).
    /// With a `gh:` source, fetches the remote package first, then applies it.
    ///
    /// Examples:
    ///   dfiles bootstrap
    ///   dfiles bootstrap gh:alice/my-env
    ///   dfiles bootstrap gh:alice/my-env@v1.2 --profile work --dry-run
    Bootstrap {
        /// Remote environment package: `gh:owner/repo` or `gh:owner/repo@ref`.
        /// Omit to bootstrap from the local repo.
        source: Option<String>,

        /// Profile to apply.
        #[arg(long, default_value = "default")]
        profile: String,

        /// Print what would be applied without writing any files or fetching packages.
        #[arg(long)]
        dry_run: bool,
    },

    /// Initialize a new dfiles repository in the current or --dir directory.
    Init,

    /// Start tracking a dotfile by copying it into the repo's source/ directory.
    Add {
        /// Absolute or relative path to the file to track (e.g. ~/.zshrc).
        file: PathBuf,

        /// Track as a symlink: on apply, dest will be symlinked back into source/
        /// instead of copied. Use for files that apps manage themselves (e.g. VS Code settings).
        #[arg(long)]
        link: bool,
    },

    /// Apply tracked files and packages to this machine.
    ///
    /// Copies source files to their destinations, installs Homebrew packages,
    /// runs mise, and fetches AI skills/commands. Backs up any existing files first.
    Apply {
        /// Profile to apply.
        #[arg(long, default_value = "default")]
        profile: String,

        /// Apply only a single module (e.g. --module shell).
        #[arg(long)]
        module: Option<String>,

        /// Print what would be applied without writing any files.
        #[arg(long)]
        dry_run: bool,

        /// (Debug builds only) Write files into this directory instead of `~`.
        ///
        /// Mirrors the real filesystem layout under the given root so you can
        /// inspect the result without touching your live configuration.
        /// Example: `dfiles apply --dest /tmp/dfiles-test`
        #[cfg_attr(debug_assertions, arg(long, value_name = "DIR"))]
        #[cfg_attr(not(debug_assertions), arg(skip))]
        dest: Option<PathBuf>,
    },

    /// Show drift between tracked source files and live destinations.
    ///
    /// Drift markers: ✓ clean  M modified  ? missing  ! source missing
    Status {
        /// Profile to check.
        #[arg(long, default_value = "default")]
        profile: String,
    },

    /// Run `brew install`/`uninstall` and keep your dfiles Brewfiles in sync.
    ///
    /// Use these commands instead of bare `brew install` when you want the
    /// change to persist across machines — the formula is automatically added
    /// to (or removed from) the Brewfile(s) in your dfiles repo.
    ///
    /// Examples:
    ///   dfiles brew install ripgrep
    ///   dfiles brew install iterm2 --cask
    ///   dfiles brew uninstall ripgrep
    Brew {
        #[command(subcommand)]
        action: BrewAction,
    },

    /// Import dotfiles from another dotfile manager.
    ///
    /// Reads the source manager's directory, decodes its naming conventions,
    /// and imports files into dfiles' source/ directory with generated module
    /// TOML configs.
    ///
    /// What is imported:
    ///   Plain files         — copied verbatim (template = false)
    ///   private_ prefix     — imported with private = true (chmod 0600)
    ///   executable_ prefix  — imported with executable = true (chmod 0755)
    ///   symlink_ prefix     — imported as symlinks (link = true) when the
    ///                         target can be resolved; skipped otherwise
    ///   *.tmpl files        — Go template syntax converted to Tera and
    ///                         imported with template = true; unconvertible
    ///                         constructs are preserved with a warning
    ///
    /// What is skipped:
    ///   run_* / once_* / run_once_*  — install scripts
    ///   exact_* / create_* / modify_*  — unsupported chezmoi attributes
    ///   .chezmoi* / chezmoistate.*   — chezmoi-internal files
    ///
    /// Examples:
    ///   dfiles import --from chezmoi
    ///   dfiles import --from chezmoi --source ~/my-chezmoi-dir --dry-run
    Import {
        /// Source format to import from. Currently only `chezmoi` is supported.
        #[arg(long)]
        from: String,

        /// Path to the source manager's directory.
        /// Auto-detected via `chezmoi source-path` if not specified.
        #[arg(long)]
        source: Option<std::path::PathBuf>,

        /// Print what would be imported without writing any files.
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    let repo = match &cli.dir {
        Some(d) => d.clone(),
        None => repo_root()?,
    };

    // State and backup directories live outside the repo (not committed).
    let state_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".dfiles");
    let backup_dir = state_dir.join("backups");
    let claude_dir = std::env::var("DFILES_CLAUDE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".claude")
        });
    let envs_dir = std::env::var("DFILES_ENVS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| state_dir.join("envs"));

    match &cli.command {
        Commands::Bootstrap {
            source,
            profile,
            dry_run,
        } => {
            commands::bootstrap::run(&commands::bootstrap::BootstrapOptions {
                source: source.as_deref(),
                repo_root: &repo,
                dest_root: std::path::Path::new("/"),
                backup_dir: &backup_dir,
                state_dir: &state_dir,
                claude_dir: &claude_dir,
                envs_dir: &envs_dir,
                profile,
                dry_run: *dry_run,
            })?;
        }

        Commands::Init => {
            commands::init::run(&repo)?;
        }

        Commands::Add { file, link } => {
            commands::add::run(&repo, file, *link)?;
        }

        Commands::Apply {
            profile,
            module,
            dry_run,
            dest,
        } => {
            let dest_root_buf = dest
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/"));
            commands::apply::run(&commands::apply::ApplyOptions {
                repo_root: &repo,
                dest_root: &dest_root_buf,
                backup_dir: &backup_dir,
                state_dir: &state_dir,
                claude_dir: &claude_dir,
                profile,
                module_filter: module.as_deref(),
                dry_run: *dry_run,
            })?;
        }

        Commands::Status { profile } => {
            commands::status::run(&commands::status::StatusOptions {
                repo_root: &repo,
                dest_root: std::path::Path::new("/"),
                claude_dir: &claude_dir,
                profile,
            })?;
        }

        Commands::Brew { action } => match action {
            BrewAction::Install { name, cask, module } => {
                commands::brew::install(&repo, name, *cask, module.as_deref())?;
            }
            BrewAction::Uninstall { name, cask } => {
                commands::brew::uninstall(&repo, name, *cask)?;
            }
        },

        Commands::Import { from, source, dry_run } => {
            if from != "chezmoi" {
                anyhow::bail!(
                    "Unknown import source '{}'. Only 'chezmoi' is supported in v1.",
                    from
                );
            }
            commands::import::run(&commands::import::ImportOptions {
                repo_root: &repo,
                source_dir: source.as_deref(),
                dry_run: *dry_run,
            })?;
        }
    }

    Ok(())
}
