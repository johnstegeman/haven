# dfiles Command Reference

## Quick Reference

```
dfiles init [source] [--branch <b>] [--apply] [--profile <p>]
dfiles list
dfiles add <file> [--link] [--apply] [--update]
dfiles remove <file> [--dry-run]
dfiles apply [--profile <p>] [--module <m>] [--dry-run]
            [--files] [--brews] [--ai]
            [--apply-externals]
            [--remove-unreferenced-brews] [--interactive]
dfiles diff  [--profile <p>] [--module <m>]
            [--files] [--brews] [--ai]
            [--stat] [--color always|never|auto]
dfiles status [--profile <p>] [--files] [--brews] [--ai]
dfiles source-path
dfiles brew install <name> [--cask] [--module <m>]
dfiles brew uninstall <name> [--cask]
dfiles import --from chezmoi [--source <dir>] [--dry-run]
             [--include-ignored-files]
dfiles ai discover
dfiles ai add <source> [--name <n>] [--platforms <p>] [--deploy symlink|copy]
dfiles ai fetch [<name>]
dfiles ai update [<name>]
dfiles ai remove <name> [--yes]
dfiles ai search <query> [--limit <n>]
dfiles ai scan <path> [--dry-run]
dfiles upgrade [--check] [--force]
dfiles telemetry [--enable] [--disable] [--note "<message>"]
dfiles security-scan [--entropy]
dfiles completions fish|zsh|bash
```

---

## Global Options

| Option | Default | Description |
|--------|---------|-------------|
| `--dir <path>` | `~/.local/share/dfiles` (XDG default; `~/dfiles` if it exists) | dfiles repo directory. Also read from `DFILES_DIR` env var. |

---

## `dfiles source-path`

Print the absolute path to the dfiles repo directory and exit. Useful in scripts
and shell aliases.

```
dfiles source-path
```

```sh
# Examples:
cd $(dfiles source-path)
alias dfiles-edit='$EDITOR $(dfiles source-path)/dfiles.toml'
```

---

## `dfiles init`

Create or clone a dfiles repository. **Use this once, on first-time setup.**
For subsequent re-provisioning of an already-initialised machine, use `dfiles apply`.

Without a source, creates a blank scaffold at `--dir`. With a source, clones the
repository and optionally applies it immediately.

```
dfiles init [source] [--branch <b>] [--apply] [--profile <p>]
```

| Argument/Option | Description |
|-----------------|-------------|
| `source` | Optional. Git URL or `gh:owner/repo[@ref]`. Omit to create a blank scaffold. |
| `--branch <b>` | Branch to clone. Overrides any `@ref` in source. |
| `--apply` | Apply the cloned repo immediately after cloning. Requires a source. |
| `--profile <p>` | Profile to apply. Requires `--apply`. |

---

## `dfiles list`

List all tracked files with their decoded destination paths.

```
dfiles list
```

Prints one line per tracked file. Flag annotations appear in parentheses when
present.

```
~/.zshrc
~/.gitconfig          (template)
~/.ssh/config         (private)
~/.local/bin/delta    (extfile)
~/.config/nvim        (extdir)
~/.vimrc              (symlink)
~/.env.example        (create-only)
```

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

## `dfiles add`

Start tracking a dotfile by copying it into `source/`.

For directories: if the directory is a git repo, interactively prompts to track it as
an external clone (re-cloned on apply) or recursively copy its files.

```
dfiles add <file> [--link] [--apply] [--update]
```

| Argument/Option | Description |
|-----------------|-------------|
| `file` | Path to the file or directory to track (e.g. `~/.zshrc`). |
| `--link` | Track as a symlink: on apply, the destination will be symlinked back to `source/` rather than copied. Use for files that apps manage themselves (e.g. VS Code settings). |
| `--apply` | Immediately install after adding: replace the original with a symlink back into `source/`. Only valid with `--link`. |
| `--update` | Re-copy the file into `source/` even if it is already tracked. Without this flag, adding an already-tracked file is an error. |

---

## `dfiles remove`

Stop tracking a dotfile by removing it from `source/`. The live file on disk is
**not** touched — only the `source/` copy is deleted.

