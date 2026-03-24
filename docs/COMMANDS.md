# haven Command Reference

## Quick Reference

```
haven init [source] [--branch <b>] [--apply] [--profile <p>]
haven list [--profile <p>] [--files] [--brews] [--ai]
haven add <file> [--link] [--apply] [--update]
haven remove <file> [--dry-run]
haven apply [--profile <p>] [--module <m>] [--dry-run]
            [--files] [--brews] [--ai]
            [--apply-externals]
            [--remove-unreferenced-brews] [--interactive] [--zap]
haven diff  [--profile <p>] [--module <m>]
            [--files] [--brews] [--ai]
            [--stat] [--color always|never|auto]
haven status [--profile <p>] [--files] [--brews] [--ai]
haven source-path
haven brew install <name> [--cask] [--module <m>]
haven brew uninstall <name> [--cask]
haven import --from chezmoi [--source <dir>] [--dry-run]
             [--include-ignored-files]
haven ai discover
haven ai add <source> [--name <n>] [--platforms <p>] [--deploy symlink|copy]
haven ai fetch [<name>]
haven ai update [<name>]
haven ai remove <name> [--yes]
haven ai search <query> [--limit <n>]
haven ai scan <path> [--dry-run]
haven ai backends
haven data
haven unmanaged [--path <p>] [--depth <n>]
haven upgrade [--check] [--force]
haven telemetry [--enable] [--disable]
                [--note|--action|--bug|--question "<message>"]
                [--list] [--list-notes] [--list-actions] [--list-bugs] [--list-questions]
haven security-scan [--entropy]
haven completions fish|zsh|bash
```

---

## Global Options

| Option | Default | Description |
|--------|---------|-------------|
| `--dir <path>` | `~/.local/share/haven` | haven repo directory. Also read from `HAVEN_DIR` env var. |

---

## `haven source-path`

Print the absolute path to the haven repo directory and exit. Useful in scripts
and shell aliases.

```
haven source-path
```

```sh
# Examples:
cd $(haven source-path)
alias haven-edit='$EDITOR $(haven source-path)/haven.toml'
```

---

## `haven init`

Create or clone a haven repository. **Use this once, on first-time setup.**
For subsequent re-provisioning of an already-initialised machine, use `haven apply`.

Without a source, creates a blank scaffold at `--dir`. With a source, clones the
repository and optionally applies it immediately.

```
haven init [source] [--branch <b>] [--apply] [--profile <p>]
```

| Argument/Option | Description |
|-----------------|-------------|
| `source` | Optional. Git URL or `gh:owner/repo[@ref]`. Omit to create a blank scaffold. |
| `--branch <b>` | Branch to clone. Overrides any `@ref` in source. |
| `--apply` | Apply the cloned repo immediately after cloning. Requires a source. |
| `--profile <p>` | Profile to apply. Requires `--apply`. |

---

## `haven list`

List tracked files, Homebrew packages, and AI skills.

```
haven list [--profile <p>] [--files] [--brews] [--ai]
```

Without filter flags, all three sections are shown under `[files]`, `[brew]`,
and `[ai]` headers. Pass one or more flags to show only those sections.

```
[files]
~/.zshrc
~/.gitconfig          (template)
~/.ssh/config         (private)
~/.local/bin/delta    (extfile)
~/.config/nvim        (extdir)
~/.vimrc              (symlink)
~/.env.example        (create-only)

[brew]
brew bat
brew ripgrep
cask ghostty

[ai]
skill pomodoro  (gh:user/pomodoro)
```

| Option | Description |
|--------|-------------|
| `--profile <p>` | Scope to a specific profile. Default: active profile. |
| `--files` | Show tracked files only. |
| `--brews` | Show Homebrew packages only (master + module Brewfiles). |
| `--ai` | Show AI skills only. |

**File annotations** (appear in parentheses):

| Annotation | Meaning |
|------------|---------|
| `template` | Rendered through Tera before writing |
| `symlink` | Destination is a symlink back into `source/` |
| `private` | Destination is chmod 0600 (0700 for directories) |
| `executable` | Destination is chmod 0755 |
| `extdir` | Remote git repo cloned into destination on apply |
| `extfile` | Remote file/archive downloaded to destination on apply |
| `create-only` | Only written if destination does not already exist |
| `exact` | Untracked files in destination directory are removed on apply |

