mod ai_platform;
mod ai_skill;
mod chezmoi;
mod chezmoi_template;
mod claude_md;
mod commands;
mod config;
mod config_injection;
mod diff_util;
mod drift;
mod fs;
mod github;
mod homebrew;
mod ignore;
mod lock;
mod mise;
mod onepassword;
mod skill_cache;
mod source;
mod state;
mod telemetry;
mod template;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Subcommand)]
enum AiAction {
    /// Scan this machine for installed AI agent platforms and offer to update
    /// `ai/platforms.toml`.
    ///
    /// Detects platforms by checking for known binaries (claude, codex, cursor,
    /// etc.) and config directories. Prints what was found and prompts before
    /// making any changes.
    ///
    /// Example:
    ///   dfiles ai discover
    Discover,

    /// Add a skill declaration to `ai/skills.toml`.
    ///
    /// Does not deploy the skill; run `dfiles apply --ai` afterward.
    ///
    /// Examples:
    ///   dfiles ai add gh:anthropics/skills/pdf-processing
    ///   dfiles ai add gh:owner/repo --name my-skill --platforms claude-code,codex
    ///   dfiles ai add dir:~/projects/my-skill --deploy copy
    Add {
        /// Skill source: `gh:owner/repo[/subpath][@ref]` or `dir:~/path`.
        source: String,

        /// Local name for the skill. Defaults to the last path component of source.
        #[arg(long)]
        name: Option<String>,

        /// Target platforms: `all`, `cross-client`, or comma-separated platform IDs
        /// (e.g. `claude-code,codex`). Default: `all`.
        #[arg(long, default_value = "all")]
        platforms: String,

        /// Deploy method: `symlink` (default) or `copy`.
        #[arg(long, default_value = "symlink")]
        deploy: String,
    },

    /// Download skills into the local cache without deploying them.
    ///
    /// Respects the lock file — already-cached skills at the pinned SHA are skipped.
    /// `dir:` skills are always skipped (local dirs are read directly on apply).
    ///
    /// Examples:
    ///   dfiles ai fetch
    ///   dfiles ai fetch pdf-processing
    Fetch {
        /// Skill name to fetch. Omit to fetch all skills.
        name: Option<String>,
    },

    /// Fetch the latest version of skills, ignoring the current lock SHA.
    ///
    /// Unlike `fetch`, this clears the lock entry before fetching so the
    /// skill is always re-downloaded from its source. Run `dfiles apply --ai`
    /// afterward to deploy updated skills.
    ///
    /// Examples:
    ///   dfiles ai update
    ///   dfiles ai update pdf-processing
    Update {
        /// Skill name to update. Omit to update all skills.
        name: Option<String>,
    },

    /// Remove a skill from `ai/skills.toml` and optionally remove deployed copies.
    ///
    /// Never removes deployed files automatically — always prompts unless --yes
    /// is given.
    ///
    /// Example:
    ///   dfiles ai remove pdf-processing
    ///   dfiles ai remove pdf-processing --yes
    Remove {
        /// Skill name as declared in `ai/skills.toml`.
        name: String,

        /// Skip confirmation prompts.
        #[arg(long)]
        yes: bool,
    },

    /// Search skills.sh for skills matching a query and display results.
    ///
    /// Results show the skill name, gh: source, and install count.
    /// To add a skill from the results: `dfiles ai add gh:owner/repo/skill-name`.
    ///
    /// Examples:
    ///   dfiles ai search jujutsu
    ///   dfiles ai search "pdf processing" --limit 5
    Search {
        /// Search query.
        query: String,

        /// Maximum number of results to show. Default: 10.
        #[arg(long, default_value = "10")]
        limit: u8,
    },

