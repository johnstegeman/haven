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
mod ai_config;
mod skill_backend;
mod skill_backend_agentskills;
mod skill_backend_factory;
mod skill_backend_native;
mod skill_cache;
mod util;
mod source;
mod state;
mod telemetry;
mod template;
mod vcs;

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
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
    ///   haven ai discover
    Discover,

    /// Add a skill declaration to `ai/skills.toml`.
    ///
    /// Does not deploy the skill; run `haven apply --ai` afterward.
    ///
    /// Examples:
    ///   haven ai add gh:anthropics/skills/pdf-processing
    ///   haven ai add gh:owner/repo --name my-skill --platforms claude-code,codex
    ///   haven ai add dir:~/projects/my-skill --deploy copy
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

    /// Import a locally-developed skill into the haven repo.
    ///
    /// Copies the skill directory into `ai/skills/<name>/files/`, writes
    /// `ai/skills/<name>/skill.toml` with `source = "repo:"`, creates a blank
    /// `all.md` snippet stub, and removes the original directory.
    ///
    /// Run `haven apply --ai` afterward to deploy the skill symlink to
    /// `~/.claude/skills/<name>` (or equivalent for your active platforms).
    ///
    /// Examples:
    ///   haven ai add-local ~/.claude/skills/myskill
    ///   haven ai add-local ~/dev/my-skill --name myskill
    ///   haven ai add-local ~/.claude/skills/myskill --platforms claude-code
    #[command(name = "add-local")]
    AddLocal {
        /// Path to the local skill directory to import.
        path: String,

        /// Name for the skill. Defaults to the directory name of the path.
        #[arg(long)]
        name: Option<String>,

        /// Target platforms: `all`, `cross-client`, or comma-separated platform IDs.
        /// Default: `all`.
        #[arg(long, default_value = "all")]
        platforms: String,
    },

    /// Download skills into the local cache without deploying them.
    ///
    /// Respects the lock file — already-cached skills at the pinned SHA are skipped.
    /// `dir:` and `repo:` skills are always skipped (read directly on apply).
    ///
    /// Examples:
    ///   haven ai fetch
    ///   haven ai fetch pdf-processing
    Fetch {
        /// Skill name to fetch. Omit to fetch all skills.
        name: Option<String>,
    },

    /// Fetch the latest version of skills, ignoring the current lock SHA.
    ///
    /// Unlike `fetch`, this clears the lock entry before fetching so the
    /// skill is always re-downloaded from its source. Run `haven apply --ai`
    /// afterward to deploy updated skills.
    ///
    /// Examples:
    ///   haven ai update
    ///   haven ai update pdf-processing
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
    ///   haven ai remove pdf-processing
    ///   haven ai remove pdf-processing --yes
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
    /// To add a skill from the results: `haven ai add gh:owner/repo/skill-name`.
    ///
    /// Examples:
    ///   haven ai search jujutsu
    ///   haven ai search "pdf processing" --limit 5
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
    ///   haven ai scan ~/.claude/skills
    ///   haven ai scan ~/.agents/skills --dry-run
    Scan {
        /// Directory to scan for skill subdirectories.
        path: String,

        /// Show what would be added without writing to ai/skills.toml.
        #[arg(long)]
        dry_run: bool,
    },

    /// List all known skill backends and their availability on this machine.
    ///
    /// Checks whether each backend's required runner is present on PATH.
    /// The active backend (from ai/config.toml) is marked with *.
    ///
    /// Example:
    ///   haven ai backends
    Backends,
}

#[derive(Subcommand)]
enum BrewAction {
    /// Install a formula and record it in a Brewfile in your haven repo.
    ///
    /// Examples:
    ///   haven brew install ripgrep
    ///   haven brew install iterm2 --cask
    ///   haven brew install ripgrep --module packages
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

    /// Uninstall a formula and remove it from all Brewfiles in your haven repo.
    ///
    /// Examples:
    ///   haven brew uninstall ripgrep
    ///   haven brew uninstall iterm2 --cask
    Uninstall {
        /// Formula or cask name.
        name: String,

        /// Uninstall as a cask.
        #[arg(long)]
        cask: bool,
    },
}

use config::haven::{HavenConfig, repo_root};

