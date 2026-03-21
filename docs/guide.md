# dfiles User Guide

dfiles is an AI-first dotfiles and environment manager. It tracks your dotfiles,
Homebrew packages, language runtimes, and Claude Code skills in a single git
repository, and can reproduce your full development environment on any machine
from a single command.

---

## Concepts

**Repo** — a git repository (default: `~/dfiles`) that holds your config and source
files. You commit and push it like any other repo.

**Source file** — a dotfile stored under `source/` inside the repo, with its
destination path and flags encoded directly in the filename. dfiles copies (or
renders) it to its decoded destination on apply.

**Module** — a named group of packages and external repos defined in
`config/modules/<name>.toml`. Modules control Homebrew, mise, AI tools, and
externals. Files are **not** listed in modules — their encoded filenames in
`source/` are the sole source of truth.

**Profile** — a named set of modules defined in `dfiles.toml`. Different machines
or contexts (work, personal, minimal) activate different subsets of modules.

**State** — `~/.dfiles/state.json` records what was last applied. Used by
`dfiles status` to detect drift.

---

## Repo layout

```
~/dfiles/
├── dfiles.toml                 # profiles and which modules each profile activates
├── dfiles.lock                 # pinned SHA-256 for every fetched GitHub source
├── dfiles-manifest.json        # (optional) package manifest for bootstrap
│
├── source/                     # dotfiles with magic-name encoded filenames
│   ├── dot_zshrc               # → ~/.zshrc
│   ├── dot_gitconfig.tmpl      # → ~/.gitconfig  (rendered before writing)
│   ├── private_dot_ssh/
│   │   └── id_rsa              # → ~/.ssh/id_rsa  (chmod 0600)
│   └── dot_config/
│       ├── git/
│       │   └── config          # → ~/.config/git/config
│       └── extdir_nvim         # → git clone ... ~/.config/nvim
│
├── brew/                       # Homebrew Brewfiles
│   ├── Brewfile                # master (used when no --module)
│   └── Brewfile.<module>       # per-module Brewfile
│
└── config/
    └── modules/
        ├── shell.toml
        ├── git.toml
        └── packages.toml
```

Everything under `source/`, `brew/`, and `config/` is committed to git.
`~/.dfiles/` (state, backups) is not committed — `dfiles init` adds it to `.gitignore`.

---

## Getting started

### Initialize a repo

```
dfiles init
```

Creates the directory structure and a starter `dfiles.toml` with default profiles.
If the directory is not already a git repo, dfiles prints a reminder to run
`git init` or `jj init`.

By default the repo lives at `~/dfiles`. Override with:

```
dfiles --dir /path/to/repo init
# or permanently:
export DFILES_DIR=/path/to/repo
```

### Track a file or directory

```
dfiles add ~/.zshrc
dfiles add ~/.gitconfig
dfiles add ~/.tmux/plugins/tpm   # git repo → prompts for extdir or files
dfiles add ~/.config/nvim         # same
```

**Files:** Copied into `source/` with a magic-name encoded filename. Permissions
are auto-detected — `private_` for mode 0600, `executable_` for any execute bit.
Intermediate directory permissions are also encoded: `~/.ssh` at 0700 produces
`source/private_dot_ssh/`.

**Directories:** If the directory is a git repo with remotes, dfiles asks whether
to add it as an `extdir_` external (cloned on apply) or copy all files recursively.
Hidden subdirectories (`.git`, etc.) are skipped when adding recursively.

No TOML entry is needed — the encoded filename (or extdir_ marker) is the
complete record.

### Apply your config

```
dfiles apply
dfiles apply --profile work
dfiles apply --module shell      # apply one module only
dfiles apply --dry-run           # print the plan without writing anything
dfiles apply --dest ~/staging    # apply to a staging dir (chroot-style, for testing)
```

For each tracked file, dfiles backs up the existing destination file to
`~/.dfiles/backups/` before overwriting it.

### Check for drift

```
dfiles status
dfiles status --profile work
dfiles diff                   # full unified diff (what would change)
dfiles diff --stat            # summary: filenames + line counts only
```