Files matching patterns in `config/ignore` are excluded from the list.

---

## `haven data`

Show all template variables available in `.tmpl` files.

```
haven data
```

Prints built-in variables (os, hostname, username, etc.) and custom variables
from the `[data]` section of `haven.toml`:

```
os         = macos
hostname   = my-laptop
username   = alice
home_dir   = /Users/alice
source_dir = /Users/alice/.local/share/haven

data.host                = my-laptop
data.kanata_path         = /usr/local/bin/kanata
```

Custom variables are defined in `haven.toml`:

```toml
[data]
host = "my-laptop"
kanata_path = "/usr/local/bin/kanata"
```

And used in `.tmpl` files:

```
export HOST={{ data.host }}
{{ if data.host }}# host-specific config{% endif %}
```

---

## `haven add`

Start tracking a dotfile by copying it into `source/`.

For directories: if the directory is a git repo, interactively prompts to track it as
an external clone (re-cloned on apply) or recursively copy its files.

```
haven add <file> [--link] [--apply] [--update]
```

| Argument/Option | Description |
|-----------------|-------------|
| `file` | Path to the file or directory to track (e.g. `~/.zshrc`). |
| `--link` | Track as a symlink: on apply, the destination will be symlinked back to `source/` rather than copied. Use for files that apps manage themselves (e.g. VS Code settings). |
| `--apply` | Immediately install after adding: replace the original with a symlink back into `source/`. Only valid with `--link`. |
| `--update` | Re-copy the file into `source/` even if it is already tracked. Without this flag, adding an already-tracked file is an error. |

---

## `haven remove`

Stop tracking a dotfile by removing it from `source/`. The live file on disk is
**not** touched — only the `source/` copy is deleted.

```
haven remove <file> [--dry-run]
```

| Argument/Option | Description |
|-----------------|-------------|
| `file` | Destination path to stop tracking (e.g. `~/.zshrc`). |
| `--dry-run` | Print what would be removed without deleting any files. |

---

## `haven apply`

Apply tracked files and packages to this machine.

Copies source files to their destinations, installs Homebrew packages, runs mise,
and deploys AI skills. Backs up any existing files first.

By default all sections are applied. Use `--files`, `--brews`, and/or `--ai` to
apply only specific sections.

```
haven apply [--profile <p>] [--module <m>] [--dry-run]
            [--files] [--brews] [--ai]
            [--apply-externals]
            [--remove-unreferenced-brews] [--interactive] [--zap]
```

| Option | Description |
|--------|-------------|
| `--profile <p>` | Profile to apply. Default: last-used profile from state, or `default`. |
| `--module <m>` | Scope brew and mise operations to this module. **Dotfiles in source/ are always applied globally — this flag does not filter file operations.** |
| `--dry-run` | Print the plan without writing any files. |
| `--files` | Apply dotfile copies/symlinks. |
| `--brews` | Run `brew bundle install`. |
| `--ai` | Deploy AI skills from `ai/skills/*/skill.toml`. |
| `--apply-externals` | Pull (update) existing `extdir_` git clones in addition to cloning missing ones. Without this, existing clones are left as-is. |
| *(AI injection)* | When `--ai` is active, skill snippets from `ai/skills/<name>/all.md` and `ai/skills/<name>/<platform>.md` are injected into platform config files (e.g. `~/.claude/CLAUDE.md`) between `<!-- haven managed start -->` / `<!-- haven managed end -->` markers. If the config file has no markers and the session is interactive, you are prompted to add them. |
| `--remove-unreferenced-brews` | After installing, uninstall any leaf formula/cask not referenced by any active Brewfile. |
| `--interactive` | Like `--remove-unreferenced-brews` but prompts for confirmation before removing. Implies `--remove-unreferenced-brews`. |
| `--zap` | Like `--remove-unreferenced-brews` but also passes `--zap` to `brew uninstall --cask`, removing associated app data and support files. Implies `--remove-unreferenced-brews`. |

**Section filter behavior:** If none of `--files/--brews/--ai` are given, all sections
are applied. If any are given, only those sections run.

