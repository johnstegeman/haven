mod chezmoi;
mod chezmoi_template;
mod claude_md;
mod commands;
mod config;
mod diff_util;
mod drift;
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

    /// Initialize a new dfiles repository, optionally cloned from a remote.
    ///
    /// Without a source, creates a blank scaffold in the --dir directory.
    /// With a source, clones the repository and optionally applies it.
    ///
    /// Examples:
    ///   dfiles init
    ///   dfiles init gh:alice/dotfiles
    ///   dfiles init gh:alice/dotfiles --branch dev
    ///   dfiles init https://github.com/alice/dotfiles --apply
    ///   dfiles init gh:alice/dotfiles --apply --profile work
    Init {
        /// Git repository to clone as your dfiles repo.
        /// Accepts `gh:owner/repo[@ref]` notation or any URL that `git clone` accepts
        /// (HTTPS, SSH, local path, etc.). Omit to create a blank scaffold.
        source: Option<String>,

        /// Branch to clone. If omitted, the repo's default branch is used.
        /// Overrides any `@ref` in the source if both are given.
        #[arg(long)]
        branch: Option<String>,

        /// Apply the cloned repo immediately after cloning. Requires a source.
        #[arg(long)]
        apply: bool,

        /// Profile to apply. Requires --apply.
        #[arg(long)]
        profile: Option<String>,
    },

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
    ///
    /// By default all sections are applied. Use --files, --brews, and/or --ai
    /// to apply only specific sections.
    Apply {
        /// Profile to apply. Defaults to the last-used profile saved in state,
        /// or "default" if no prior apply has been recorded.
        #[arg(long)]
        profile: Option<String>,

        /// Apply only a single module (e.g. --module shell).
        #[arg(long)]
        module: Option<String>,

        /// Print what would be applied without writing any files.
        #[arg(long)]
        dry_run: bool,

        /// Apply dotfile copies/symlinks. If none of --files/--brews/--ai are
        /// given, all sections are applied.
        #[arg(long)]
        files: bool,

        /// Apply Homebrew packages. If none of --files/--brews/--ai are given,
        /// all sections are applied.
        #[arg(long)]
        brews: bool,

        /// Apply AI tools (mise, skills, commands). If none of --files/--brews/--ai
        /// are given, all sections are applied.
        #[arg(long)]
        ai: bool,

        /// After installing packages, uninstall any leaf formula or cask that is
        /// not referenced by any Brewfile in the active profile.
        #[arg(long)]
        remove_unreferenced_brews: bool,

        /// Show the list of unreferenced packages and prompt for confirmation
        /// before removing. Implies --remove-unreferenced-brews.
        #[arg(long)]
        interactive: bool,

        /// Pull (update) existing extdir_ clones in addition to cloning missing ones.
        /// By default, extdir_ entries that are already cloned are left as-is.
        #[arg(long)]
        apply_externals: bool,

        /// (Debug builds only) Write files into this directory instead of `~`.
        ///
        /// Mirrors the real filesystem layout under the given root so you can
        /// inspect the result without touching your live configuration.
        /// Example: `dfiles apply --dest /tmp/dfiles-test`
        #[cfg_attr(debug_assertions, arg(long, value_name = "DIR"))]
        #[cfg_attr(not(debug_assertions), arg(skip))]
        dest: Option<PathBuf>,
    },

    /// Show the diff between tracked source files/packages and live state.
    ///
    /// Exits 0 when everything is up to date, 1 when drift is found.
    ///
    /// By default all sections are diffed. Use --files, --brews, and/or --ai
    /// to inspect only specific sections.
    ///
    /// Examples:
    ///   dfiles diff
    ///   dfiles diff --files
    ///   dfiles diff --brews
    ///   dfiles diff --stat
    ///   dfiles diff --profile work --color=always
    Diff {
        /// Profile to diff. Defaults to the last-used profile saved in state,
        /// or "default" if no prior apply has been recorded.
        #[arg(long)]
        profile: Option<String>,

        /// Scope brew/AI diff to a single module.
        #[arg(long)]
        module: Option<String>,

        /// Diff source files only.
        #[arg(long)]
        files: bool,

        /// Diff Homebrew packages only.
        #[arg(long)]
        brews: bool,

        /// Diff AI tools (skills, commands) only.
        #[arg(long)]
        ai: bool,

        /// Show a summary (file names + change counts) instead of full diff content.
        #[arg(long)]
        stat: bool,

        /// Control color output: always, never, or auto (default).
        #[arg(long, default_value = "auto", value_parser = parse_color_mode)]
        color: commands::diff::ColorMode,

        /// (Debug builds only) Treat this directory as the filesystem root.
        #[cfg_attr(debug_assertions, arg(long, value_name = "DIR"))]
        #[cfg_attr(not(debug_assertions), arg(skip))]
        dest: Option<PathBuf>,
    },

    /// Show drift between tracked source files and live destinations.
    ///
    /// Drift markers: ✓ clean  M modified  ? missing  ! source missing
    Status {
        /// Profile to check. Defaults to the last-used profile saved in state,
        /// or "default" if no prior apply has been recorded.
        #[arg(long)]
        profile: Option<String>,
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

/// Parse the `--color` argument into a `ColorMode`.
fn parse_color_mode(s: &str) -> Result<commands::diff::ColorMode, String> {
    match s {
        "always" => Ok(commands::diff::ColorMode::Always),
        "never"  => Ok(commands::diff::ColorMode::Never),
        "auto"   => Ok(commands::diff::ColorMode::Auto),
        other    => Err(format!("unknown color mode '{}'; use always, never, or auto", other)),
    }
}

/// Resolve the profile to use for apply/status.
///
/// Priority: explicit --profile flag → last-used profile in state.json → "default"
fn resolve_profile(explicit: Option<&str>, state_dir: &std::path::Path) -> String {
    if let Some(p) = explicit {
        return p.to_string();
    }
    crate::state::State::load(state_dir)
        .ok()
        .and_then(|s| s.profile)
        .unwrap_or_else(|| "default".to_string())
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

        Commands::Init {
            source,
            branch,
            apply,
            profile,
        } => {
            commands::init::run(&commands::init::InitOptions {
                repo_root: &repo,
                source: source.as_deref(),
                branch: branch.as_deref(),
                apply: *apply,
                profile: profile.as_deref(),
                dest_root: std::path::Path::new("/"),
                backup_dir: &backup_dir,
                state_dir: &state_dir,
                claude_dir: &claude_dir,
            })?;
        }

        Commands::Add { file, link } => {
            commands::add::run(&repo, file, *link)?;
        }

        Commands::Apply {
            profile,
            module,
            dry_run,
            files,
            brews,
            ai,
            apply_externals,
            remove_unreferenced_brews,
            interactive,
            dest,
        } => {
            let resolved = resolve_profile(profile.as_deref(), &state_dir);
            let dest_root_buf = dest
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/"));
            let none_specified = !files && !brews && !ai;
            commands::apply::run(&commands::apply::ApplyOptions {
                repo_root: &repo,
                dest_root: &dest_root_buf,
                backup_dir: &backup_dir,
                state_dir: &state_dir,
                claude_dir: &claude_dir,
                profile: &resolved,
                module_filter: module.as_deref(),
                dry_run: *dry_run,
                apply_files: *files || none_specified,
                apply_brews: *brews || none_specified,
                apply_ai: *ai || none_specified,
                apply_externals: *apply_externals,
                remove_unreferenced_brews: *remove_unreferenced_brews || *interactive,
                interactive: *interactive,
            })?;
        }

        Commands::Diff {
            profile,
            module,
            files,
            brews,
            ai,
            stat,
            color,
            dest,
        } => {
            let resolved = resolve_profile(profile.as_deref(), &state_dir);
            let dest_root_buf = dest
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/"));
            let none_specified = !files && !brews && !ai;
            let has_drift = commands::diff::run(&commands::diff::DiffOptions {
                repo_root: &repo,
                dest_root: &dest_root_buf,
                claude_dir: &claude_dir,
                profile: &resolved,
                module_filter: module.as_deref(),
                diff_files: *files || none_specified,
                diff_brews: *brews || none_specified,
                diff_ai: *ai || none_specified,
                stat_only: *stat,
                color: *color,
            })?;
            if has_drift {
                std::process::exit(1);
            }
        }

        Commands::Status { profile } => {
            let resolved = resolve_profile(profile.as_deref(), &state_dir);
            commands::status::run(&commands::status::StatusOptions {
                repo_root: &repo,
                dest_root: std::path::Path::new("/"),
                claude_dir: &claude_dir,
                profile: &resolved,
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