    /// Scan a skills directory for unmanaged skills and offer to add them.
    ///
    /// Walks the given directory for subdirectories containing a SKILL.md.
    /// For each unmanaged skill, tries to detect the gh: source via git remote,
    /// then falls back to searching skills.sh. Prompts for confirmation before
    /// adding anything to ai/skills.toml.
    ///
    /// Examples:
    ///   dfiles ai scan ~/.claude/skills
    ///   dfiles ai scan ~/.agents/skills --dry-run
    Scan {
        /// Directory to scan for skill subdirectories.
        dir: String,

        /// Show what would be added without writing to ai/skills.toml.
        #[arg(long)]
        dry_run: bool,
    },
}

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

use config::dfiles::{DfilesConfig, repo_root};

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
    /// Create a new dfiles repository (first-time setup).
    ///
    /// Without a source, creates a blank scaffold in the --dir directory.
    /// With a source, clones the repository and optionally applies it immediately.
    ///
    /// Use this once when setting up a machine for the first time. For subsequent
    /// re-provisioning of an already-initialised machine, use `dfiles apply`.
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

        /// Immediately install after adding: replace the original file with a symlink
        /// back into source/. Only valid with --link.
        #[arg(long, requires = "link")]
        apply: bool,

        /// Re-copy the file into source/ even if it is already tracked.
        /// Without this flag, adding an already-tracked file is an error.
        #[arg(long)]
        update: bool,
    },

    /// Stop tracking a dotfile by removing it from the source/ directory.
    ///
    /// The live file on disk is left unchanged — only the source/ copy is removed.
    /// Run `dfiles status` first to verify the path before removing.
    ///
    /// Examples:
    ///   dfiles remove ~/.zshrc
    ///   dfiles remove ~/.config/git/config --dry-run
    Remove {
        /// Destination path to stop tracking (e.g. ~/.zshrc).
        file: PathBuf,

        /// Print what would be removed without deleting any files.
        #[arg(long)]
        dry_run: bool,
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

        /// Scope brew and AI operations to this module (e.g. --module shell).
        /// Note: dotfiles in source/ are always applied globally regardless of this flag.
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

        /// Execute scripts from `source/scripts/` during this apply.
        /// `run_once_` scripts execute only once per machine; subsequent runs are skipped.
        /// `run_` scripts execute on every apply when this flag is present.
        /// Scripts are never run without this flag (opt-in for safety).
        #[arg(long)]
        run_scripts: bool,

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

        /// Scope brew and AI diff to a single module.
        /// Note: dotfiles in source/ are always diffed globally regardless of this flag.
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
    ///
    /// By default all sections are shown. Use --files, --brews, and/or --ai
    /// to inspect only specific sections.
    Status {
        /// Profile to check. Defaults to the last-used profile saved in state,
        /// or "default" if no prior apply has been recorded.
        #[arg(long)]
        profile: Option<String>,

        /// Show dotfile drift only.
        #[arg(long)]
        files: bool,

        /// Show Homebrew package drift only.
        #[arg(long)]
        brews: bool,

        /// Show AI skill drift only.
        #[arg(long)]
        ai: bool,
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

    /// Manage AI agent skills across platforms.
    ///
    /// Skills are declared in `ai/skills.toml` and deployed to platform skill
    /// directories (e.g. `~/.claude/skills/`) by `dfiles apply`.
    ///
    /// Examples:
    ///   dfiles ai discover
    ///   dfiles ai add gh:anthropics/skills/pdf-processing
    ///   dfiles ai fetch
    ///   dfiles ai update
    ///   dfiles ai remove my-skill
    Ai {
        #[command(subcommand)]
        action: AiAction,
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
    ///   .chezmoiignore patterns       — unless --include-ignored-files is given
    ///
    /// Examples:
    ///   dfiles import --from chezmoi
    ///   dfiles import --from chezmoi --source ~/my-chezmoi-dir --dry-run
    ///   dfiles import --from chezmoi --include-ignored-files
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

        /// Import files that match `.chezmoiignore` patterns instead of skipping them.
        /// The patterns are still written to `config/ignore`, so `dfiles apply`,
        /// `status`, and `diff` will continue to exclude those files.
        #[arg(long)]
        include_ignored_files: bool,
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
    // Collect raw args before clap consumes them.
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let cmd_name = cmd_name_from_args(&raw_args);
    let flags = flags_from_args(&raw_args);

    let config_enabled = try_load_telemetry_config();
    let recorder = telemetry::Recorder::new(
        telemetry::is_enabled(config_enabled),
        cmd_name,
        flags,
        None, // profile resolved after full parse; fine to omit for now
    );

    let result = run();
    recorder.finish(&result);

    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

/// Extract the subcommand name from raw CLI args (first non-flag arg).
fn cmd_name_from_args(args: &[String]) -> String {
    // Skip global flags like --dir /path (flag + value pairs) to find the subcommand.
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--dir" || arg == "--profile" {
            skip_next = true;
            continue;
        }
        if !arg.starts_with('-') {
            return arg.clone();
        }
    }
    "unknown".to_string()
}

/// Collect flag names (args starting with `-`) from raw CLI args.
fn flags_from_args(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|a| a.starts_with('-'))
        .cloned()
        .collect()
}