---

## `haven diff`

Show the diff between tracked source files/packages and live state.

Exits 0 when everything is up to date, 1 when drift is found.

By default all sections are diffed. Use `--files`, `--brews`, and/or `--ai` to
inspect only specific sections.

```
haven diff [--profile <p>] [--module <m>]
           [--files] [--brews] [--ai]
           [--stat] [--color always|never|auto]
```

| Option | Description |
|--------|-------------|
| `--profile <p>` | Profile to diff. Default: last-used profile from state, or `default`. |
| `--module <m>` | Scope brew and AI diff to this module. **Dotfiles in source/ are always diffed globally.** |
| `--files` | Diff dotfile copies/symlinks. |
| `--brews` | Diff Homebrew packages. |
| `--ai` | Diff AI skills deployment state. |
| `--stat` | Show a summary (file names + change counts) instead of full diff content. |
| `--color <mode>` | `always`, `never`, or `auto` (default: auto — color when stdout is a tty). |

**Section filter behavior:** If none of `--files/--brews/--ai` are given, all sections
are diffed. If any are given, only those sections run.

---

## `haven status`

Show drift between tracked source files and live destinations. More concise than `diff`.

Drift markers: `✓` clean  `M` modified  `?` missing  `!` source missing

By default all sections are shown. Use `--files`, `--brews`, and/or `--ai` to
inspect only specific sections.

```
haven status [--profile <p>] [--files] [--brews] [--ai]
```

| Option | Description |
|--------|-------------|
| `--profile <p>` | Profile to check. Default: last-used profile from state, or `default`. |
| `--files` | Show dotfile drift only. |
| `--brews` | Show Homebrew package drift only. |
| `--ai` | Show AI skill drift (from `ai/skills.toml`) only. |

---

## `haven brew`

Run `brew install`/`uninstall` and keep Brewfiles in sync.

Use these instead of bare `brew install` when you want changes to persist across
machines — the formula is automatically added to or removed from the Brewfile(s).

### `haven brew install`

```
haven brew install <name> [--cask] [--module <m>]
```

| Argument/Option | Description |
|-----------------|-------------|
| `name` | Formula or cask name (e.g. `ripgrep`, `iterm2`). |
| `--cask` | Install as a cask (GUI apps, fonts, etc.). |
| `--module <m>` | Record in this module's `brew/Brewfile.<m>`. Default: master `brew/Brewfile`. |

### `haven brew uninstall`

```
haven brew uninstall <name> [--cask]
```

| Argument/Option | Description |
|-----------------|-------------|
| `name` | Formula or cask name. |
| `--cask` | Uninstall as a cask. |

Removes the formula from **all** Brewfiles in the repo, then runs `brew uninstall`.

---

## `haven import`

Import dotfiles from another dotfile manager (one-time migration).

Currently only `chezmoi` is supported as a source format.

```
haven import --from chezmoi [--source <dir>] [--dry-run] [--include-ignored-files]
```

| Option | Description |
|--------|-------------|
| `--from <manager>` | Source format. Currently only `chezmoi`. |
| `--source <dir>` | Path to the source manager's directory. Auto-detected via `chezmoi source-path` if not given. |
| `--dry-run` | Print what would be imported without writing any files. |
| `--include-ignored-files` | Import files matching `.chezmoiignore` patterns instead of skipping them. The patterns are still written to `config/ignore`. |

**What is imported:** Plain files, `private_` prefix, `executable_` prefix,
`symlink_` prefix, `*.tmpl` files (Go template syntax converted to Tera),
`.chezmoiexternal.toml` git-repo entries (converted to `extdir_` markers).

**What is skipped:** `run_*` / `once_*` / `run_once_*` install scripts; `exact_*` / `create_*` /
`modify_*`; `.chezmoi*` / `chezmoistate.*`; patterns in `.chezmoiignore` (unless
`--include-ignored-files`).

---

## `haven ai`

Manage AI agent skills across platforms. Skills are declared as directories under
`ai/skills/<name>/` (one `skill.toml` per skill) and deployed to platform skill
directories (e.g. `~/.claude/skills/`) on `haven apply`.

### `haven ai discover`

Scan this machine for installed AI agent platforms and offer to update `ai/platforms.toml`.

