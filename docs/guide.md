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

**Module** — a named group of packages defined in `modules/<name>.toml`. Modules
control Homebrew and mise. Files are **not** listed in modules — their encoded
filenames in `source/` are the sole source of truth.

**Profile** — a named set of modules defined in `dfiles.toml`. Different machines
or contexts (work, personal, minimal) activate different subsets of modules.

**State** — `~/.dfiles/state.json` records what was last applied. Used by
`dfiles status` to detect drift.

---

## Repo layout

```
~/dfiles/
├── dfiles.toml                 # profiles and which modules each profile activates
├── dfiles.lock                 # pinned SHA for every fetched GitHub source
│
├── source/                     # dotfiles with magic-name encoded filenames
│   ├── dot_zshrc               # → ~/.zshrc
│   ├── dot_gitconfig.tmpl      # → ~/.gitconfig  (rendered before writing)
│   ├── private_dot_ssh/
│   │   └── id_rsa              # → ~/.ssh/id_rsa  (chmod 0600)
│   └── dot_config/
│       ├── git/config          # → ~/.config/git/config
│       └── extdir_nvim         # → git clone ... ~/.config/nvim
│
├── ai/                         # AI skill declarations
│   ├── skills.toml             # [[skill]] entries
│   └── platforms.toml          # which AI platforms are active on this repo
│
├── brew/                       # Homebrew Brewfiles
│   ├── Brewfile                # master (used when no --module)
│   └── Brewfile.<module>       # per-module Brewfile
│
└── modules/                    # per-module package config
    ├── shell.toml
    ├── git.toml
    └── packages.toml
```

Everything under `source/`, `brew/`, `ai/`, and `modules/` is committed to git.
`~/.dfiles/` (state, backups, skill cache) is not committed — `dfiles init` adds it to `.gitignore`.

---

## Installation

### macOS and Linux (recommended)

```sh
curl -fsSL https://raw.githubusercontent.com/johnstegeman/dfiles/main/install.sh | sh
```