#[derive(Parser)]
#[command(
    name = "haven",
    version = telemetry::BUILD_VERSION,
    about = "AI-first dotfiles & environment manager",
    long_about = "haven tracks dotfiles, packages, and AI tools across machines.\n\
                  \n\
                  Repo directory: ~/.local/share/haven  (override: --dir or HAVEN_DIR)\n\
                  State directory: ~/.haven (backups, lock file, applied state)\n\
                  Claude directory: ~/.claude (skills, commands, CLAUDE.md)",
)]
struct Cli {
    /// haven repo directory. Defaults to ~/.local/share/haven; overridden by HAVEN_DIR env var.
    #[arg(long, global = true, env = "HAVEN_DIR")]
    dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate shell completion scripts for haven.
    ///
    /// Print the completion script to stdout and source it in your shell config.
    ///
    /// Fish:
    ///   haven completions fish > ~/.config/fish/completions/haven.fish
    ///
    /// Zsh (add to ~/.zshrc):
    ///   source <(haven completions zsh)
    ///
    /// Bash (add to ~/.bashrc):
    ///   source <(haven completions bash)
    ///
    /// If you use a shell alias (e.g. `alias hv=haven`), pass --cmd-name so
    /// completions fire on the alias rather than the binary name:
    ///
    ///   haven completions fish --cmd-name hv | source
    ///   haven completions fish --cmd-name hv > ~/.config/fish/completions/hv.fish
    Completions {
        /// Shell to generate completions for: fish, zsh, or bash.
        shell: Shell,

        /// Override the command name used in the completion script.
        ///
        /// Useful when you invoke haven via an alias (e.g. `hv`). The generated
        /// completions will trigger on the alias name instead of `haven`.
        #[arg(long, value_name = "NAME")]
        cmd_name: Option<String>,
    },

    /// Print the path to the haven repo directory and exit.
    ///
    /// Useful in shell scripts and aliases:
    ///   cd $(haven source-path)
    ///   alias haven-cd='cd $(haven source-path)'
    ///
    /// Resolution order:
    ///   1. $HAVEN_DIR env var
    ///   2. ~/haven if it contains a haven repo (migration)
    ///   3. $XDG_DATA_HOME/haven
    ///   4. ~/.local/share/haven  (default for new installs)
    SourcePath,

    /// Create a new haven repository (first-time setup).
    ///
    /// Without a source, creates a blank scaffold in the --dir directory.
    /// With a source, clones the repository and optionally applies it immediately.
    ///
    /// Use this once when setting up a machine for the first time. For subsequent
    /// re-provisioning of an already-initialised machine, use `haven apply`.
    ///
    /// Examples:
    ///   haven init
    ///   haven init gh:alice/dotfiles
    ///   haven init gh:alice/dotfiles --branch dev
    ///   haven init https://github.com/alice/dotfiles --apply
    ///   haven init gh:alice/dotfiles --apply --profile work
    ///   haven init gh:alice/dotfiles --vcs jj
    Init {
        /// Git repository to clone as your haven repo.
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

        /// VCS backend to use: `git` (default) or `jj` (Jujutsu colocated).
        /// Overrides HAVEN_VCS env var and haven.toml [vcs] settings.
        #[arg(long, value_name = "BACKEND")]
        vcs: Option<String>,
    },