```
haven ai discover
```

### `haven ai add`

Add a skill declaration to `ai/skills/<name>/skill.toml` and create a blank
`all.md` snippet stub. Does **not** deploy the skill; run `haven apply --ai`
afterward to deploy.

```
haven ai add <source> [--name <n>] [--platforms <p>] [--deploy symlink|copy]
```

| Argument/Option | Description |
|-----------------|-------------|
| `source` | Skill source: `gh:owner/repo[/subpath][@ref]` or `dir:~/path`. |
| `--name <n>` | Local name for the skill. Default: inferred from source. |
| `--platforms <p>` | Target platforms: `all`, `cross-client`, or comma-separated IDs. Default: `all`. |
| `--deploy <method>` | `symlink` (default) or `copy`. |

### `haven ai fetch`

Download skills into the local cache (`~/.haven/skills/`) without deploying them.
Respects the lock file — already-cached skills at the pinned SHA are skipped.

```
haven ai fetch [<name>]
```

| Argument | Description |
|----------|-------------|
| `name` | Optional. Skill name to fetch. Omit to fetch all skills. |

### `haven ai update`

Fetch the latest version of skills from their sources, updating the lock file SHAs.
Does **not** deploy; run `haven apply --ai` afterward to deploy updated skills.

```
haven ai update [<name>]
```

| Argument | Description |
|----------|-------------|
| `name` | Optional. Skill name to update. Omit to update all skills. |

**Difference from `fetch`:** `fetch` respects the current lock SHA (no-op if already cached).
`update` clears the lock SHA and pulls the latest version, then records the new SHA.

### `haven ai remove`

Remove a skill directory (`ai/skills/<name>/`) and optionally remove it from
platform skill directories.

```
haven ai remove <name> [--yes]
```

| Argument/Option | Description |
|-----------------|-------------|
| `name` | Skill name (directory name under `ai/skills/`). |
| `--yes` | Skip confirmation prompts. |

### `haven ai search`

