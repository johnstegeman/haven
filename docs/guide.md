# dfiles User Guide

dfiles is an AI-first dotfiles and environment manager. It tracks your dotfiles,
Homebrew packages, language runtimes, and Claude Code skills in a single git
repository, and can reproduce your full development environment on any machine
from a single command.

---

## Concepts

**Repo** — a git repository (default: `~/dfiles`) that holds your config and source
files. You commit and push it like any other repo.

**Module** — a named group of files and packages defined in
`config/modules/<name>.toml`. Modules are the unit of organisation: `shell`, `git`,
`editor`, `packages`, `ai`, etc.

**Profile** — a named set of modules defined in `dfiles.toml`. Different machines
or contexts (work, personal, minimal) activate different subsets of modules.

**Source file** — your actual dotfile stored under `source/` inside the repo.
dfiles copies (or renders) it to its destination on apply.

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
├── source/                     # your actual dotfiles, stored verbatim
│   ├── zshrc
│   ├── gitconfig
│   └── ssh-config.tmpl         # .tmpl suffix → rendered before writing
│
└── config/
    └── modules/
        ├── shell.toml
        ├── git.toml
        ├── packages.toml
        └── ai.toml
```

Everything under `source/` and `config/` is committed to git.
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

### Track a file

```
dfiles add ~/.zshrc
dfiles add ~/.gitconfig --module git
```

Copies the file into `source/` and appends a `[[files]]` entry to the module's
TOML. The `--module` flag defaults to `shell`.

### Apply your config

```
dfiles apply
dfiles apply --profile work
dfiles apply --module shell      # apply one module only
dfiles apply --dry-run           # print the plan without writing anything
```

For each tracked file, dfiles backs up the existing destination file to
`~/.dfiles/backups/` before overwriting it.

### Check for drift

```
dfiles status
dfiles status --profile work
```

Compares each tracked source file against its live destination and reports:

| Marker | Meaning |
|--------|---------|
| `✓`    | File is in sync |
| `M`    | Destination exists but differs from source |
| `?`    | Destination does not exist (never applied) |
| `!`    | Source file is missing from the repo |

---

## Modules

A module is a TOML file at `config/modules/<name>.toml`. Every section is optional.
Sections: `[[files]]`, `[[externals]]`, `[homebrew]`, `[mise]`, `[ai]`.

### File entries

```toml
# config/modules/shell.toml

[[files]]
source = "zshrc"          # relative to source/ in the repo
dest   = "~/.zshrc"       # destination; ~ is expanded to the home directory

[[files]]
source = "ssh-config.tmpl"
dest   = "~/.ssh/config"
template = true           # render through the template engine before writing

[[files]]
source = "id_rsa"
dest   = "~/.ssh/id_rsa"
private = true            # chmod 0600 — owner read/write only

[[files]]
source = "deploy.sh"
dest   = "~/bin/deploy.sh"
executable = true         # chmod 0755 — owner/group/other can execute

[[files]]
source = "secret_script"
dest   = "~/.local/bin/secret_script"
private    = true         # combined: chmod 0700 — only owner can read, write, execute
executable = true
```

`template` defaults to `false`. Files without `template = true` are copied
verbatim — `{{ }}` inside them is never interpreted.

`private` defaults to `false`. When `true`, the destination file is set to mode
`0600` (owner read/write only). Use for SSH keys, API tokens, and other sensitive
files that should not be readable by other users.

`executable` defaults to `false`. When `true`, the destination file is set to mode
`0755`. When combined with `private = true`, the mode is `0700`.

| `private` | `executable` | Mode   | Typical use |
|-----------|-------------|--------|-------------|
| false     | false       | (unchanged) | Config files, dotfiles |
| true      | false       | 0600   | SSH keys, tokens, credentials |
| false     | true        | 0755   | Scripts, binaries in `~/bin` |
| true      | true        | 0700   | Private executable scripts |

### Externals

```toml
# config/modules/editor.toml

[[externals]]
dest = "~/.config/nvim"   # destination directory; ~ is expanded
type = "git"              # only "git" is currently supported
url  = "https://github.com/user/nvim-config"
ref  = "main"             # optional — branch, tag, or commit SHA
```

On apply, dfiles:
- **Absent dest** — runs `git clone --depth 1 [--branch ref] url dest`
- **Present git repo** — runs `git -C dest pull --ff-only`
- **Present non-git directory** — hard error (remove manually first)

`dfiles status` reports `?` if the dest directory is absent, and `M` if it
exists but is not a git repo.

Multiple externals can live in the same module:

```toml
[[externals]]
dest = "~/.config/nvim"
type = "git"
url  = "https://github.com/user/nvim-config"