/// Load the telemetry.enabled setting from dfiles.toml (best-effort, never panics).
fn try_load_telemetry_config() -> bool {
    (|| -> Option<bool> {
        let repo = repo_root().ok()?;
        let cfg = DfilesConfig::load(&repo).ok()?;
        Some(cfg.telemetry.enabled)
    })()
    .unwrap_or(false)
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
    match &cli.command {
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

        Commands::Add { file, link, apply, update } => {
            commands::add::run(&repo, file, *link, *apply, *update)?;
        }

        Commands::Remove { file, dry_run } => {
            commands::remove::run(&commands::remove::RemoveOptions {
                repo_root: &repo,
                file,
                dry_run: *dry_run,
            })?;
        }

        Commands::Apply {
            profile,
            module,
            dry_run,
            files,
            brews,
            ai,
            apply_externals,
            run_scripts,
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
                run_scripts: *run_scripts,
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
                state_dir: &state_dir,
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

        Commands::Status { profile, files, brews, ai } => {
            let resolved = resolve_profile(profile.as_deref(), &state_dir);
            commands::status::run(&commands::status::StatusOptions {
                repo_root: &repo,
                dest_root: std::path::Path::new("/"),
                claude_dir: &claude_dir,
                profile: &resolved,
                show_files: *files,
                show_brews: *brews,
                show_ai: *ai,
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

        Commands::Ai { action } => match action {
            AiAction::Discover => {
                commands::ai::discover(&commands::ai::DiscoverOptions {
                    repo_root: &repo,
                })?;
            }
            AiAction::Add {
                source,
                name,
                platforms,
                deploy,
            } => {
                commands::ai::add(&commands::ai::AddOptions {
                    repo_root: &repo,
                    source,
                    name: name.as_deref(),
                    platforms,
                    deploy,
                })?;
            }
            AiAction::Fetch { name } => {
                commands::ai::fetch(&commands::ai::FetchOptions {
                    repo_root: &repo,
                    state_dir: &state_dir,
                    name: name.as_deref(),
                })?;
            }
            AiAction::Update { name } => {
                commands::ai::update(&commands::ai::UpdateOptions {
                    repo_root: &repo,
                    state_dir: &state_dir,
                    name: name.as_deref(),
                })?;
            }
            AiAction::Remove { name, yes } => {
                commands::ai::remove(&commands::ai::RemoveOptions {
                    repo_root: &repo,
                    state_dir: &state_dir,
                    name,
                    yes: *yes,
                })?;
            }
            AiAction::Search { query, limit } => {
                commands::ai::search(&commands::ai::SearchOptions {
                    query,
                    limit: *limit,
                })?;
            }
            AiAction::Scan { dir, dry_run } => {
                commands::ai::scan(&commands::ai::ScanOptions {
                    repo_root: &repo,
                    state_dir: &state_dir,
                    dir,
                    dry_run: *dry_run,
                })?;
            }
        },

        Commands::Import { from, source, dry_run, include_ignored_files } => {
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
                include_ignored_files: *include_ignored_files,
            })?;
        }
    }

    Ok(())
}