`dfiles status` gives you a quick overview — which files are in sync, which have
drifted, which are missing. `dfiles diff` shows you the exact lines that differ.

| Marker | Meaning |
|--------|---------|
| `✓`    | File is in sync |
| `M`    | Destination exists but differs from source |
| `?`    | Destination does not exist (never applied) |

---

## Magic-name encoding

All file metadata is encoded in the source filename. No separate TOML file registry.

| Filename component | Decoded meaning |
|--------------------|-----------------|
| `dot_` prefix on any component | Replace with `.` |
| `private_` prefix on file | chmod 0600 (owner read/write only) |
| `private_` prefix on directory | chmod 0700 (owner rwx only) |
| `executable_` prefix | chmod 0755 (or 0700 if combined with `private_`) |
| `symlink_` prefix | Create a symlink pointing into `source/` |
| `extdir_` prefix on file | Marker file — clone a remote git repo into this directory on apply |
| `.tmpl` suffix on file | Render through the Tera template engine |

Prefixes can be stacked in any order: `private_executable_` and
`executable_private_` are equivalent.

### Examples

| `source/` path | Destination | Permissions |
|----------------|-------------|-------------|
| `dot_zshrc` | `~/.zshrc` | unchanged |
| `dot_config/git/config` | `~/.config/git/config` | unchanged |
| `private_dot_ssh/id_rsa` | `~/.ssh/id_rsa` | 0600 |
| `private_dot_ssh/` (dir) | `~/.ssh/` | 0700 |
| `executable_dot_local/bin/foo` | `~/.local/bin/foo` | 0755 |
| `private_executable_dot_local/bin/s` | `~/.local/bin/s` | 0700 |
| `symlink_vscode_settings.json` | `~/vscode_settings.json` | symlink |
| `dot_gitconfig.tmpl` | `~/.gitconfig` | (rendered) |

---

## Modules

A module is a TOML file at `config/modules/<name>.toml`. Every section is optional.
Sections: `[homebrew]`, `[mise]`, `[ai]`, `requires_op`.

Modules do **not** list source files — including external git repos. All tracked
files and externals live under `source/` using magic-name encoding.

### External git repos (extdir_)

External directories (git repos cloned into your home directory, like plugin
managers or separate config repos) are tracked as `extdir_` marker files inside
`source/`. The marker file's location encodes the destination; its content is a
small TOML file with the URL and optional ref.

```
source/dot_config/extdir_nvim
```

Content of the marker file:

```toml
type = "git"
url  = "https://github.com/user/nvim-config"
ref  = "main"   # optional — branch, tag, or commit SHA
```

This marker file tells dfiles: clone `https://github.com/user/nvim-config` into
`~/.config/nvim` when applying.

**Adding externals:**

The easiest way is `dfiles add <directory>`. If the directory is a git repo,
dfiles detects the remotes and asks whether to add it as an external or copy
all its files recursively:

```
~/.config/nvim is a git repository with 1 remote:

  1) origin   https://github.com/user/nvim-config

How to add?
  1) Add as external (cloned on apply)
  f) Add all files recursively
  q) Skip
[1/f/q]:
```

You can also write the marker file by hand under `source/` — the path follows
the same `dot_` encoding as all other source entries.

**Examples:**

| `source/` marker path | Live destination | URL |
|------------------------|-----------------|-----|
| `dot_tmux/plugins/extdir_tpm` | `~/.tmux/plugins/tpm` | (from marker content) |
| `dot_config/extdir_nvim` | `~/.config/nvim` | (from marker content) |
| `dot_oh-my-zsh/custom/extdir_plugins` | `~/.oh-my-zsh/custom/plugins` | (from marker content) |

**Apply behavior:**

- **Dest absent** — runs `git clone [--branch ref] url dest`
- **Dest is a git repo** — skipped by default (use `--apply-externals` to pull)
- **Dest is not a git repo** — hard error (remove manually first)

`dfiles status` reports `?` if the dest is absent and `M` if it exists but is
not a git repo.