Search the [skills.sh](https://skills.sh) registry for available skills.

```
haven ai search <query> [--limit <n>]
```

| Argument/Option | Description |
|-----------------|-------------|
| `query` | Search term (e.g. `pdf`, `browser`, `git`). |
| `--limit <n>` | Maximum number of results to show. Default: 10. |

Results show the skill source in `gh:owner/repo/skill` format and install count.
Copy the source and pass it to `haven ai add` to start tracking it.

### `haven ai scan`

Interactively scan an existing skills directory and offer to add any unmanaged
skills to `ai/skills.toml`.

```
haven ai scan <path> [--dry-run]
```

| Argument/Option | Description |
|-----------------|-------------|
| `path` | Path to a skills directory to scan (e.g. `~/.claude/skills`). |
| `--dry-run` | Show what would be added without modifying `ai/skills.toml`. |

For each unmanaged skill found, haven tries to identify its GitHub source via:
1. Git remote detection (for skills that are git clones)
2. skills.sh registry search (fuzzy match on the skill directory name)

If a source is found it is shown as a suggestion; the user is prompted to
confirm (`y`), edit the source (`e`), or skip (`n`) each skill. Skills that
are already tracked in `ai/skills.toml` are silently skipped.

### `haven ai backends`

List all known skill backends with their availability status. The currently
active backend (from `ai/config.toml`, or `native` by default) is marked.

```
haven ai backends
```

Output example:

```
Skill backends:
  ✓ native   (active) — built-in, zero dependencies
  ✗ skillkit — runner 'npx' not found — install Node.js or set runner = "bunx"
    akm      — not yet implemented
```

See [Skill Backends](skill-backends.md) in the reference for configuration
details and switching instructions.

---

## `haven telemetry`

Manage local telemetry: enable, disable, annotate, or query the telemetry log.

```
haven telemetry [--enable] [--disable]
                [--note|--action|--bug|--question "<message>"]
                [--list] [--list-notes] [--list-actions] [--list-bugs] [--list-questions]
```

Without flags, prints the current telemetry status (enabled/disabled).

| Flag | Description |
|------|-------------|
| `--enable` | Set `[telemetry] enabled = true` in `haven.toml`. |
| `--disable` | Set `[telemetry] enabled = false` in `haven.toml`. |
| `--note "<text>"` | Append a free-form note (`kind: "note"`, ID prefix `N`). |
| `--action "<text>"` | Record a deliberate action taken (`kind: "action"`, ID prefix `A`). |
| `--bug "<text>"` | Record a bug observed (`kind: "bug"`, ID prefix `B`). |
| `--question "<text>"` | Record a question for later investigation (`kind: "question"`, ID prefix `Q`). |
| `--list` | Print all telemetry entries to stdout. |
| `--list-notes` | Print only `kind: "note"` entries. |
| `--list-actions` | Print only `kind: "action"` entries. |
| `--list-bugs` | Print only `kind: "bug"` entries. |
| `--list-questions` | Print only `kind: "question"` entries. |

Annotation flags (`--note`, `--action`, `--bug`, `--question`) always write to
`~/.haven/telemetry.jsonl` regardless of whether telemetry is enabled. Each
annotation is assigned an auto-generated, sequenced ID (e.g. `B000001`,
`B000002`) that is printed to stdout after the command.

```sh
# Turn telemetry on or off
haven telemetry --enable
haven telemetry --disable

# Check current status
haven telemetry

# Annotate the log
haven telemetry --note "starting fresh config — prior runs were testing"
haven telemetry --bug "apply --brews not running module brewfiles"
haven telemetry --action "reset brewfile to last known good state"
haven telemetry --question "why does brew leaves return tap-qualified names?"

# List annotations
haven telemetry --list-bugs
haven telemetry --list-notes
haven telemetry --list
```

Entries are JSONL and easy to filter directly:

```sh
jq 'select(.kind=="bug")' ~/.haven/telemetry.jsonl
```

---

## `haven upgrade`

Upgrade haven to the latest version by downloading the release from GitHub,
verifying its SHA256 checksum, and atomically replacing the running binary.

```
haven upgrade [--check] [--force]
```

| Flag | Description |
|------|-------------|
| `--check` | Check whether an update is available without installing it. Exits 0 when up to date, exits 1 when an update is available. |
| `--force` | Install the latest version even if the current version is already the latest. |

```sh
# Check for an update (CI-friendly — exits 1 if update is available)
haven upgrade --check

# Upgrade to the latest version
haven upgrade

# Reinstall the current version (useful after path changes)
haven upgrade --force
```

**How it works:**

1. Queries the GitHub releases API for the latest tag.
2. Downloads the platform-specific tarball and `SHA256SUMS` file.
3. Verifies the tarball checksum before extracting.
4. Atomically replaces the running binary (write to `haven.new`, then rename).

Supported platforms: macOS (arm64, x86_64), Linux (x86_64, aarch64, armv7, i686).

---

## `haven unmanaged`

Find files in `~` that are not tracked by haven.

```
haven unmanaged                    # scan ~ up to depth 3
haven unmanaged --path ~/.config   # scan a specific directory
haven unmanaged --depth 5          # scan deeper
```

Walks the home directory and reports any files that have no corresponding entry
in `source/`. At the home root, only dotfiles and dotdirs (names starting with
`.`) are examined — `Documents/`, `Downloads/`, `Projects/` etc. are skipped.

High-noise directories are automatically excluded:

| Category | Skipped |
|----------|---------|
| VCS | `.git`, `.jj`, `.hg`, `.svn` |
| Caches | `.cache`, `.npm`, `.cargo`, `.rustup`, `node_modules` |
| haven state | `.haven` |
| macOS | `Library`, `.Trash`, `.Spotlight-V100` |
| App state | `.android`, `.kube`, `.docker`, `.minikube` |

Example output:

```
~/.gitconfig
~/.zshenv
~/.config/bat/config
~/.local/bin/my-script
```

Pipe to `haven add` to start tracking discovered files:

```sh
haven unmanaged | head -5
haven add ~/.config/bat/config
```

---

## `haven security-scan`

Scan all tracked source files for secrets, sensitive filenames, and credential paths.

Checks each tracked file for:
- **Filename patterns** — `.env`, `id_rsa`, `.pem`, `credentials`, `secrets`, `.key`, etc.
- **Path patterns** — `~/.aws/credentials`, `~/.kube/**`, `~/.ssh/**`, `~/.config/gh/hosts.yml`, `~/.docker/config.json`, etc.
- **Content patterns** — GitHub tokens (`ghp_`/`ghs_`/`github_pat_`), AWS access keys (`AKIA…`), PEM private keys, OpenAI keys (`sk-…`), Anthropic keys (`sk-ant-…`), generic password/secret assignments
- **High-entropy strings** — opt-in via `--entropy` (disabled by default to reduce false positives)

Exits 0 when clean, 1 when findings are reported.

```
haven security-scan [--entropy]
```

| Option | Description |
|--------|-------------|
| `--entropy` | Also flag high-entropy strings (≥16 chars, Shannon entropy >4.5 bits/char). Opt-in: may produce false positives on base64 data. |

**Suppressing false positives** — add paths to `[security] allow` in `haven.toml`:

```toml
[security]
allow = [
  "~/.config/gh/hosts.yml",     # intentionally tracked
  "~/.config/gcloud/**",        # managed by gcloud CLI, not a secret
]
```

Patterns follow the same glob syntax as `config/ignore` (`*` matches within a path segment, `**` crosses separators).

**Integration with `haven add`** — when a file is added with `haven add`, its content is automatically scanned. If sensitive patterns are found, you are prompted before the file is saved to `source/`. Declining removes it immediately with no partial state left behind.

---

## `haven completions`

Print a shell completion script to stdout.

```
haven completions <shell>
```

| Argument | Description |
|----------|-------------|
| `shell` | `fish`, `zsh`, or `bash` |

**Setup:**

```sh
# Fish — write once to the completions directory:
haven completions fish > ~/.config/fish/completions/haven.fish

# Zsh — add to ~/.zshrc:
source <(haven completions zsh)

# Bash — add to ~/.bashrc:
source <(haven completions bash)
```

---

## Concepts

### Profiles

A profile selects which modules are active. Declared in `haven.toml` under `[profile.<name>]`.
The last-used profile is saved in state and reused automatically unless overridden with `--profile`.

### Sections: `--files`, `--brews`, `--ai`

Three commands — `apply`, `diff`, and `status` — operate on sections:

| Section | What it covers |
|---------|---------------|
| `--files` | Dotfiles in `source/` and external git clones (`extdir_*`) |
| `--brews` | Homebrew packages declared in Brewfiles |
| `--ai` | Skills declared in `ai/skills/*/skill.toml` |

### Modules

Modules group Homebrew packages and mise tool configs. Declared in
`modules/<name>.toml`. The `--module` flag on `apply` and `diff` scopes
brew/mise operations to a single module. **File operations are always global.**

### Skill sources

| Prefix | Example | Description |
|--------|---------|-------------|
| `gh:` | `gh:anthropics/skills/pdf-processing@v1` | GitHub repo or subdirectory. Optional `@ref` for a branch/tag. |
| `dir:` | `dir:~/projects/my-skill` | Local directory. Read directly, not cached. |

### Skill platforms

| Value | Meaning |
|-------|---------|
| `"all"` | All active platforms in `ai/platforms.toml`, excluding `cross-client` |
| `"cross-client"` | Only the cross-client platform (`~/.agents/skills/`) |
| `["claude-code", "codex"]` | Explicit list, filtered to active platforms |

### Deploy methods

| Method | Behavior |
|--------|----------|
| `symlink` (default) | Creates an absolute symlink `{skills_dir}/{name}` → cache dir. Updates instantly when cache is refreshed. |
| `copy` | Copies the skill directory to `{skills_dir}/{name}`. Required for platforms that don't follow symlinks. |

### Supply chain protection

Every `gh:` skill source is pinned by SHA in `haven.lock`. On cache miss, the
freshly-fetched SHA is compared against the recorded lock entry. A mismatch is a
hard error — run `haven ai update <name>` to explicitly accept the changed content.

### Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `HAVEN_DIR` | `~/.local/share/haven` | Repo root directory |
| `HAVEN_CLAUDE_DIR` | `~/.claude` | Claude Code directory (skills, CLAUDE.md) |
| `HAVEN_TELEMETRY` | unset | `1` to enable local telemetry, `0` to force-disable |
