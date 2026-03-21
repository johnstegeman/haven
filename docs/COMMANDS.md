# dfiles Command Reference

## Quick Reference

```
dfiles bootstrap [source] [--profile <p>] [--dry-run]
dfiles init [source] [--branch <b>] [--apply] [--profile <p>]
dfiles add <file> [--link]
dfiles apply [--profile <p>] [--module <m>] [--dry-run]
            [--files] [--brews] [--ai]
            [--apply-externals]
            [--remove-unreferenced-brews] [--interactive]
dfiles diff  [--profile <p>] [--module <m>]
            [--files] [--brews] [--ai]
            [--stat] [--color always|never|auto]
dfiles status [--profile <p>] [--files] [--brews] [--ai]
dfiles brew install <name> [--cask] [--module <m>]
dfiles brew uninstall <name> [--cask]
dfiles import --from chezmoi [--source <dir>] [--dry-run]
             [--include-ignored-files]
dfiles ai discover
dfiles ai add <source> [--name <n>] [--platforms <p>] [--deploy symlink|copy]
dfiles ai fetch [<name>]
dfiles ai update [<name>]
dfiles ai remove <name> [--yes]
```

---

## Global Options

| Option | Default | Description |
|--------|---------|-------------|
| `--dir <path>` | `~/dfiles` | dfiles repo directory. Also read from `DFILES_DIR` env var. |

---

## `dfiles bootstrap`

Apply and verify your environment on this machine. The workhorse for day-to-day provisioning.

Without a source, equivalent to `dfiles apply` + `dfiles status` on the local repo.
With a `gh:` source, fetches the remote package first, then applies it.

Use `dfiles init` instead when setting up a machine for the first time — `init` creates
the local repo structure that `bootstrap` expects to already exist.

```
dfiles bootstrap [source] [--profile <p>] [--dry-run]
```

| Argument/Option | Description |
|-----------------|-------------|
| `source` | Optional. `gh:owner/repo` or `gh:owner/repo@ref`. Omit to use local repo. |
| `--profile <p>` | Profile to apply. Default: `default`. |
| `--dry-run` | Print what would be applied without writing any files or fetching packages. |

---

## `dfiles init`

Create or clone a dfiles repository. **Use this once, on first-time setup.**
For subsequent re-provisioning of an already-initialised machine, use `dfiles bootstrap`.

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

## `dfiles add`

Start tracking a dotfile by copying it into `source/`.

For directories: if the directory is a git repo, interactively prompts to track it as
an external clone (re-cloned on apply) or recursively copy its files.

```
dfiles add <file> [--link]
```

| Argument/Option | Description |
|-----------------|-------------|
| `file` | Path to the file or directory to track (e.g. `~/.zshrc`). |
| `--link` | Track as a symlink: on apply, the destination will be symlinked back to `source/` rather than copied. Use for files that apps manage themselves (e.g. VS Code settings). |

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
| `--module <m>` | Scope brew and AI operations to this module. **Dotfiles in source/ are always applied globally — this flag does not filter file operations.** |
| `--dry-run` | Print the plan without writing any files. |
| `--files` | Apply dotfile copies/symlinks. |
| `--brews` | Run `brew bundle install`. |
| `--ai` | Deploy AI skills from `ai/skills.toml` (and legacy `[ai]` module sections). |
| `--apply-externals` | Pull (update) existing `extdir_` git clones in addition to cloning missing ones. Without this, existing clones are left as-is. |
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
| `--module <m>` | Scope brew and AI diff to this module. **Dotfiles in source/ are always diffed globally — this flag does not filter file operations.** |
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
| `--ai` | Show AI skill drift only. |

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
| `--module <m>` | Record in this module's `brew/Brewfile.<m>`. Auto-detected when there is exactly one Brewfile; required when there are several. Default: master `brew/Brewfile`. |

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

**What is imported:** Plain files, `private_` prefix (→ `private = true`), `executable_` prefix
(→ `executable = true`), `symlink_` prefix (→ `link = true`), `*.tmpl` files (Go template
syntax converted to Tera).

**What is skipped:** `run_*` / `once_*` / `run_once_*` install scripts; `exact_*` / `create_*` /
`modify_*`; `.chezmoi*` / `chezmoistate.*`; patterns in `.chezmoiignore` (unless
`--include-ignored-files`).

---

## `dfiles ai`

Manage AI agent skills across platforms. Skills are declared in `ai/skills.toml` and
deployed to platform skill directories (e.g. `~/.claude/skills/`) on `dfiles apply`.

### `dfiles ai discover`

Scan this machine for installed AI agent platforms and offer to update `ai/platforms.toml`.

Detects platforms by checking for known binaries (`claude`, `codex`, `cursor`, etc.) and
config directories. Prints what was found and prompts before making any changes.

```
dfiles ai discover
```

### `dfiles ai add`

Interactively add a skill declaration to `ai/skills.toml`. Does **not** deploy the skill;
run `dfiles apply --ai` afterward to deploy.

```
dfiles ai add <source> [--name <n>] [--platforms <p>] [--deploy symlink|copy]
```