**`--apply-externals` flag:**

By default, `dfiles apply` skips externals that are already present as git repos
(leaving them at their current state). Pass `--apply-externals` to pull updates:

```
dfiles apply --apply-externals
```

### Homebrew

```toml
[homebrew]
brewfile = "brew/Brewfile.shell"
```

The `brewfile` field is a single path relative to the repo root pointing to a
Homebrew Brewfile. On apply, dfiles runs `brew bundle install --file=<path>`.
If Homebrew is not installed, this section is skipped with a warning.

Use `dfiles brew install` and `dfiles brew uninstall` instead of bare `brew`
commands to keep your Brewfiles automatically in sync:

```
dfiles brew install ripgrep                   # → brew/Brewfile (master)
dfiles brew install ripgrep --module shell    # → brew/Brewfile.shell
dfiles brew install iterm2 --cask             # cask entry
dfiles brew uninstall ripgrep                 # removes from ALL Brewfiles
```

**Brewfile layout:**

| Path | Used when |
|------|-----------|
| `brew/Brewfile` | `dfiles brew install` with no `--module` |
| `brew/Brewfile.<name>` | `dfiles brew install --module <name>` |

### Mise

```toml
[mise]
config = "source/mise.toml"   # path relative to repo root; omit to use global config
```

On apply, dfiles runs `mise install`. If mise is not installed, the section is
skipped with a hint to install it.

### AI tools (Claude Code)

```toml
[ai]
skills   = ["gh:owner/skills-repo@v1.2"]
commands = ["gh:owner/commands-repo@main"]
```

Skills are installed to `~/.claude/skills/<repo>/`.
Commands are installed to `~/.claude/commands/<repo>/`.

After every successful apply, dfiles regenerates `~/.claude/CLAUDE.md` so Claude
Code can see which skills and commands are available.

Source references use the format `gh:owner/repo` or `gh:owner/repo@ref` where
`ref` is a tag, branch, or commit SHA. Fetched sources are pinned in `dfiles.lock`.

### 1Password guard

```toml
requires_op = true
```

Adding `requires_op = true` to any module causes dfiles to skip that module's
externals, brew, mise, and AI steps with a warning if the `op` CLI is not
installed or the user is not signed in. Source files in `source/` are always
applied regardless of this flag.

---

## Profiles

`dfiles.toml` defines which modules are active for each profile.

```toml
[profile.default]
modules = ["shell", "git", "packages"]

[profile.work]
extends = "default"       # inherits all modules from default
modules = ["secrets"]     # then adds this one

[profile.minimal]
modules = ["shell"]
```

Profiles support single-level inheritance via `extends`. The parent's modules are
applied first, then the child's modules are appended (duplicates removed).

Apply a specific profile:

```
dfiles apply --profile work
dfiles status --profile work
```

The default profile name is `default`.

---

## Templates