```
dfiles remove <file> [--dry-run]
```

| Argument/Option | Description |
|-----------------|-------------|
| `file` | Destination path to stop tracking (e.g. `~/.zshrc`). |
| `--dry-run` | Print what would be removed without deleting any files. |

---

## `dfiles apply`

Apply tracked files and packages to this machine.

Copies source files to their destinations, installs Homebrew packages, runs mise,
and deploys AI skills. Backs up any existing files first.

By default all sections are applied. Use `--files`, `--brews`, and/or `--ai` to
apply only specific sections.

```
dfiles apply [--profile <p>] [--module <m>] [--dry-run]
            [--files] [--brews] [--ai]
            [--apply-externals]
            [--remove-unreferenced-brews] [--interactive]
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
| *(AI injection)* | When `--ai` is active, skill snippets from `ai/skills/<name>/all.md` and `ai/skills/<name>/<platform>.md` are injected into platform config files (e.g. `~/.claude/CLAUDE.md`) between `<!-- dfiles managed start -->` / `<!-- dfiles managed end -->` markers. If the config file has no markers and the session is interactive, you are prompted to add them. |
| `--remove-unreferenced-brews` | After installing, uninstall any leaf formula/cask not referenced by any active Brewfile. |
| `--interactive` | Like `--remove-unreferenced-brews` but prompts for confirmation before removing. Implies `--remove-unreferenced-brews`. |

**Section filter behavior:** If none of `--files/--brews/--ai` are given, all sections
are applied. If any are given, only those sections run.

---

## `dfiles diff`

Show the diff between tracked source files/packages and live state.

Exits 0 when everything is up to date, 1 when drift is found.

By default all sections are diffed. Use `--files`, `--brews`, and/or `--ai` to
inspect only specific sections.

```
dfiles diff [--profile <p>] [--module <m>]
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

## `dfiles status`

Show drift between tracked source files and live destinations. More concise than `diff`.

Drift markers: `✓` clean  `M` modified  `?` missing  `!` source missing

By default all sections are shown. Use `--files`, `--brews`, and/or `--ai` to
inspect only specific sections.

```
dfiles status [--profile <p>] [--files] [--brews] [--ai]
```

| Option | Description |
|--------|-------------|
| `--profile <p>` | Profile to check. Default: last-used profile from state, or `default`. |
| `--files` | Show dotfile drift only. |
| `--brews` | Show Homebrew package drift only. |
| `--ai` | Show AI skill drift (from `ai/skills.toml`) only. |

---

## `dfiles brew`

Run `brew install`/`uninstall` and keep Brewfiles in sync.

Use these instead of bare `brew install` when you want changes to persist across
machines — the formula is automatically added to or removed from the Brewfile(s).

### `dfiles brew install`

```
dfiles brew install <name> [--cask] [--module <m>]
```

| Argument/Option | Description |
|-----------------|-------------|
| `name` | Formula or cask name (e.g. `ripgrep`, `iterm2`). |
| `--cask` | Install as a cask (GUI apps, fonts, etc.). |
| `--module <m>` | Record in this module's `brew/Brewfile.<m>`. Default: master `brew/Brewfile`. |

### `dfiles brew uninstall`

```
dfiles brew uninstall <name> [--cask]
```

| Argument/Option | Description |
|-----------------|-------------|
| `name` | Formula or cask name. |
| `--cask` | Uninstall as a cask. |

Removes the formula from **all** Brewfiles in the repo, then runs `brew uninstall`.

---

## `dfiles import`

Import dotfiles from another dotfile manager (one-time migration).

Currently only `chezmoi` is supported as a source format.

