# Getting Started with dfiles — for chezmoi users

This guide is for people who already have a working chezmoi setup and want to
migrate to dfiles. It covers the automated importer, what carries over unchanged,
what needs attention, and what chezmoi features do not exist yet in dfiles.

If you are new to dotfile managers entirely, start with [the user guide](guide.md).

---

## Why switch?

dfiles shares chezmoi's magic-name encoding (`dot_`, `private_`, `.tmpl`, etc.) —
so most of your source files work as-is. But dfiles adds three things chezmoi
doesn't have:

- **Homebrew and mise management** — packages and language runtimes declared
  alongside your dotfiles, applied from the same repo.
- **AI skill management** — `dfiles ai add/fetch/apply` fetches and deploys
  Claude Code skills (and other AI agent skills) from GitHub, with SHA-pinned
  supply-chain protection.
- **Profiles** — a single repo that applies different subsets of config to a
  work laptop vs. a personal machine vs. a minimal server.

If your workflow is "maintain dotfiles and manage packages in separate tools",
dfiles collapses that into one.

---

## Concepts that map directly

| chezmoi concept | dfiles equivalent |
|----------------|-------------------|
| Source directory (`~/.local/share/chezmoi`) | Repo root (`~/dfiles` or `~/.local/share/dfiles`) |
| `source/` file tree | `source/` file tree (same magic-name encoding) |
| `dot_`, `private_`, `executable_`, `symlink_`, `.tmpl` | Identical — same encoding |
| `extdir_` in `.chezmoiexternal.toml` | `extdir_` marker files in `source/` |
| `.chezmoiignore` | `config/ignore` (Tera template evaluated at runtime — same behaviour as chezmoi) |
| `chezmoi apply` | `dfiles apply` |
| `chezmoi diff` | `dfiles diff` |
| `chezmoi status` | `dfiles status` |
| `chezmoi managed` | `dfiles list` |
| `chezmoi add` | `dfiles add` |
| `chezmoi forget`/`remove` | `dfiles remove` |
| `chezmoi upgrade` | `dfiles upgrade` |
| `.chezmoidata.yaml` / `.chezmoidata.toml` | `[data]` section in `dfiles.toml` |
| Go templates (`.tmpl` files) | Tera templates (`.tmpl` files) — syntax differs |

---

## Migration

### Step 1: Install dfiles

```sh
curl -fsSL https://raw.githubusercontent.com/johnstegeman/dfiles/main/install.sh | sh
```

### Step 2: Initialize a dfiles repo

```sh
dfiles init
```

This creates `~/dfiles` (or `~/.local/share/dfiles` on a fresh install) with the
scaffolding: `dfiles.toml`, `source/`, `brew/`, `modules/shell.toml`, `.gitignore`.

### Step 3: Run the importer

```sh
dfiles import --from chezmoi
```

The importer locates your chezmoi source directory automatically (checks
`~/.local/share/chezmoi`). To point it elsewhere:

```sh
dfiles import --from chezmoi --source ~/my-chezmoi-dir
```

Always try `--dry-run` first to see what would happen:

```sh
dfiles import --from chezmoi --dry-run
```

### What the importer does

| Input | Action |
|-------|--------|
| Dotfiles (`dot_`, `private_`, `executable_`) | Copied to `source/` unchanged |
| Template files (`.tmpl` suffix) | Copied and Go template syntax converted to Tera |
| `.chezmoiexternal.toml` git repos | Converted to `extdir_` marker files in `source/` |
| `.chezmoiignore` | Converted to Tera template syntax and written to `config/ignore` |
| `.chezmoidata.yaml` / `.chezmoidata.toml` | Flat string values written to `[data]` in `dfiles.toml` |
| `symlink_` + `.tmpl` files | Converted: template renders to symlink target path |

### What the importer skips (with reasons)

| chezmoi item | Status in dfiles |
|-------------|-----------------|
| `modify_` scripts | Not supported — skipped with a message |
| `run_`, `run_once_`, `run_onchange_` scripts | Skipped with guidance |
| `exact_` and `create_` prefixes | Skipped — use `dfiles add` after migration |
| `.chezmoi*` internal files | Skipped — chezmoi-internal only |
| Nested/object data in `.chezmoidata.*` | Only flat string values are migrated |

After import, re-running is safe — it is idempotent and never overwrites existing
source files.

### Step 4: Verify

```sh
dfiles apply --dry-run
```

Review the plan. If everything looks right:

```sh
dfiles apply
```

Your files are now managed by dfiles.

---

## Template syntax conversion

The importer converts Go template syntax to Tera. Here is the full mapping:

| chezmoi / Go template | dfiles / Tera |
|----------------------|----------------|
| `{{ .chezmoi.hostname }}` | `{{ hostname }}` |
| `{{ .chezmoi.username }}` | `{{ username }}` |
| `{{ .chezmoi.os }}` | `{{ os }}` |
| `{{ .chezmoi.arch }}` | `{{ arch }}` |
| `{{ .chezmoi.homeDir }}` | `{{ get_env(name="HOME") }}` |
| `{{ .chezmoi.sourceDir }}` | `{{ source_dir }}` |
| `{{- if eq .chezmoi.os "darwin" -}}` | `{% if os == "macos" %}` |
| `{{- if eq .chezmoi.os "linux" -}}` | `{% if os == "linux" %}` |
| `{{- else if eq ... -}}` | `{% elif ... %}` |
| `{{- else -}}` | `{% else %}` |
| `{{- end -}}` | `{% endif %}` |
| `{{/* comment */}}` | `{# comment #}` |
| `{{ .someVar }}` (custom data) | `{{ data.someVar }}` |
| `{{ (index . "someVar") }}` | `{{ data.someVar }}` |
| `{{ if .someVar }}` | `{% if data.someVar %}` |
| `{{ env "VAR" }}` | `{{ get_env(name="VAR") }}` |

**OS name note:** chezmoi uses `"darwin"` for macOS; dfiles uses `"macos"`. The
importer rewrites these automatically.

**Custom data note:** chezmoi accesses custom data as `.key` in templates. dfiles
namespaces it under `data.key` to avoid collisions with built-in variables. The
importer rewrites all custom variable references and migrates the values from
`.chezmoidata.*` into `[data]` in `dfiles.toml`.

Go constructs with no Tera equivalent (custom functions, complex pipelines) are
preserved as-is with a warning. You will need to convert these manually.

### Checking template variables in scope

```sh
dfiles data
```

```
os         = macos
hostname   = my-laptop
username   = alice
home_dir   = /Users/alice
source_dir = /Users/alice/.local/share/dfiles

data.kanata_path = /usr/local/bin/kanata
data.work_email  = alice@corp.example
```

---

## Command equivalence

### Daily workflow

| What you want to do | chezmoi | dfiles |
|--------------------|---------|--------|
| Apply all config | `chezmoi apply` | `dfiles apply` |
| Apply one file | `chezmoi apply ~/.zshrc` | `dfiles apply` (all files, fast) |
| Preview changes | `chezmoi diff` | `dfiles diff` |
| Summary of drift | `chezmoi status` | `dfiles status` |
| See tracked files | `chezmoi managed` | `dfiles list` |
| Find untracked dotfiles | — | `dfiles unmanaged` |
| Track a new file | `chezmoi add ~/.foo` | `dfiles add ~/.foo` |
| Re-track a changed file | `chezmoi re-add ~/.foo` | `dfiles add ~/.foo --update` |
| Stop tracking a file | `chezmoi forget ~/.foo` | `dfiles remove ~/.foo` |
| Edit source file | `chezmoi edit ~/.foo` | `$EDITOR $(dfiles source-path)/source/dot_foo` |
| Go to source dir | `chezmoi cd` | `cd $(dfiles source-path)` |
| Dry run | `chezmoi apply --dry-run` | `dfiles apply --dry-run` |
| Check for drift on CI | `chezmoi verify` | `dfiles diff` (exits 1 on drift) |

### Migration and updates

| What you want to do | chezmoi | dfiles |
|--------------------|---------|--------|
| Import from chezmoi | — | `dfiles import --from chezmoi` |
| Pull latest + apply | `chezmoi update` | `cd ~/dfiles && git pull && dfiles apply` |
| Upgrade the binary | `chezmoi upgrade` | `dfiles upgrade` |

### Templates and data

| What you want to do | chezmoi | dfiles |
|--------------------|---------|--------|
| Preview a rendered template | `chezmoi cat ~/.foo` | *(not available — use `dfiles apply --dry-run`)* |
| Check template variables | `chezmoi data` | `dfiles data` |
| Execute a template expression | `chezmoi execute-template '{{ .chezmoi.os }}'` | *(not available)* |

### Secret management

| What you want to do | chezmoi | dfiles |
|--------------------|---------|--------|
| Read from 1Password | `{{ onepasswordField "..." "..." }}` | `{{ op(path="op://...") }}` |
| Read from environment | `{{ env "VAR" }}` | `{{ get_env(name="VAR") }}` |
| Bitwarden / LastPass / Vault | Built-in integrations | Not yet supported |
| age / GPG encryption in repo | Supported | Not yet supported |

---

## What chezmoi does that dfiles does not (yet)

dfiles is younger than chezmoi. These are the known gaps as of v0.3.0:

| Feature | chezmoi | dfiles |
|---------|---------|--------|
| **In-repo encryption** | age and GPG support for encrypting files before committing | Not supported. Workaround: use `op()` to read secrets from 1Password at apply time instead of storing them in the repo |
| **`modify_` scripts** | Scripts that transform the existing destination file | Skipped on import. Workaround: convert to a `.tmpl` file with `get_env()` or `op()` calls |
| **`run_onchange_` scripts** | Re-run a script when its content changes | Not supported — only `run_once_` |
| **`chezmoi cat`** | Print rendered output of a template without applying | Not implemented. Workaround: `dfiles apply --dry-run --dest /tmp/staging` |
| **`chezmoi execute-template`** | Evaluate a template expression from the CLI | Not implemented |
| **`chezmoi chattr`** | Change the magic-name attributes of a tracked file | Not implemented. Rename the source file manually |
| **`chezmoi merge`** | Three-way merge when source and destination have both changed | Not implemented |
| **Multiple secret backends** | Bitwarden, LastPass, Vault, Keeper, Passbolt, etc. | Only 1Password via `op()` |
| **`chezmoi doctor`** | Diagnostic check of the environment | Not implemented |
| **`chezmoi dump`** | Dump the full source state as JSON/YAML | Not implemented |
| **Interactive template prompts** | `promptString`, `promptBool`, `promptChoice` | Not supported in templates |
| **Per-file templates for external archives** | `.chezmoiexternal.toml` with templated URLs | `extfile_` supports SHA verification; no templated URLs yet |

If any of these are blocking your migration, open an issue on GitHub or continue
using chezmoi for that file while managing the rest with dfiles.

---

## Setting up Homebrew with modules and profiles

This is where dfiles diverges from chezmoi. chezmoi tracks dotfiles; managing
packages is out of scope. dfiles tracks both in the same repo.

### Brewfiles

dfiles Brewfiles live at `brew/Brewfile` (master) and `brew/Brewfile.<module>`
(per-module). Create them manually or use `dfiles brew install` to add packages
and keep them in sync automatically:

```sh
# Add a formula to the master Brewfile
dfiles brew install ripgrep

# Add a formula to a named module's Brewfile
dfiles brew install ripgrep --module shell

# Add a cask
dfiles brew install iterm2 --cask

# Remove a package from all Brewfiles
dfiles brew uninstall ripgrep
```

Or write the Brewfile directly:

```sh
# brew/Brewfile.shell
brew "fish"
brew "starship"
brew "ripgrep"
brew "fd"
brew "bat"
cask "iterm2"
```

### Modules

A module is a TOML file at `modules/<name>.toml`. It ties a Brewfile (and
optional mise config) to a name:

```toml
# modules/shell.toml
[homebrew]
brewfile = "brew/Brewfile.shell"

[mise]
config = "source/mise.toml"
```

Modules group related packages. You might have `shell.toml`, `work.toml`,
`dev.toml`, or `personal.toml`. Files in `source/` are not listed in
modules — their encoded filenames are the complete record.

### Profiles

Profiles live in `dfiles.toml` and control which modules apply on each machine:

```toml
[profile.default]
modules = ["shell", "git"]

[profile.work]
extends = "default"
modules = ["work", "secrets"]

[profile.personal]
extends = "default"
modules = ["personal"]

[profile.server]
modules = ["shell"]
```

`extends` inherits all parent modules first, then appends the child's list.

Apply a profile:

```sh
dfiles apply --profile work
dfiles status --profile personal
```

**Workflow tip:** commit `dfiles.toml` with all your profiles. On a new machine,
run `dfiles init gh:yourname/dfiles --apply --profile <name>` to clone the repo
and apply the right profile in one step.

---

## Custom template variables

If you used `.chezmoidata.yaml` or `.chezmoidata.toml` for machine-specific
variables, the importer writes them into `[data]` in `dfiles.toml`:

```toml
# dfiles.toml
[data]
work_email    = "alice@corp.example"
kanata_path   = "/usr/local/bin/kanata"
homebrew_path = "/opt/homebrew"
```

In any `.tmpl` file, access them as `{{ data.<key> }}`:

```sh
# source/dot_gitconfig.tmpl
[user]
  email = {{ data.work_email }}

[core]
  editor = {{ data.kanata_path }}
```

Run `dfiles data` to see all variables in scope, including the built-ins
(`os`, `hostname`, `username`, `home_dir`, `source_dir`).

---

## Importing AI skills

If you already have Claude Code skills installed under `~/.claude/skills/`,
dfiles can detect them and bring them under management:

```sh
# Scan and interactively import unmanaged skills
dfiles ai scan ~/.claude/skills
```