Source files with a `.tmpl` suffix are rendered through the
[Tera](https://keats.github.io/tera/) template engine (Jinja2-compatible syntax)
before being written to their destination. The `.tmpl` suffix is stripped from
the destination filename.

### Available variables

| Variable | Value |
|----------|-------|
| `{{ os }}` | `"macos"`, `"linux"`, or the OS name |
| `{{ hostname }}` | Machine hostname |
| `{{ username }}` | Current user (`$USER`) |
| `{{ profile }}` | Active profile name |
| `{{ get_env(name="VAR") }}` | Value of environment variable `VAR` |
| `{{ get_env(name="VAR", default="fallback") }}` | With fallback if unset |

### Example: OS-conditional config

```
# source/dot_gitconfig.tmpl — stored in the repo
[core]
{% if os == "macos" %}
  editor = /opt/homebrew/bin/nvim
{% else %}
  editor = /usr/bin/nvim
{% endif %}
```

### Example: Profile-conditional config

```
# source/dot_zshrc.tmpl
export PATH="$HOME/.local/bin:$PATH"
{% if profile == "work" %}
source ~/.work-aliases
{% endif %}
```

### Example: Environment variable injection

```
# source/dot_config/tool/config.tmpl
api_base = {{ get_env(name="API_BASE", default="https://api.example.com") }}
```

### Tera features

Full Jinja2-style control flow is available: `{% if %}`, `{% for %}`, filters,
macros, and `{# comments #}`. See the
[Tera documentation](https://keats.github.io/tera/docs/) for the complete
reference.

Files without the `.tmpl` suffix are **never** rendered — curly braces
in shell scripts, Makefiles, and similar files are left untouched.

---

## 1Password integration

dfiles can read secrets from 1Password at apply time and render them directly into
destination files, without ever storing them in the repo or on disk.

### Prerequisites

1. Install the `op` CLI: <https://developer.1password.com/docs/cli/get-started/>
2. Sign in: `op signin`

### Usage in templates

```
# source/dot_config/gh/hosts.yml.tmpl
github.com:
  user: alice
  oauth_token: {{ op(path="Personal/GitHub/token") }}
  # or with the full op:// URI:
  oauth_token: {{ op(path="op://Personal/GitHub/oauth_token") }}
```

The `op://vault/item/field` URI is the 1Password secret reference format. If you
omit the `op://` prefix, dfiles adds it automatically.

### Module guard

Mark any module that contains `op()` calls with `requires_op = true`:

```toml
# config/modules/secrets.toml
requires_op = true
```

Any `extdir_` markers under `source/` that are associated with this profile
will also respect `requires_op` — they are skipped if `op` is not available.

If `op` is not installed or the user is not signed in, the module is skipped with
a warning instead of failing with an error. All other modules are applied normally.

---

## Bootstrap

`dfiles bootstrap` sets up a machine from scratch. It combines apply + status into
a single command, and can optionally fetch a remote environment package first.

### Bootstrap from the local repo

```
dfiles bootstrap
dfiles bootstrap --profile work
dfiles bootstrap --dry-run
```

Equivalent to `dfiles apply` followed by `dfiles status`. If a `dfiles-manifest.json`
is present in the repo root, a banner is printed showing the package name and version.

### Bootstrap from a remote package

```
dfiles bootstrap gh:alice/my-env
dfiles bootstrap gh:alice/my-env@v1.2
dfiles bootstrap gh:alice/my-env@v1.2 --profile work --dry-run
```

Downloads the GitHub repository archive, extracts it to `~/.dfiles/envs/`, then
applies and reports status. The `@ref` suffix pins a specific tag, branch, or
commit SHA.

In dry-run mode the fetch is skipped entirely — no network requests are made.

### The package manifest

If the repo contains a `dfiles-manifest.json`, it is displayed as a banner during
bootstrap:

```json
{
  "name": "my-ai-env",
  "version": "v1.2",
  "author": "alice",
  "profiles": ["default", "work"],
  "modules": ["shell", "git", "packages", "ai"],
  "created": "2026-03-20"
}
```

All fields except `name` and `version` are optional.

### The lock file

`dfiles.lock` pins the SHA-256 of every fetched GitHub source for reproducible
installs. Commit it alongside your config:

```toml
# dfiles.lock (auto-generated — do not edit by hand)
[sources."gh:alice/my-env@v1.2"]
sha256     = "3a9f2..."
fetched_at = "2026-03-20T12:00:00Z"
```

---

## Importing from chezmoi

If you already manage dotfiles with chezmoi, `dfiles import` converts your existing
setup into dfiles format in one step.

```
dfiles import --from chezmoi
dfiles import --from chezmoi --source ~/my-chezmoi-dir
dfiles import --from chezmoi --dry-run
```

### What gets imported

Files are copied into `source/` preserving their chezmoi magic-name encoding, which
dfiles uses natively. No translation needed for filenames — `dot_`, `private_`,
`executable_`, and `symlink_` prefixes are understood directly.

| chezmoi source path | dfiles source path | destination |
|--------------------|--------------------|-------------|
| `dot_zshrc` | `source/dot_zshrc` | `~/.zshrc` |
| `dot_config/git/config` | `source/dot_config/git/config` | `~/.config/git/config` |
| `private_dot_ssh/id_rsa` | `source/private_dot_ssh/id_rsa` | `~/.ssh/id_rsa` (0600) |
| `executable_dot_local/bin/foo` | `source/executable_dot_local/bin/foo` | `~/.local/bin/foo` (0755) |
| `private_executable_dot_local/bin/s` | `source/private_executable_dot_local/bin/s` | (0700) |

`.chezmoiexternal.toml` is also parsed. Each entry with `type = "git-repo"` is
converted to an `extdir_` marker file in `source/`:

| `.chezmoiexternal.toml` entry | dfiles result |
|-------------------------------|---------------|
| `["~/.config/nvim"]` / `type = "git-repo"` | `source/dot_config/extdir_nvim` |
| `["~/.config/tmux/plugins/tpm"]` / `type = "git-repo"` | `source/dot_tmux/plugins/extdir_tpm` |

The marker file content mirrors the original TOML (`type`, `url`, `ref`).

### What gets converted

`.tmpl` files are converted from Go template syntax to Tera syntax during import:

| chezmoi construct | dfiles Tera equivalent |
|------------------|----------------------|
| `{{ .chezmoi.hostname }}` | `{{ hostname }}` |
| `{{ .chezmoi.username }}` | `{{ username }}` |
| `{{ .chezmoi.os }}` | `{{ os }}` |
| `{{ .chezmoi.homeDir }}` | `{{ get_env(name="HOME") }}` |
| `{{ .someVar }}` | `{{ someVar }}` |
| `{{- if eq .chezmoi.os "darwin" -}}` | `{% if os == "macos" %}` |
| `{{- if eq .chezmoi.os "linux" -}}` | `{% if os == "linux" %}` |
| `{{- if (index . "key") }}` | `{% if key %}` |
| `{{- else if ... }}` | `{% elif ... %}` |
| `{{- else }}` | `{% else %}` |
| `{{- end }}` | `{% endif %}` |
| `{{/* comment */}}` | `{# comment #}` |

Constructs with no direct mapping (range loops, custom functions, pipeline operators)
are preserved as-is and a warning is printed. The imported file is still usable;
flagged constructs will need manual cleanup before `dfiles apply`.

### What gets skipped (v1)

| chezmoi prefix/suffix | Reason |
|----------------------|--------|
| `symlink_` entries whose target cannot be resolved | Cannot copy a dangling symlink |
| `exact_` / `create_` / `modify_` | Unsupported chezmoi attributes |
| `run_once_` / `run_` / `once_` | Install/run scripts |
| `.chezmoi*` / `chezmoistate.*` | chezmoi-internal files |

Skipped items are always listed with a reason. P1 follow-ons for run scripts
are tracked in `TODOS.md`.

### Source directory detection

1. `--source <path>` — use as-is
2. `chezmoi source-path` — subprocess (if `chezmoi` is on `PATH`)
3. `~/.local/share/chezmoi` — XDG default fallback
4. Hard error with a hint if none of the above is found

### After importing

```
dfiles apply --dry-run        # verify the import looks correct
dfiles apply                  # deploy to this machine
```

The importer prints a hint if any new module is not yet listed in a profile in
`dfiles.toml`. Re-running import is safe — it is idempotent and never overwrites
existing source files.

---

## Command reference

### `dfiles init`

Initialize a new dfiles repo.

```
dfiles init [--dir <path>]
```

Creates `dfiles.toml`, `source/`, `brew/`, `config/modules/shell.toml`, and `.gitignore`.
Fails if already initialized.

### `dfiles add <path>`

Track a dotfile or directory.

```
dfiles add ~/.zshrc
dfiles add ~/.ssh/id_rsa
dfiles add ~/.tmux/plugins/tpm
dfiles add ~/.config/nvim
```

**Tracking a file:** Copies it into `source/` with a magic-name encoded filename.
Flags are auto-detected from the file's permissions:

- No group/other read bits (mode `0600`) → `private_` prefix
- Any execute bit → `executable_` prefix
- Directory permissions are also checked: if `~/.ssh` is `0700`, adding
  `~/.ssh/config` produces `source/private_dot_ssh/config`

Warns before tracking files matching sensitive patterns (SSH keys, `.env`, etc.).

**Tracking a directory:** If the directory is a git repo with one or more remotes,
dfiles prompts you to choose:

- **Add as external** — writes an `extdir_` marker file in `source/` (the
  directory will be cloned on apply). Asks which remote to use if there are several.
- **Add files recursively** — copies every non-hidden file in the directory tree
  into `source/` with encoded filenames. Hidden directories (like `.git`) are skipped.

If the directory has no git remotes, files are added recursively without prompting.

No TOML entry is written — the encoded filename (or `extdir_` marker) is the
complete record.

### `dfiles apply [--profile <name>] [--module <name>] [--dry-run] [--dest <path>]`

Apply tracked files and packages to this machine.

```
dfiles apply
dfiles apply --profile work
dfiles apply --module shell
dfiles apply --dry-run
dfiles apply --dest ~/staging    # write to staging dir instead of real home
```

- Backs up existing destination files to `~/.dfiles/backups/` before overwriting.
- Installs Homebrew packages, runs mise, and fetches AI skills/commands.
- Regenerates `~/.claude/CLAUDE.md` after every successful apply.
- Writes state to `~/.dfiles/state.json`.

`--dest <path>` rebases all destinations under `<path>` — useful for testing
apply output without touching your real home directory.

### `dfiles status [--profile <name>]`

Show drift between tracked source files and live destinations.

```
dfiles status
dfiles status --profile work
```

### `dfiles diff [--profile <name>] [--stat] [--color <auto|always|never>]`

Show a unified diff between each tracked source file and its live destination.

```
dfiles diff
dfiles diff --profile work
dfiles diff --stat            # summary only (filenames + changed line counts)
dfiles diff --color never     # plain text output for piping
```

Output markers:

| Marker | Meaning |
|--------|---------|
| `--- source/...` / `+++ ~/...` | Unified diff header |
| `? ~/.tmux/plugins/tpm  (extdir: not cloned)` | External not yet cloned |

`--stat` shows a condensed summary: `  M ~/.zshrc  (+3 -1)`. Templates are
rendered before diffing so you see the final content, not raw Tera syntax.

### `dfiles bootstrap [<source>] [--profile <name>] [--dry-run]`

Bootstrap this machine from a local or remote environment package.

```
dfiles bootstrap
dfiles bootstrap gh:owner/repo@ref
dfiles bootstrap gh:owner/repo@ref --profile work --dry-run
```

### `dfiles brew install <name> [--cask] [--module <name>]`

Install a Homebrew formula and record it in your dfiles repo.

```
dfiles brew install ripgrep
dfiles brew install iterm2 --cask
dfiles brew install ripgrep --module packages
```

Runs `brew install [--cask] <name>` and adds the entry to the appropriate Brewfile:

- No `--module` → `brew/Brewfile` (master)
- `--module <name>` → `brew/Brewfile.<name>` (created and registered in the module config if needed)

### `dfiles brew uninstall <name> [--cask]`

Uninstall a Homebrew formula and remove it from all Brewfiles in your repo.

```
dfiles brew uninstall ripgrep
dfiles brew uninstall iterm2 --cask
```

Scans every Brewfile under `brew/`, removes the matching entry, then runs
`brew uninstall [--cask] <name>`.

### `dfiles import --from <format> [--source <path>] [--dry-run]`

Import dotfiles from another dotfile manager.

```
dfiles import --from chezmoi
dfiles import --from chezmoi --source ~/path/to/chezmoi
dfiles import --from chezmoi --dry-run
```

Currently only `chezmoi` is supported as a source format.

---

## Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `DFILES_DIR` | `~/dfiles` | Repo root directory |
| `DFILES_CLAUDE_DIR` | `~/.claude` | Claude Code directory (skills, commands, CLAUDE.md) |
| `DFILES_ENVS_DIR` | `~/.dfiles/envs` | Where remote bootstrap packages are extracted |