```
dfiles import --from chezmoi [--source <dir>] [--dry-run] [--include-ignored-files]
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

## `dfiles ai`

Manage AI agent skills across platforms. Skills are declared as directories under
`ai/skills/<name>/` (one `skill.toml` per skill) and deployed to platform skill
directories (e.g. `~/.claude/skills/`) on `dfiles apply`.

### `dfiles ai discover`

Scan this machine for installed AI agent platforms and offer to update `ai/platforms.toml`.

```
dfiles ai discover
```

### `dfiles ai add`

Add a skill declaration to `ai/skills/<name>/skill.toml` and create a blank
`all.md` snippet stub. Does **not** deploy the skill; run `dfiles apply --ai`
afterward to deploy.

```
dfiles ai add <source> [--name <n>] [--platforms <p>] [--deploy symlink|copy]
```

| Argument/Option | Description |
|-----------------|-------------|
| `source` | Skill source: `gh:owner/repo[/subpath][@ref]` or `dir:~/path`. |
| `--name <n>` | Local name for the skill. Default: inferred from source. |
| `--platforms <p>` | Target platforms: `all`, `cross-client`, or comma-separated IDs. Default: `all`. |
| `--deploy <method>` | `symlink` (default) or `copy`. |

### `dfiles ai fetch`

Download skills into the local cache (`~/.dfiles/skills/`) without deploying them.
Respects the lock file — already-cached skills at the pinned SHA are skipped.

```
dfiles ai fetch [<name>]
```

| Argument | Description |
|----------|-------------|
| `name` | Optional. Skill name to fetch. Omit to fetch all skills. |

### `dfiles ai update`

Fetch the latest version of skills from their sources, updating the lock file SHAs.
Does **not** deploy; run `dfiles apply --ai` afterward to deploy updated skills.

```
dfiles ai update [<name>]
```

| Argument | Description |
|----------|-------------|
| `name` | Optional. Skill name to update. Omit to update all skills. |

**Difference from `fetch`:** `fetch` respects the current lock SHA (no-op if already cached).
`update` clears the lock SHA and pulls the latest version, then records the new SHA.

### `dfiles ai remove`

Remove a skill directory (`ai/skills/<name>/`) and optionally remove it from
platform skill directories.

```
dfiles ai remove <name> [--yes]
```

| Argument/Option | Description |
|-----------------|-------------|
| `name` | Skill name (directory name under `ai/skills/`). |
| `--yes` | Skip confirmation prompts. |

### `dfiles ai search`

Search the [skills.sh](https://skills.sh) registry for available skills.

```
dfiles ai search <query> [--limit <n>]
```

| Argument/Option | Description |
|-----------------|-------------|
| `query` | Search term (e.g. `pdf`, `browser`, `git`). |
| `--limit <n>` | Maximum number of results to show. Default: 10. |

Results show the skill source in `gh:owner/repo/skill` format and install count.
Copy the source and pass it to `dfiles ai add` to start tracking it.

### `dfiles ai scan`

Interactively scan an existing skills directory and offer to add any unmanaged
skills to `ai/skills.toml`.

```
dfiles ai scan <path> [--dry-run]
```

| Argument/Option | Description |
|-----------------|-------------|
| `path` | Path to a skills directory to scan (e.g. `~/.claude/skills`). |
| `--dry-run` | Show what would be added without modifying `ai/skills.toml`. |

For each unmanaged skill found, dfiles tries to identify its GitHub source via:
1. Git remote detection (for skills that are git clones)
2. skills.sh registry search (fuzzy match on the skill directory name)

If a source is found it is shown as a suggestion; the user is prompted to
confirm (`y`), edit the source (`e`), or skip (`n`) each skill. Skills that
are already tracked in `ai/skills.toml` are silently skipped.

---

## `dfiles telemetry`

Manage local telemetry: enable, disable, or annotate the telemetry log.

```
dfiles telemetry [--enable] [--disable] [--note "<message>"]
```

Without flags, prints the current telemetry status.

| Flag | Description |
|------|-------------|
| `--enable` | Set `[telemetry] enabled = true` in `dfiles.toml`. |
| `--disable` | Set `[telemetry] enabled = false` in `dfiles.toml`. |
| `--note "<text>"` | Append a free-form note to `~/.dfiles/telemetry.jsonl`. Always writes regardless of whether telemetry is enabled. |

```sh
# Turn telemetry on or off
dfiles telemetry --enable
dfiles telemetry --disable

# Check current status
dfiles telemetry