For each unmanaged skill, dfiles inspects the git remote or searches the
skills.sh registry to identify the source. You confirm, edit, or skip each one.
Confirmed skills are appended to `ai/skills.toml`.

After scanning:

```sh
dfiles apply --ai    # deploy and write ~/.claude/CLAUDE.md
```

### Adding new skills

```sh
# Browse what's available
dfiles ai search browser
dfiles ai search pdf

# Add a skill from GitHub
dfiles ai add gh:anthropics/skills/pdf-processing@v1.0

# Add a locally developed skill
dfiles ai add-local ~/projects/my-skill

# Deploy
dfiles apply --ai
```

### Skills in ai/skills.toml

```toml
[[skill]]
name      = "pdf-processing"
source    = "gh:anthropics/skills/pdf-processing@v1.0"
platforms = "all"

[[skill]]
name      = "my-commands"
source    = "repo:"             # bundled in this dfiles repo
platforms = ["claude-code"]
```

Every `gh:` source is SHA-pinned in `dfiles.lock`. Run `dfiles ai update` to
pull updates and refresh the pinned SHAs.

---

## Setting up a new machine

With chezmoi you run `chezmoi init --apply gh:yourname/dotfiles`. dfiles is similar:

```sh
# Install dfiles
curl -fsSL https://raw.githubusercontent.com/johnstegeman/dfiles/main/install.sh | sh

# Clone your repo and apply
dfiles init gh:yourname/dfiles --apply

# Or with a specific profile
dfiles init gh:yourname/dfiles --apply --profile work
```

`dfiles init --apply` clones the repo, then runs `dfiles apply`. If you also want
Homebrew packages and AI skills:

```sh
dfiles init gh:yourname/dfiles --apply
dfiles apply --brews          # install Homebrew packages
dfiles apply --ai             # deploy AI skills
```

Or all at once:

```sh
dfiles apply                  # applies everything (files + brews + mise + ai)
```

---

## Daily workflow after migration

After the initial migration, your workflow is almost identical to chezmoi's:

```sh
# Something drifted — check what changed
dfiles status
dfiles diff

# Snap the live file back into source
dfiles add ~/.zshrc --update

# Push the update
cd ~/dfiles && git add -A && git commit -m "update zshrc" && git push

# Pull and apply on another machine
cd ~/dfiles && git pull && dfiles apply
```

There is no `dfiles update` shortcut (equivalent to `chezmoi update`) — you pull
the repo manually. This is intentional: dfiles does not assume which VCS workflow
you prefer. If you use jj, `jj git fetch && jj rebase -d main@origin && dfiles apply`.

---

## Troubleshooting migration

### Templates that didn't convert cleanly

The importer emits a warning for each Go template construct it could not convert.
Look for `# dfiles: TODO` comments in the converted `.tmpl` files. Each one
marks a construct you need to translate manually.

Check the [template syntax table](#template-syntax-conversion) above and the
[Tera documentation](https://keats.github.io/tera/docs/) for reference.

### Files that were skipped

Run with `--dry-run` to see the full list of what would be skipped and why:

```sh
dfiles import --from chezmoi --dry-run 2>&1 | grep -i skip
```

For `modify_` scripts: the typical chezmoi use case is injecting an environment
variable or secret. Replace the `modify_` script with a `.tmpl` file that reads
the value via `{{ get_env(name="VAR") }}` or `{{ op(path="op://...") }}`.

### File applies but looks wrong

```sh
dfiles diff ~/.zshrc      # see exact diff between source and destination
dfiles apply --dry-run    # preview full apply plan
```

### Check if anything in your home dir is still untracked

```sh
dfiles unmanaged
dfiles unmanaged --path ~/.config --depth 4
```

### Security scan before committing

Run a scan on the imported source files to catch anything that shouldn't be in
the repo:

```sh
dfiles security-scan
dfiles security-scan --entropy    # also flag high-entropy strings
```

Add false positives to `[security] allow` in `dfiles.toml`.

---

## Keeping chezmoi around during the transition

You do not have to switch all at once. chezmoi and dfiles can coexist: they manage
different files. A pragmatic approach:

1. Run `dfiles import --from chezmoi --dry-run` to see the full picture.
2. Move your most-used dotfiles to dfiles first.
3. For any file that uses a chezmoi feature dfiles doesn't support yet (e.g.
   `modify_` scripts, age encryption), leave it in chezmoi.
4. Add the chezmoi source directory to `config/ignore` in dfiles so it isn't
   accidentally imported twice.

When you are ready to cut over fully, run `chezmoi unmanage` on each file
you've migrated and eventually `rm -rf ~/.local/share/chezmoi`.