    /// List all tracked items: files, Homebrew packages, and AI skills.
    ///
    /// Without flags, shows all sections. Use --files, --brews, or --ai to
    /// show only that section. Use --filter to search by substring, and
    /// --count to print just the total count instead of individual entries.
    ///
    /// Examples:
    ///   haven list
    ///   haven list --files
    ///   haven list --brews
    ///   haven list --ai
    ///   haven list --files --filter settings
    ///   haven list --files --count
    List {
        /// Show only tracked dotfiles.
        #[arg(long, conflicts_with_all = ["brews", "ai"])]
        files: bool,

        /// Show only Homebrew packages.
        #[arg(long, conflicts_with_all = ["files", "ai"])]
        brews: bool,

        /// Show only AI skills.
        #[arg(long, conflicts_with_all = ["files", "brews"])]
        ai: bool,

        /// Profile to resolve modules for.
        /// Defaults to the last-used profile (same as `haven apply`).
        #[arg(long)]
        profile: Option<String>,

        /// Show only entries whose path or name contains this substring (case-insensitive).
        #[arg(long, value_name = "PATTERN")]
        filter: Option<String>,

        /// Print only the total count of matching entries instead of listing them.
        #[arg(long)]
        count: bool,
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

    /// Re-copy an already-tracked file into the repo's source/ directory.
    ///
    /// Alias for `haven add --update <file>`. Use this when you have edited a
    /// tracked file directly on disk and want to push those changes back into
    /// your haven repo.
    ///
    /// Examples:
    ///   haven update ~/.zshrc
    ///   haven update ~/.config/git/config
    Update {
        /// Absolute or relative path to the tracked file to re-copy.
        file: PathBuf,
    },

    /// Stop tracking a dotfile by removing it from the source/ directory.
    ///
    /// The live file on disk is left unchanged — only the source/ copy is removed.
    /// Run `haven status` first to verify the path before removing.
    ///
    /// Examples:
    ///   haven remove ~/.zshrc
    ///   haven remove ~/.config/git/config --dry-run
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

        /// When removing unreferenced casks, also delete their associated data and
        /// files (passes --zap to `brew uninstall --cask`). Implies
        /// --remove-unreferenced-brews.
        #[arg(long)]
        zap: bool,

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

        /// How to resolve conflicts when the destination file was edited since
        /// the last apply. `prompt` asks interactively (default on TTY, falls
        /// back to `skip` in non-TTY environments). `skip` keeps the user's
        /// version and exits 1. `overwrite` replaces silently and exits 0.
        #[arg(long, value_name = "MODE", default_value = "prompt")]
        on_conflict: String,

        /// VCS backend to use for new extdir clones: `git` (default) or `jj`.
        /// Overrides HAVEN_VCS env var and haven.toml [vcs] settings.
        #[arg(long, value_name = "BACKEND")]
        vcs: Option<String>,

        /// (Debug builds only) Write files into this directory instead of `~`.
        ///
        /// Mirrors the real filesystem layout under the given root so you can
        /// inspect the result without touching your live configuration.
        /// Example: `haven apply --dest /tmp/haven-test`
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
    ///   haven diff
    ///   haven diff --files
    ///   haven diff --brews
    ///   haven diff --stat
    ///   haven diff --profile work --color=always
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

    /// Run `brew install`/`uninstall` and keep your haven Brewfiles in sync.
    ///
    /// Use these commands instead of bare `brew install` when you want the
    /// change to persist across machines — the formula is automatically added
    /// to (or removed from) the Brewfile(s) in your haven repo.
    ///
    /// Examples:
    ///   haven brew install ripgrep
    ///   haven brew install iterm2 --cask
    ///   haven brew uninstall ripgrep
    Brew {
        #[command(subcommand)]
        action: BrewAction,
    },

    /// Manage AI agent skills across platforms.
    ///
    /// Skills are declared in `ai/skills.toml` and deployed to platform skill
    /// directories (e.g. `~/.claude/skills/`) by `haven apply`.
    ///
    /// Examples:
    ///   haven ai discover
    ///   haven ai add gh:anthropics/skills/pdf-processing
    ///   haven ai fetch
    ///   haven ai update
    ///   haven ai remove my-skill
    Ai {
        #[command(subcommand)]
        action: AiAction,
    },

    /// Show the active VCS backend and how it was resolved.
    ///
    /// Prints whether haven will use `git` or `jj` for new clones, and
    /// indicates whether the setting came from a CLI flag, environment variable,
    /// config file, interactive detection, or default.
    ///
    /// Examples:
    ///   haven vcs
    Vcs,

    /// Show all template variables available in `.tmpl` files.
    ///
    /// Prints built-in variables (os, hostname, username, etc.) and custom
    /// variables from the `[data]` section of `haven.toml`.
    ///
    /// Useful for debugging templates or verifying that custom data variables
    /// are correctly set before running `haven apply`.
    ///
    /// Examples:
    ///   haven data
    Data,

    /// Find files in ~ that are not tracked by haven.
    ///
    /// Walks the home directory (or a specified path) and reports any files
    /// that have no corresponding entry in `source/`. Only dotfiles and
    /// dotdirs (names starting with `.`) are examined at the home root.
    ///
    /// High-noise directories (`.cache`, `.cargo`, `node_modules`, `.git`,
    /// `Library`, etc.) are automatically skipped.
    ///
    /// Examples:
    ///   haven unmanaged
    ///   haven unmanaged --path ~/.config
    ///   haven unmanaged --depth 2
    Unmanaged {
        /// Root path to walk. Defaults to `~`.
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,

        /// Maximum depth below the root. 0 means unlimited.
        #[arg(long, default_value = "3")]
        depth: usize,
    },

    /// Scan tracked source files for secrets, sensitive filenames, and credentials.
    ///
    /// Checks each tracked file for:
    ///   - Sensitive filename patterns (.env, id_rsa, .pem, credentials, etc.)
    ///   - Sensitive destination paths (~/.ssh/**, ~/.aws/credentials, ~/.kube/**, etc.)
    ///   - Content patterns (GitHub tokens, AWS keys, PEM keys, OpenAI keys, etc.)
    ///
    /// Exits 0 when clean, 1 when findings are reported.
    /// Add paths to `[security] allow` in haven.toml to suppress false positives.
    ///
    /// Examples:
    ///   haven security-scan
    ///   haven security-scan --entropy
    #[command(name = "security-scan")]
    SecurityScan {
        /// Also report high-entropy strings (opt-in: may produce false positives).
        #[arg(long)]
        entropy: bool,
    },

    /// Manage local telemetry: enable, disable, or annotate the telemetry log.
    ///
    /// Enable / disable writes `[telemetry] enabled = true/false` to `haven.toml`.
    /// Annotations write a structured entry to `~/.haven/telemetry.jsonl` regardless
    /// of whether telemetry is currently enabled.
    ///
    /// All annotation flags auto-generate sequential IDs:
    ///   --note     → N000001, N000002, …
    ///   --action   → A000001, A000002, …
    ///   --bug      → B000001, B000002, …
    ///   --question → Q000001, Q000002, …
    ///
    /// Without any flags, prints the current telemetry status.
    ///
    /// Examples:
    ///   haven telemetry --enable
    ///   haven telemetry --disable
    ///   haven telemetry --note "starting fresh config — prior data is from testing"
    ///   haven telemetry --action "testing chezmoi migration guide"
    ///   haven telemetry --bug "security scan flags ~/.ssh/id_rsa.pub despite allowlist"
    ///   haven telemetry --question "should allowlist use repo name or target name?"
    ///   haven telemetry --list
    ///   haven telemetry --list-bugs
    ///   haven telemetry --list-questions
    Telemetry {
        /// Enable telemetry by setting `[telemetry] enabled = true` in haven.toml.
        #[arg(long, conflicts_with_all = ["disable", "note", "action", "bug", "question", "list", "list_notes", "list_actions", "list_bugs", "list_questions"])]
        enable: bool,

        /// Disable telemetry by setting `[telemetry] enabled = false` in haven.toml.
        #[arg(long, conflicts_with_all = ["enable", "note", "action", "bug", "question", "list", "list_notes", "list_actions", "list_bugs", "list_questions"])]
        disable: bool,

        /// Append a note to the telemetry log. Auto-generates an ID (N000001, N000002, …).
        #[arg(long, conflicts_with_all = ["enable", "disable", "action", "bug", "question", "list", "list_notes", "list_actions", "list_bugs", "list_questions"])]
        note: Option<String>,

        /// Record an action you took. Auto-generates an ID (A000001, A000002, …).
        #[arg(long, conflicts_with_all = ["enable", "disable", "note", "bug", "question", "list", "list_notes", "list_actions", "list_bugs", "list_questions"])]
        action: Option<String>,

        /// Record a bug you found. Auto-generates an ID (B000001, B000002, …).
        #[arg(long, conflicts_with_all = ["enable", "disable", "note", "action", "question", "list", "list_notes", "list_actions", "list_bugs", "list_questions"])]
        bug: Option<String>,

        /// Record a question you have. Auto-generates an ID (Q000001, Q000002, …).
        #[arg(long, conflicts_with_all = ["enable", "disable", "note", "action", "bug", "list", "list_notes", "list_actions", "list_bugs", "list_questions"])]
        question: Option<String>,

        /// Print all telemetry log entries.
        #[arg(long, conflicts_with_all = ["enable", "disable", "note", "action", "bug", "question", "list_notes", "list_actions", "list_bugs", "list_questions"])]
        list: bool,

        /// Print only note entries.
        #[arg(long, conflicts_with_all = ["enable", "disable", "note", "action", "bug", "question", "list", "list_actions", "list_bugs", "list_questions"])]
        list_notes: bool,

        /// Print only action entries.
        #[arg(long, conflicts_with_all = ["enable", "disable", "note", "action", "bug", "question", "list", "list_notes", "list_bugs", "list_questions"])]
        list_actions: bool,

        /// Print only bug entries.
        #[arg(long, conflicts_with_all = ["enable", "disable", "note", "action", "bug", "question", "list", "list_notes", "list_actions", "list_questions"])]
        list_bugs: bool,

        /// Print only question entries.
        #[arg(long, conflicts_with_all = ["enable", "disable", "note", "action", "bug", "question", "list", "list_notes", "list_actions", "list_bugs"])]
        list_questions: bool,
    },

    /// Upgrade haven to the latest version.
    ///
    /// Downloads the latest release from GitHub, verifies the SHA256 checksum,
    /// and atomically replaces the running binary in place.
    ///
    /// Examples:
    ///   haven upgrade              # install the latest version
    ///   haven upgrade --check      # check without installing (exits 1 if update available)
    ///   haven upgrade --force      # reinstall even if already on latest
    Upgrade {
        /// Check whether an update is available without installing it.
        /// Exits 0 when up to date, 1 when an update is available.
        #[arg(long)]
        check: bool,

        /// Install the latest version even if the current version is already up to date.
        #[arg(long)]
        force: bool,
    },

    /// Import dotfiles from another dotfile manager.
    ///
    /// Reads the source manager's directory, decodes its naming conventions,
    /// and imports files into haven's source/ directory with generated module
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
    ///   haven import --from chezmoi
    ///   haven import --from chezmoi --source ~/my-chezmoi-dir --dry-run
    ///   haven import --from chezmoi --include-ignored-files
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
        /// The patterns are still written to `config/ignore`, so `haven apply`,
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

/// Parse `--vcs` string to a [`vcs::VcsBackend`], returning a user-facing error on bad input.
fn parse_vcs_flag(s: &str) -> anyhow::Result<vcs::VcsBackend> {
    match s.to_lowercase().as_str() {
        "git" => Ok(vcs::VcsBackend::Git),
        "jj"  => Ok(vcs::VcsBackend::Jj),
        other => anyhow::bail!("unknown --vcs value '{}'; use 'git' or 'jj'", other),
    }
}

/// Load the VCS backend from haven.toml, or return None if not set or parse fails.
fn vcs_from_config(repo: &std::path::Path) -> Option<vcs::VcsBackend> {
    use config::haven::HavenConfig;
    let cfg = HavenConfig::load(repo).ok()?;
    match cfg.vcs.backend.as_deref() {
        Some("git") => Some(vcs::VcsBackend::Git),
        Some("jj")  => Some(vcs::VcsBackend::Jj),
        _ => None,
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

/// Set `[telemetry] enabled` in `haven.toml` using `toml_edit` so all other
/// content (comments, formatting, other keys) is preserved.
fn set_telemetry_in_config(repo: &std::path::Path, enabled: bool) -> Result<()> {
    let path = repo.join("haven.toml");
    let text = if path.exists() {
        std::fs::read_to_string(&path)
            .with_context(|| format!("Cannot read {}", path.display()))?
    } else {
        String::new()
    };

    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .context("haven.toml contains invalid TOML")?;

    // `doc["telemetry"]["enabled"]` creates the [telemetry] table if absent.
    doc["telemetry"]["enabled"] = toml_edit::value(enabled);

    std::fs::write(&path, doc.to_string())
        .with_context(|| format!("Cannot write {}", path.display()))?;

    println!(
        "Telemetry {}. ({} updated)",
        if enabled { "enabled" } else { "disabled" },
        path.display()
    );
    Ok(())
}

/// Load the telemetry.enabled setting from haven.toml (best-effort, never panics).
fn try_load_telemetry_config() -> bool {
    (|| -> Option<bool> {
        let repo = repo_root().ok()?;
        let cfg = HavenConfig::load(&repo).ok()?;
        Some(cfg.telemetry.enabled)
    })()
    .unwrap_or(false)
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    // Completions don't need a repo — handle before repo_root() resolution.
    if let Commands::Completions { shell, cmd_name } = &cli.command {
        let name = cmd_name.as_deref().unwrap_or("haven");
        generate(*shell, &mut Cli::command(), name, &mut std::io::stdout());
        return Ok(());
    }

    // Telemetry annotations and --list* don't need a repo — handle before repo_root().
    match &cli.command {
        Commands::Telemetry { note: Some(text), .. } => {
            let id = telemetry::append_note(text)?;
            println!("Note {} recorded in ~/.haven/telemetry.jsonl", id);
            return Ok(());
        }
        Commands::Telemetry { action: Some(text), .. } => {
            let id = telemetry::append_typed("action", 'A', text)?;
            println!("Action {} recorded in ~/.haven/telemetry.jsonl", id);
            return Ok(());
        }
        Commands::Telemetry { bug: Some(text), .. } => {
            let id = telemetry::append_typed("bug", 'B', text)?;
            println!("Bug {} recorded in ~/.haven/telemetry.jsonl", id);
            return Ok(());
        }
        Commands::Telemetry { question: Some(text), .. } => {
            let id = telemetry::append_typed("question", 'Q', text)?;
            println!("Question {} recorded in ~/.haven/telemetry.jsonl", id);
            return Ok(());
        }
        Commands::Telemetry { list: true, .. } => {
            telemetry::list(None)?;
            return Ok(());
        }
        Commands::Telemetry { list_notes: true, .. } => {
            telemetry::list(Some("note"))?;
            return Ok(());
        }
        Commands::Telemetry { list_actions: true, .. } => {
            telemetry::list(Some("action"))?;
            return Ok(());
        }
        Commands::Telemetry { list_bugs: true, .. } => {
            telemetry::list(Some("bug"))?;
            return Ok(());
        }
        Commands::Telemetry { list_questions: true, .. } => {
            telemetry::list(Some("question"))?;
            return Ok(());
        }
        _ => {}
    }

    let repo = match &cli.dir {
        Some(d) => d.clone(),
        None => repo_root()?,
    };

    // State and backup directories live outside the repo (not committed).
    let state_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".haven");
    let backup_dir = state_dir.join("backups");
    let claude_dir = std::env::var("HAVEN_CLAUDE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".claude")
        });
    match &cli.command {
        Commands::SourcePath => {
            println!("{}", repo.display());
            return Ok(());
        }

        Commands::Init {
            source,
            branch,
            apply,
            profile,
            vcs: vcs_flag,
        } => {
            // VCS resolution (and the jj prompt) only matters when cloning a source.
            // Scaffold mode (`haven init` with no source) doesn't clone anything,
            // so skip the prompt entirely to avoid asking about jj before erroring
            // out on "already initialized".
            let vcs_backend = if source.is_some() {
                let cli_backend = vcs_flag.as_deref().map(parse_vcs_flag).transpose()?;
                let config_backend = vcs_from_config(&repo);
                let resolved = vcs::resolve(cli_backend, config_backend, None)?;
                resolved.map(|r| r.backend).unwrap_or(vcs::VcsBackend::Git)
            } else {
                vcs::VcsBackend::Git
            };
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
                vcs_backend,
            })?;
        }

        Commands::List { files, brews, ai, profile, filter, count } => {
            let resolved = resolve_profile(profile.as_deref(), &state_dir);
            commands::list::run(&commands::list::ListOptions {
                repo_root: &repo,
                profile: &resolved,
                show_files: *files,
                show_brews: *brews,
                show_ai: *ai,
                filter: filter.as_deref(),
                count: *count,
            })?;
        }

        Commands::Add { file, link, apply, update } => {
            commands::add::run(&repo, file, *link, *apply, *update)?;
        }

        Commands::Update { file } => {
            commands::add::run(&repo, file, false, false, true)?;
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
            on_conflict,
            apply_externals,
            run_scripts,
            remove_unreferenced_brews,
            interactive,
            zap,
            vcs: vcs_flag,
            dest,
        } => {
            let resolved = resolve_profile(profile.as_deref(), &state_dir);
            let dest_root_buf = dest
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/"));
            let none_specified = !files && !brews && !ai;
            let cli_backend = vcs_flag.as_deref().map(parse_vcs_flag).transpose()?;
            let config_backend = vcs_from_config(&repo);
            let vcs_resolved = vcs::resolve(cli_backend, config_backend, Some(&repo))?;
            let vcs_backend = vcs_resolved.map(|r| r.backend).unwrap_or(vcs::VcsBackend::Git);
            let on_conflict_mode: commands::apply::OnConflict = on_conflict.parse()?;
            let outcome = commands::apply::run(&commands::apply::ApplyOptions {
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
                remove_unreferenced_brews: *remove_unreferenced_brews || *interactive || *zap,
                interactive: *interactive,
                zap: *zap,
                vcs_backend,
                on_conflict: on_conflict_mode,
            })?;
            if outcome.had_conflict_skips {
                std::process::exit(1);
            }
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
                state_dir: &state_dir,
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
            AiAction::AddLocal { path, name, platforms } => {
                commands::ai::add_local(&commands::ai::AddLocalOptions {
                    repo_root: &repo,
                    path,
                    name: name.as_deref(),
                    platforms,
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
                    repo_root: &repo,
                    query,
                    limit: *limit,
                })?;
            }
            AiAction::Scan { path, dry_run } => {
                commands::ai::scan(&commands::ai::ScanOptions {
                    repo_root: &repo,
                    state_dir: &state_dir,
                    dir: path,
                    dry_run: *dry_run,
                })?;
            }
            AiAction::Backends => {
                commands::ai::backends(&commands::ai::BackendsOptions {
                    repo_root: &repo,
                })?;
            }
        },

        Commands::Vcs => {
            let config_backend = vcs_from_config(&repo);
            let resolved = vcs::resolve(None, config_backend, Some(&repo))?;
            match resolved {
                Some(ref r) => vcs::print_status(r, &repo),
                None => {
                    // User aborted the detection prompt — nothing to print.
                    return Ok(());
                }
            }
        }

        Commands::Data => {
            let config = HavenConfig::load(&repo).unwrap_or_default();
            let ctx = template::TemplateContext::from_env("default", &repo, config.data);
            println!("os        = {}", ctx.os);
            println!("hostname  = {}", ctx.hostname);
            println!("username  = {}", ctx.username);
            println!("home_dir  = {}", ctx.home_dir);
            println!("source_dir = {}", ctx.source_dir);
            if ctx.data.is_empty() {
                println!();
                println!("No [data] variables set in haven.toml.");
                println!("Add them like:");
                println!("  [data]");
                println!("  host = \"my-laptop\"");
            } else {
                println!();
                let mut keys: Vec<&String> = ctx.data.keys().collect();
                keys.sort();
                for k in keys {
                    println!("data.{:<20} = {}", k, ctx.data[k]);
                }
            }
        }

        Commands::Unmanaged { path, depth } => {
            let root = path.as_deref();
            commands::unmanaged::run(&commands::unmanaged::UnmanagedOptions {
                repo_root: &repo,
                root,
                depth: *depth,
            })?;
        }

        Commands::SecurityScan { entropy } => {
            commands::security_scan::run(&commands::security_scan::ScanOptions {
                repo_root: &repo,
                entropy: *entropy,
            })?;
        }

        Commands::Completions { .. } => unreachable!(), // handled before repo resolution
        // Annotations and --list* are handled above; --enable/--disable and bare status fall through.
        Commands::Telemetry { note: Some(_), .. }
        | Commands::Telemetry { action: Some(_), .. }
        | Commands::Telemetry { bug: Some(_), .. }
        | Commands::Telemetry { question: Some(_), .. }
        | Commands::Telemetry { list: true, .. }
        | Commands::Telemetry { list_notes: true, .. }
        | Commands::Telemetry { list_actions: true, .. }
        | Commands::Telemetry { list_bugs: true, .. }
        | Commands::Telemetry { list_questions: true, .. } => unreachable!(),
        Commands::Telemetry { enable, disable, .. } => {
            if *enable {
                set_telemetry_in_config(&repo, true)?;
            } else if *disable {
                set_telemetry_in_config(&repo, false)?;
            } else {
                let is_on = telemetry::is_enabled(try_load_telemetry_config());
                println!(
                    "Telemetry is currently {}.",
                    if is_on { "enabled" } else { "disabled" }
                );
                println!("  haven telemetry --enable                    # turn on");
                println!("  haven telemetry --disable                   # turn off");
                println!("  haven telemetry --note \"<text>\"             # record a note (N000001…)");
                println!("  haven telemetry --action \"<text>\"           # record an action (A000001…)");
                println!("  haven telemetry --bug \"<text>\"              # record a bug (B000001…)");
                println!("  haven telemetry --question \"<text>\"         # record a question (Q000001…)");
                println!("  haven telemetry --list                      # show all entries");
                println!("  haven telemetry --list-notes                # show notes only");
                println!("  haven telemetry --list-actions              # show actions only");
                println!("  haven telemetry --list-bugs                 # show bugs only");
                println!("  haven telemetry --list-questions            # show questions only");
            }
        }

        Commands::Upgrade { check, force } => {
            commands::upgrade::run(&commands::upgrade::UpgradeOptions {
                check_only: *check,
                force: *force,
            })?;
        }

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