# Annotate the log with context
dfiles telemetry --note "starting fresh config — prior runs were testing"
dfiles telemetry --note "hit an error with extfile_ on m4 mac, will investigate"
dfiles telemetry --note "onboarding new work macbook"
```

Notes appear in the JSONL file alongside command events and are easy to filter:

```sh
grep '"kind":"note"' ~/.dfiles/telemetry.jsonl | jq .
```

---

## `dfiles upgrade`

Upgrade dfiles to the latest version by downloading the release from GitHub,
verifying its SHA256 checksum, and atomically replacing the running binary.

```
dfiles upgrade [--check] [--force]
```

| Flag | Description |
|------|-------------|
| `--check` | Check whether an update is available without installing it. Exits 0 when up to date, exits 1 when an update is available. |
| `--force` | Install the latest version even if the current version is already the latest. |

```sh
# Check for an update (CI-friendly — exits 1 if update is available)
dfiles upgrade --check

# Upgrade to the latest version
dfiles upgrade

# Reinstall the current version (useful after path changes)
dfiles upgrade --force
```

**How it works:**

1. Queries the GitHub releases API for the latest tag.
2. Downloads the platform-specific tarball and `SHA256SUMS` file.
3. Verifies the tarball checksum before extracting.
4. Atomically replaces the running binary (write to `dfiles.new`, then rename).

Supported platforms: macOS (arm64, x86_64), Linux (x86_64, aarch64, armv7, i686).

---

## `dfiles security-scan`

Scan all tracked source files for secrets, sensitive filenames, and credential paths.

Checks each tracked file for:
- **Filename patterns** — `.env`, `id_rsa`, `.pem`, `credentials`, `secrets`, `.key`, etc.
- **Path patterns** — `~/.aws/credentials`, `~/.kube/**`, `~/.ssh/**`, `~/.config/gh/hosts.yml`, `~/.docker/config.json`, etc.
- **Content patterns** — GitHub tokens (`ghp_`/`ghs_`/`github_pat_`), AWS access keys (`AKIA…`), PEM private keys, OpenAI keys (`sk-…`), Anthropic keys (`sk-ant-…`), generic password/secret assignments
- **High-entropy strings** — opt-in via `--entropy` (disabled by default to reduce false positives)

Exits 0 when clean, 1 when findings are reported.

```
dfiles security-scan [--entropy]
```

| Option | Description |
|--------|-------------|
| `--entropy` | Also flag high-entropy strings (≥16 chars, Shannon entropy >4.5 bits/char). Opt-in: may produce false positives on base64 data. |

**Suppressing false positives** — add paths to `[security] allow` in `dfiles.toml`:

```toml
[security]
allow = [
  "~/.config/gh/hosts.yml",     # intentionally tracked
  "~/.config/gcloud/**",        # managed by gcloud CLI, not a secret
]
```

Patterns follow the same glob syntax as `config/ignore` (`*` matches within a path segment, `**` crosses separators).

**Integration with `dfiles add`** — when a file is added with `dfiles add`, its content is automatically scanned. If sensitive patterns are found, you are prompted before the file is saved to `source/`. Declining removes it immediately with no partial state left behind.

---

## `dfiles completions`

Print a shell completion script to stdout.

```
dfiles completions <shell>
```

| Argument | Description |
|----------|-------------|
| `shell` | `fish`, `zsh`, or `bash` |

**Setup:**

```sh
# Fish — write once to the completions directory:
dfiles completions fish > ~/.config/fish/completions/dfiles.fish

# Zsh — add to ~/.zshrc:
source <(dfiles completions zsh)

# Bash — add to ~/.bashrc:
source <(dfiles completions bash)
```

---

## Concepts

### Profiles

A profile selects which modules are active. Declared in `dfiles.toml` under `[profile.<name>]`.
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

Every `gh:` skill source is pinned by SHA in `dfiles.lock`. On cache miss, the
freshly-fetched SHA is compared against the recorded lock entry. A mismatch is a
hard error — run `dfiles ai update <name>` to explicitly accept the changed content.

### Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `DFILES_DIR` | `~/.local/share/dfiles` (XDG; `~/dfiles` if it exists) | Repo root directory |
| `DFILES_CLAUDE_DIR` | `~/.claude` | Claude Code directory (skills, CLAUDE.md) |
| `DFILES_TELEMETRY` | unset | `1` to enable local telemetry, `0` to force-disable |