| Argument/Option | Description |
|-----------------|-------------|
| `source` | Skill source: `gh:owner/repo[/subpath][@ref]` or `dir:~/path`. |
| `--name <n>` | Local name for the skill. Default: inferred from source (last path component). |
| `--platforms <p>` | Target platforms: `all`, `cross-client`, or comma-separated IDs. Default: `all`. |
| `--deploy <method>` | `symlink` (default) or `copy`. |

### `dfiles ai fetch`

Download skills into the local cache (`~/.dfiles/skills/`) without deploying them.
Respects the lock file — already-cached skills at the pinned SHA are skipped.

Use this to pre-warm the cache before running `dfiles apply`, or to fetch without
touching any platform skill directories.

```
dfiles ai fetch [<name>]
```

| Argument | Description |
|----------|-------------|
| `name` | Optional. Skill name to fetch. Omit to fetch all skills in `ai/skills.toml`. |

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
`update` ignores the lock SHA and pulls the latest version, then updates the lock.

### `dfiles ai remove`

Remove a skill from `ai/skills.toml` and optionally remove it from platform skill
directories. Never removes anything automatically — always prompts.

```
dfiles ai remove <name> [--yes]
```

| Argument/Option | Description |
|-----------------|-------------|
| `name` | Skill name as declared in `ai/skills.toml`. |
| `--yes` | Skip confirmation prompts. |

---

## Concepts

### Profiles

A profile selects which modules are active. Declared in `dfiles.toml` under `[profiles.<name>]`.
The last-used profile is saved in state and reused automatically unless overridden with `--profile`.

### Sections: `--files`, `--brews`, `--ai`

Three commands — `apply`, `diff`, and `status` — operate on sections. A section
controls which parts of your environment are touched:

| Section | What it covers |
|---------|---------------|
| `--files` | Dotfiles in `source/` and external git clones (`extdir_*`) |
| `--brews` | Homebrew packages declared in Brewfiles |
| `--ai` | Skills in `ai/skills.toml` + legacy `[ai]` module sections |

### Modules

Modules group Homebrew packages, mise tool configs, and AI skills. Declared in
`config/modules/<name>.toml`. The `--module` flag on `apply` and `diff` scopes
brew/AI operations to a single module. **File operations are always global** — they
apply regardless of `--module`.

### Skill sources

| Prefix | Example | Description |
|--------|---------|-------------|
| `gh:` | `gh:anthropics/skills/pdf-processing` | GitHub repo or subdirectory. Optional `@ref` for a branch/tag. |
| `dir:` | `dir:~/projects/my-skill` | Local directory. Read directly, not cached. |

### Skill platforms

| Value | Meaning |
|-------|---------|
| `"all"` | All active platforms in `ai/platforms.toml`, **excluding** `cross-client` |
| `"cross-client"` | Only the `cross-client` platform (`~/.agents/skills/`) |
| `["claude-code", "codex"]` | Explicit list, filtered to active platforms |

### Platform registry

Platform definitions are resolved in three layers, each overriding the previous:

1. **Embedded defaults** — built-in definitions shipped with dfiles.
2. **Local registry** (`~/.dfiles/platforms.toml`, not committed) — machine-local
   additions and full-platform overrides. Use this when a shipped definition becomes
   stale (e.g. a platform changes its skills directory) or when a new platform isn't
   in the dfiles registry yet. Same `[[platform]]` array format as the table below.
   Entries with a matching `id` replace the shipped definition; new IDs are added.
3. **Repo config** (`ai/platforms.toml` in the dfiles repo, committed) — `active` list
   plus field-level overrides applied on top of the resolved registry.

#### Built-in platform definitions

| ID | Skills directory | agentskills.io | Detection binary |
|----|-----------------|---------------|------------------|
| `claude-code` | `~/.claude/skills/` | no | `claude` |
| `codex` | `~/.codex/skills/` | no | `codex` |
| `cursor` | `~/.cursor/skills/` | no | `cursor` |
| `github-copilot` | `~/.agents/skills/` | **yes** | (none) |
| `cross-client` | `~/.agents/skills/` | **yes** | (none) |

`github-copilot` and `cross-client` share the same directory (`~/.agents/skills/`);
deploy deduplicates by resolved path so the skill is only written once.

#### Local registry format (`~/.dfiles/platforms.toml`)

```toml
# Override a shipped platform (e.g. claude-code moved to ~/.agents/skills):
[[platform]]
id = "claude-code"
name = "Claude Code"
skills_dir = "~/.agents/skills"
agentskills_compliant = true

# Add a platform not yet in dfiles:
[[platform]]
id = "new-agent"
name = "New Agent"
config_dir = "~/.new-agent"
skills_dir = "~/.new-agent/skills"
binary = "new-agent"
agentskills_compliant = false
```

### Deploy methods

| Method | Behavior |
|--------|----------|
| `symlink` (default) | Creates an absolute symlink `{skills_dir}/{name}` → cache dir. Updates instantly when cache is refreshed. |
| `copy` | Copies the skill directory to `{skills_dir}/{name}`. Required for platforms that don't follow symlinks. |