The installer detects your OS and CPU architecture, downloads the matching binary
from the [latest GitHub release](https://github.com/johnstegeman/dfiles/releases),
verifies the SHA256 checksum, and installs to `/usr/local/bin` (or `~/.local/bin`
if `/usr/local/bin` is not writable).

### Pinning a version

```sh
VERSION=v0.2.0 curl -fsSL https://raw.githubusercontent.com/johnstegeman/dfiles/main/install.sh | sh
```

### Build from source

Requires Rust 1.75+:

```sh
cargo install --git https://github.com/johnstegeman/dfiles
```

---

## Getting started

### Initialize a repo

```
dfiles init
```

Creates `dfiles.toml`, `source/`, `brew/`, `modules/shell.toml`, and `.gitignore`.
Fails if already initialized.

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

### Stop tracking a file

```
dfiles remove ~/.zshrc
dfiles remove ~/.config/git/config --dry-run
```

Deletes the source file from `source/`. The live file on disk is **not** touched.
Use `--dry-run` to see what would be removed before committing.

### Apply your config

```
dfiles apply
dfiles apply --profile work
dfiles apply --module shell      # apply one module only (brew/mise)
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
| `!`    | Source file is missing from the repo |

### Ignoring files

Files stored in the repo can be excluded from `apply`, `status`, and `diff` using
`config/ignore` at the repo root. Patterns use gitignore-style glob syntax.

```
# config/ignore

# Ignore a specific file
.zshrc

# Glob patterns
.ssh/id_*
.config/*/history

# Match everything under a directory
.local/share/some-app/**

# Negate a previous match (un-ignore)
!.local/share/some-app/keep-this
```

**Pattern rules:**

| Syntax | Meaning |
|--------|---------|
| `#` at start of line | Comment — ignored |
| `*` | Any sequence of non-`/` characters |
| `**` | Any sequence of characters including `/` |
| `?` | Any single non-`/` character |
| `!` prefix | Negate — un-ignores a previously matched path |
| Pattern with no `/` | Matches the **basename** only (last path component) |
| Pattern with `/` | Matches the **full path** from the home root |

Ignored files are committed to the repo like any other file — they just aren't
applied to the destination. This is useful for machine-specific files that you
want to keep in the repo for reference but not deploy everywhere.

If you try to `dfiles add` a file that matches an ignore pattern, dfiles will
print a message and skip it. Remove the pattern from `config/ignore` first if
you want the file to be tracked and applied normally.

---

## Magic-name encoding

All file metadata is encoded in the source filename. No separate TOML file registry.

| Filename component | Decoded meaning |
|--------------------|-----------------|
| `dot_` prefix on any component | Replace with `.` |
| `private_` prefix on file | chmod 0600 (owner read/write only) |
| `private_` prefix on directory | chmod 0700 (owner rwx only) |
| `executable_` prefix | chmod 0755 (or 0700 if combined with `private_`) |
| `symlink_` prefix | Create a symlink pointing back into `source/` |
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

A module is a TOML file at `modules/<name>.toml`. Every section is optional.
Modules control Homebrew and mise **only** — they do not list source files.

All tracked files and externals live under `source/` using magic-name encoding.

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

**Adding externals:** Run `dfiles add <directory>`. If the directory is a git repo,
dfiles detects the remotes and prompts you to choose:

```
~/.config/nvim is a git repository with 1 remote:

  1) origin   https://github.com/user/nvim-config

How to add?
  1) Add as external (cloned on apply)
  f) Add all files recursively
  q) Skip
[1/f/q]:
```

**Apply behavior:**

- **Dest absent** — runs `git clone [--branch ref] url dest`
- **Dest is a git repo** — skipped by default (use `--apply-externals` to pull)
- **Dest is not a git repo** — hard error (remove manually first)

`dfiles status` reports `?` if the dest is absent and `M` if it exists but is
not a git repo.

### Homebrew

```toml
# modules/shell.toml
[homebrew]
brewfile = "brew/Brewfile.shell"
```

The `brewfile` field points to a Homebrew Brewfile relative to the repo root.
On apply, dfiles runs `brew bundle install --file=<path>`.
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
# modules/shell.toml
[mise]
config = "source/mise.toml"   # path relative to repo root; omit to use global config
```

On apply, dfiles runs `mise install`. If mise is not installed, the section is
skipped with a hint to install it.

### 1Password guard

```toml
# modules/secrets.toml
requires_op = true
```

Adding `requires_op = true` to any module causes dfiles to skip that module's
brew and mise steps with a warning if the `op` CLI is not installed or the user
is not signed in. Source files in `source/` are always applied regardless of
this flag.

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

## AI skills

Skills are declared in `ai/skills.toml` and deployed to platform directories
(e.g. `~/.claude/skills/`) by `dfiles apply`.

```toml
# ai/skills.toml

[[skill]]
name      = "pdf-processing"
source    = "gh:anthropics/skills/pdf-processing@v1.0"
platforms = "all"

[[skill]]
name      = "my-commands"
source    = "gh:me/my-commands@main"
platforms = ["claude-code"]

[[skill]]
name      = "local-skill"
source    = "dir:~/projects/my-skill"
platforms = "all"
```

### Source formats

| Format | Example | Description |
|--------|---------|-------------|
| `gh:owner/repo` | `gh:anthropics/skills/pdf` | GitHub repo or subdirectory. Optional `@ref` for a branch/tag. |
| `dir:~/path` | `dir:~/projects/my-skill` | Local directory. Not cached; read directly on each apply. |

### Platforms

| Value | Meaning |
|-------|---------|
| `"all"` | All active platforms in `ai/platforms.toml`, excluding `cross-client` |
| `"cross-client"` | Only the cross-client platform (`~/.agents/skills/`) |
| `["claude-code"]` | Explicit list, filtered to active platforms |

### Lock and supply chain protection

Every `gh:` skill source is fetched once and its SHA pinned in `dfiles.lock`.
On subsequent applies:

- **Cache hit, SHA matches** — uses cached copy, no network.
- **Cache miss, lock has SHA** — fetches and verifies the SHA matches the lock entry.
  A mismatch is an error (supply chain protection).
- **Lock has no entry** — fetches and records the SHA.

Use `dfiles ai update [name]` to intentionally pull a new version and update the lock.

### Managing skills

```
dfiles ai discover          # detect installed AI platforms, update ai/platforms.toml
dfiles ai add <source>      # add a [[skill]] entry to ai/skills.toml
dfiles ai fetch [name]      # download to cache without deploying
dfiles ai update [name]     # re-fetch + update lock SHAs
dfiles ai remove <name>     # remove from ai/skills.toml
dfiles ai search <query>    # search skills.sh for available skills
dfiles ai scan <dir>        # interactively import unmanaged skills from a directory
```

After adding or updating skills, run `dfiles apply --ai` to deploy them.

### Discovering and importing existing skills

If you already have skills installed in a skills directory (e.g. via `npx skills add`)
and want to bring them under dfiles management, use `dfiles ai scan`:

```
dfiles ai scan ~/.claude/skills
dfiles ai scan ~/.agents/skills
```

For each unmanaged skill, dfiles tries to identify its GitHub source by inspecting
git remotes or searching the skills.sh registry. It then prompts you to confirm,
edit, or skip each one. Confirmed skills are appended to `ai/skills.toml`.

To browse what's available on skills.sh before scanning:

```
dfiles ai search pdf
dfiles ai search browser --limit 5
```

dfiles regenerates `~/.claude/CLAUDE.md` after every successful apply so Claude
Code always has an accurate inventory of installed skills.

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
# source/dot_gitconfig.tmpl
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
# modules/secrets.toml
requires_op = true
```

If `op` is not installed or the user is not signed in, the module is skipped with
a warning instead of failing with an error. All other modules are applied normally.

---

## The lock file

`dfiles.lock` pins the SHA of every fetched GitHub source for reproducible installs.
Commit it alongside your config:

```toml
# dfiles.lock (auto-generated — do not edit by hand)
[skill."gh:anthropics/skills/pdf-processing"]
sha        = "abc123def456..."
fetched_at = "2026-03-21T10:00:00Z"
```

---

## Telemetry

dfiles can write a local usage log to `~/.dfiles/telemetry.jsonl`. This is **off by
default** and never sends any data anywhere — the file is yours to inspect and optionally
share with maintainers for usage analysis.

### Enabling

In `dfiles.toml`:
```toml
[telemetry]
enabled = true
```

Or per-invocation:
```sh
DFILES_TELEMETRY=1 dfiles apply
```

### Event format

One JSON object per line:
```json
{"ts":"2026-03-21T12:00:00Z","cmd":"apply","flags":["--dry-run"],"profile":"default","os":"macos","arch":"aarch64","duration_ms":234,"exit_ok":true}
```

Fields recorded: timestamp, command name, CLI flags (names only), profile, OS, CPU
architecture, wall-clock duration, and exit status. No file paths, usernames,
hostnames, or other personal data.

---

## VCS backend (git vs jj)

By default dfiles uses `git` for all clone and init operations. If you use
[Jujutsu (jj)](https://jj-vcs.github.io/jj/), you can tell dfiles to use
`jj git clone --colocate` instead, so all repos managed by dfiles are
colocated (they have both `.jj/` and `.git/`, so both `git` and `jj`
commands work in them).

### Choosing a backend

**Priority order** (first match wins):

| Source | Example |
|--------|---------|
| `--vcs` CLI flag | `dfiles init gh:alice/dotfiles --vcs jj` |
| `DFILES_VCS` env var | `DFILES_VCS=jj dfiles apply --apply-externals` |
| `vcs.backend` in `dfiles.toml` | `[vcs] / backend = "jj"` |
| Interactive detection | jj is on PATH, no config set → prompt |
| Default | `git` |

### Persisting the choice

Set `vcs.backend` in `dfiles.toml` so every command uses jj without extra flags:

```toml
[vcs]
backend = "jj"
```

Or let dfiles prompt you: if jj is installed but nothing is configured, the next
command that needs a VCS backend will ask which to use and offer to save the choice.

### What uses the configured backend

| Operation | git | jj |
|-----------|-----|----|
| `dfiles init --source <url>` | `git clone` | `jj git clone --colocate` |
| `dfiles apply --apply-externals` (new extdir) | `git clone --depth 1` | `jj git clone --colocate --depth 1` |
| `dfiles apply --apply-externals` (existing extdir, pull) | `git pull --ff-only` | `git pull --ff-only` |
| Skill cache | `git` sparse checkout | `git` sparse checkout (always) |

Pulling existing extdirs always uses `git pull --ff-only`, because this works
in colocated repos and jj has no equivalent single-command pull.

Skill cache cloning always uses git regardless of `vcs.backend`. The skill cache
uses git sparse checkout which is not yet supported by `jj git clone`.

### Migration: converting existing plain-git extdirs

When `vcs.backend = "jj"` is set and `dfiles apply --apply-externals` encounters
an extdir that already exists on disk but has no `.jj/` directory, dfiles will
prompt you to run `jj git init --colocate` in that directory. Choosing "always"
applies the migration to all remaining extdirs without further prompts.

### Inspecting the active backend

```sh
dfiles vcs
```

Output:

```
VCS backend: jj (colocated)  (set in dfiles.toml [vcs])
jj:          installed
dfiles.toml: /Users/alice/dfiles/dfiles.toml
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
dfiles uses natively. No translation needed for filenames.

| chezmoi source path | dfiles source path | destination |
|--------------------|--------------------|-------------|
| `dot_zshrc` | `source/dot_zshrc` | `~/.zshrc` |
| `dot_config/git/config` | `source/dot_config/git/config` | `~/.config/git/config` |
| `private_dot_ssh/id_rsa` | `source/private_dot_ssh/id_rsa` | `~/.ssh/id_rsa` (0600) |
| `executable_dot_local/bin/foo` | `source/executable_dot_local/bin/foo` | `~/.local/bin/foo` (0755) |

`.chezmoiexternal.toml` is also parsed. Each entry with `type = "git-repo"` is
converted to an `extdir_` marker file in `source/`.

### What gets converted

`.tmpl` files are converted from Go template syntax to Tera syntax during import:

| chezmoi construct | dfiles Tera equivalent |
|------------------|----------------------|
| `{{ .chezmoi.hostname }}` | `{{ hostname }}` |
| `{{ .chezmoi.username }}` | `{{ username }}` |
| `{{ .chezmoi.os }}` | `{{ os }}` |
| `{{ .chezmoi.homeDir }}` | `{{ get_env(name="HOME") }}` |
| `{{- if eq .chezmoi.os "darwin" -}}` | `{% if os == "macos" %}` |
| `{{- if eq .chezmoi.os "linux" -}}` | `{% if os == "linux" %}` |
| `{{- else if ... }}` | `{% elif ... %}` |
| `{{- else }}` | `{% else %}` |
| `{{- end }}` | `{% endif %}` |
| `{{/* comment */}}` | `{# comment #}` |

Constructs with no direct mapping are preserved as-is with a warning.

### What gets skipped

| chezmoi prefix/suffix | Reason |
|----------------------|--------|
| `symlink_` entries whose target cannot be resolved | Cannot copy a dangling symlink |
| `exact_` / `create_` / `modify_` | Unsupported chezmoi attributes |
| `run_once_` / `run_` / `once_` | Install/run scripts |
| `.chezmoi*` / `chezmoistate.*` | chezmoi-internal files |

### After importing

```
dfiles apply --dry-run        # verify the import looks correct
dfiles apply                  # deploy to this machine
```

Re-running import is safe — it is idempotent and never overwrites existing source files.

---

## Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `DFILES_DIR` | `~/dfiles` | Repo root directory |
| `DFILES_CLAUDE_DIR` | `~/.claude` | Claude Code directory (skills, CLAUDE.md) |
| `DFILES_TELEMETRY` | unset | Set to `1` to enable telemetry, `0` to force-disable |
| `DFILES_VCS` | unset | Set to `git` or `jj` to override the VCS backend |