[[externals]]
dest = "~/.config/tmux/plugins/tpm"
type = "git"
url  = "https://github.com/tmux-plugins/tpm"
ref  = "v3.1.0"
```

### Homebrew

```toml
[homebrew]
brewfiles = [
  "source/Brewfile.base",
  "source/Brewfile.dev",
]
```

On apply, dfiles runs `brew bundle install --file=<path>` for each Brewfile in
order. If Homebrew is not installed, this section is skipped with a warning.

Use `dfiles brew install` and `dfiles brew uninstall` instead of bare `brew`
commands to keep your Brewfiles automatically in sync:

```
dfiles brew install ripgrep           # brew install + adds brew "ripgrep" to Brewfile
dfiles brew install iterm2 --cask     # brew install --cask + adds cask "iterm2"
dfiles brew uninstall ripgrep         # brew uninstall + removes from all Brewfiles
```

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

Adding `requires_op = true` to any module causes dfiles to skip that module with
a warning if the `op` CLI is not installed or the user is not signed in. Use this
on any module whose templates call `{{ op(...) }}`.

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

Source files with `template = true` are rendered through the
[Tera](https://keats.github.io/tera/) template engine (Jinja2-compatible syntax)
before being written to their destination.

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
# source/gitconfig.tmpl — stored in the repo
[core]
{% if os == "macos" %}
  editor = /opt/homebrew/bin/nvim
{% else %}
  editor = /usr/bin/nvim
{% endif %}
```

### Example: Profile-conditional config

```
# source/zshrc.tmpl
export PATH="$HOME/.local/bin:$PATH"
{% if profile == "work" %}
source ~/.work-aliases
{% endif %}
```

### Example: Environment variable injection

```
# source/tool-config.tmpl
api_base = {{ get_env(name="API_BASE", default="https://api.example.com") }}
```

### Tera features

Full Jinja2-style control flow is available: `{% if %}`, `{% for %}`, filters,
macros, and `{# comments #}`. See the
[Tera documentation](https://keats.github.io/tera/docs/) for the complete
reference.

Files with `template = false` (the default) are **never** rendered — curly braces
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
# source/gh-hosts.yml.tmpl
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

[[files]]
source = "gh-hosts.yml.tmpl"
dest   = "~/.config/gh/hosts.yml"
template = true
```

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

Plain files are imported — both `dot_`-prefixed files (decoded to hidden names),
bare files (installed as-is), and files with `private_` or `executable_` prefixes
(imported with permission flags set automatically):

| chezmoi source path | dfiles destination | module | flags |
|--------------------|--------------------|---------|----|
| `dot_zshrc` | `~/.zshrc` | `shell` | — |
| `dot_config/git/config` | `~/.config/git/config` | `git` | — |
| `dot_config/nvim/init.lua` | `~/.config/nvim/init.lua` | `editor` | — |
| `Justfile` | `~/Justfile` | `misc` | — |
| `bin/myscript` | `~/bin/myscript` | `misc` | — |
| `private_dot_ssh/id_rsa` | `~/.ssh/id_rsa` | `misc` | `private = true` |
| `executable_dot_local/bin/foo` | `~/.local/bin/foo` | `misc` | `executable = true` |
| `private_executable_dot_local/bin/s` | `~/.local/bin/s` | `misc` | both flags |

The `private_` and `executable_` prefixes can appear in any order and may be
stacked (e.g. `private_executable_` and `executable_private_` are equivalent).

`.chezmoiexternal.toml` is also parsed. Each entry with `type = "git-repo"` is
converted to a `[[externals]]` entry in the appropriate module TOML:

| `.chezmoiexternal.toml` entry | dfiles result |
|-------------------------------|---------------|
| `["~/.config/nvim"]` / `type = "git-repo"` | `[[externals]]` in `editor.toml` |
| `["~/.config/tmux/plugins/tpm"]` / `type = "git-repo"` | `[[externals]]` in `shell.toml` |

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
existing source files or duplicate TOML entries.

---

## Command reference

### `dfiles init`

Initialize a new dfiles repo.

```
dfiles init [--dir <path>]
```

Creates `dfiles.toml`, `source/`, `config/modules/shell.toml`, and `.gitignore`.
Fails if already initialized.

### `dfiles add <file> [--module <name>]`

Track a dotfile.

```
dfiles add ~/.zshrc
dfiles add ~/.gitconfig --module git
```

Copies `<file>` to `source/` and appends a `[[files]]` entry to
`config/modules/<module>.toml`. The `--module` flag defaults to `shell`.
Warns before tracking files matching sensitive patterns (SSH keys, `.env`, etc.).

### `dfiles apply [--profile <name>] [--module <name>] [--dry-run]`

Apply tracked files and packages to this machine.

```
dfiles apply
dfiles apply --profile work
dfiles apply --module shell
dfiles apply --dry-run
```

- Backs up existing destination files to `~/.dfiles/backups/` before overwriting.
- Installs Homebrew packages, runs mise, and fetches AI skills/commands.
- Regenerates `~/.claude/CLAUDE.md` after every successful apply.
- Writes state to `~/.dfiles/state.json`.

### `dfiles status [--profile <name>]`

Show drift between tracked source files and live destinations.

```
dfiles status
dfiles status --profile work
```

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

Runs `brew install [--cask] <name>` and adds the entry to the appropriate
Brewfile in your repo. Brewfile resolution:

- If there is exactly one Brewfile across all modules → use it.
- If `--module` is specified → use that module's Brewfile (creates one if needed).
- If there are multiple Brewfiles and no `--module` → error with a list of options.
- If there are no Brewfiles yet → creates `source/Brewfile` and registers it in the
  `packages` module.

### `dfiles brew uninstall <name> [--cask]`

Uninstall a Homebrew formula and remove it from all Brewfiles in your repo.

```
dfiles brew uninstall ripgrep
dfiles brew uninstall iterm2 --cask
```

Scans every Brewfile across all modules, removes the matching entry, then runs
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
