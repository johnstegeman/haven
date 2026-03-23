# Getting Started with haven — for chezmoi users

This guide is for people who already have a working chezmoi setup and want to
migrate to haven. It covers the automated importer, what carries over unchanged,
what needs attention, and what chezmoi features do not exist yet in haven.

If you are new to dotfile managers entirely, start with [the user guide](guide.md).

---

## Why switch?

haven shares chezmoi's magic-name encoding (`dot_`, `private_`, `.tmpl`, etc.) —
so most of your source files work as-is. But haven adds three things chezmoi
doesn't have:

- **Homebrew and mise management** — packages and language runtimes declared
  alongside your dotfiles, applied from the same repo.
- **AI skill management** — `haven ai add/fetch/apply` fetches and deploys
  Claude Code skills (and other AI agent skills) from GitHub, with SHA-pinned
  supply-chain protection.
- **Profiles** — a single repo that applies different subsets of config to a
  work laptop vs. a personal machine vs. a minimal server.

If your workflow is "maintain dotfiles and manage packages in separate tools",
haven collapses that into one.

---

## Concepts that map directly

| chezmoi concept | haven equivalent |
|----------------|-------------------|
| Source directory (`~/.local/share/chezmoi`) | Repo root (`~/.local/share/haven`) |
| `source/` file tree | `source/` file tree (same magic-name encoding) |
| `dot_`, `private_`, `executable_`, `symlink_`, `.tmpl` | Identical — same encoding |
| `extdir_` in `.chezmoiexternal.toml` | `extdir_` marker files in `source/` |
| `.chezmoiignore` | `config/ignore` (Tera template evaluated at runtime — same behaviour as chezmoi) |
| `chezmoi apply` | `haven apply` |
| `chezmoi diff` | `haven diff` |
| `chezmoi status` | `haven status` |
| `chezmoi managed` | `haven list` |
| `chezmoi add` | `haven add` |
| `chezmoi forget`/`remove` | `haven remove` |
| `chezmoi upgrade` | `haven upgrade` |
| `.chezmoidata.yaml` / `.chezmoidata.toml` | `[data]` section in `haven.toml` |
| Go templates (`.tmpl` files) | Tera templates (`.tmpl` files) — syntax differs |

---

## Migration

### Step 1: Install haven

```sh
curl -fsSL https://raw.githubusercontent.com/johnstegeman/haven/main/install.sh | sh
```

### Step 2: Initialize a haven repo

```sh
haven init
```

This creates `~/.local/share/haven` with the
scaffolding: `haven.toml`, `source/`, `brew/`, `modules/shell.toml`, `.gitignore`.

### Step 3: Run the importer

```sh
haven import --from chezmoi
```

The importer locates your chezmoi source directory automatically (checks
`~/.local/share/chezmoi`). To point it elsewhere:

```sh
haven import --from chezmoi --source ~/my-chezmoi-dir
```

Always try `--dry-run` first to see what would happen:

```sh
haven import --from chezmoi --dry-run
```

### What the importer does

| Input | Action |
|-------|--------|
| Dotfiles (`dot_`, `private_`, `executable_`) | Copied to `source/` unchanged |
| Template files (`.tmpl` suffix) | Copied and Go template syntax converted to Tera |
| `.chezmoiexternal.toml` git repos | Converted to `extdir_` marker files in `source/` |
| `.chezmoiignore` | Converted to Tera template syntax and written to `config/ignore` |
| `.chezmoidata.yaml` / `.chezmoidata.toml` | Flat string values written to `[data]` in `haven.toml` |
| `symlink_` + `.tmpl` files | Converted: template renders to symlink target path |

### What the importer skips (with reasons)

| chezmoi item | Status in haven |
|-------------|-----------------|
| `modify_` scripts | Not supported — skipped with a message |
| `run_`, `run_once_`, `run_onchange_` scripts | Skipped with guidance |
| `exact_` and `create_` prefixes | Skipped — use `haven add` after migration |
| `.chezmoi*` internal files | Skipped — chezmoi-internal only |
| Nested/object data in `.chezmoidata.*` | Only flat string values are migrated |

After import, re-running is safe — it is idempotent and never overwrites existing
source files.

### Step 4: Verify

```sh
haven apply --dry-run
```

Review the plan. If everything looks right:

```sh
haven apply
```

Your files are now managed by haven.

---

## Template syntax conversion

The importer converts Go template syntax to Tera. Here is the full mapping:

| chezmoi / Go template | haven / Tera |
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

**OS name note:** chezmoi uses `"darwin"` for macOS; haven uses `"macos"`. The
importer rewrites these automatically.

**Custom data note:** chezmoi accesses custom data as `.key` in templates. haven
namespaces it under `data.key` to avoid collisions with built-in variables. The
importer rewrites all custom variable references and migrates the values from
`.chezmoidata.*` into `[data]` in `haven.toml`.

Go constructs with no Tera equivalent (custom functions, complex pipelines) are
preserved as-is with a warning. You will need to convert these manually.

### Checking template variables in scope

```sh
haven data
```

```
os         = macos
hostname   = my-laptop
username   = alice
home_dir   = /Users/alice
source_dir = /Users/alice/.local/share/haven

data.kanata_path = /usr/local/bin/kanata
data.work_email  = alice@corp.example
```

---

## Command equivalence

### Daily workflow

| What you want to do | chezmoi | haven |
|--------------------|---------|--------|
| Apply all config | `chezmoi apply` | `haven apply` |
| Apply one file | `chezmoi apply ~/.zshrc` | `haven apply` (all files, fast) |
| Preview changes | `chezmoi diff` | `haven diff` |
| Summary of drift | `chezmoi status` | `haven status` |
| See tracked files | `chezmoi managed` | `haven list` |
| Find untracked dotfiles | — | `haven unmanaged` |
| Track a new file | `chezmoi add ~/.foo` | `haven add ~/.foo` |
| Re-track a changed file | `chezmoi re-add ~/.foo` | `haven add ~/.foo --update` |
| Stop tracking a file | `chezmoi forget ~/.foo` | `haven remove ~/.foo` |
| Edit source file | `chezmoi edit ~/.foo` | `$EDITOR $(haven source-path)/source/dot_foo` |
| Go to source dir | `chezmoi cd` | `cd $(haven source-path)` |
| Dry run | `chezmoi apply --dry-run` | `haven apply --dry-run` |
| Check for drift on CI | `chezmoi verify` | `haven diff` (exits 1 on drift) |

### Migration and updates

| What you want to do | chezmoi | haven |
|--------------------|---------|--------|
| Import from chezmoi | — | `haven import --from chezmoi` |
| Pull latest + apply | `chezmoi update` | `cd ~/.local/share/haven && git pull && haven apply` |
| Upgrade the binary | `chezmoi upgrade` | `haven upgrade` |

### Templates and data

| What you want to do | chezmoi | haven |
|--------------------|---------|--------|
| Preview a rendered template | `chezmoi cat ~/.foo` | *(not available — use `haven apply --dry-run`)* |
| Check template variables | `chezmoi data` | `haven data` |
| Execute a template expression | `chezmoi execute-template '{{ .chezmoi.os }}'` | *(not available)* |

### Secret management

| What you want to do | chezmoi | haven |
|--------------------|---------|--------|
| Read from 1Password | `{{ onepasswordField "..." "..." }}` | `{{ op(path="op://...") }}` |
| Read from environment | `{{ env "VAR" }}` | `{{ get_env(name="VAR") }}` |
| Bitwarden / LastPass / Vault | Built-in integrations | Not yet supported |
| age / GPG encryption in repo | Supported | Not yet supported |

---

## What chezmoi does that haven does not (yet)

haven is younger than chezmoi. These are the known gaps as of v0.3.0:

| Feature | chezmoi | haven |
|---------|---------|--------|
| **In-repo encryption** | age and GPG support for encrypting files before committing | Not supported. Workaround: use `op()` to read secrets from 1Password at apply time instead of storing them in the repo |
| **`modify_` scripts** | Scripts that transform the existing destination file | Skipped on import. Workaround: convert to a `.tmpl` file with `get_env()` or `op()` calls |
| **`run_onchange_` scripts** | Re-run a script when its content changes | Not supported — only `run_once_` |
| **`chezmoi cat`** | Print rendered output of a template without applying | Not implemented. Workaround: `haven apply --dry-run --dest /tmp/staging` |
| **`chezmoi execute-template`** | Evaluate a template expression from the CLI | Not implemented |
| **`chezmoi chattr`** | Change the magic-name attributes of a tracked file | Not implemented. Rename the source file manually |
| **`chezmoi merge`** | Three-way merge when source and destination have both changed | Not implemented |
| **Multiple secret backends** | Bitwarden, LastPass, Vault, Keeper, Passbolt, etc. | Only 1Password via `op()` |
| **`chezmoi doctor`** | Diagnostic check of the environment | Not implemented |
| **`chezmoi dump`** | Dump the full source state as JSON/YAML | Not implemented |
| **Interactive template prompts** | `promptString`, `promptBool`, `promptChoice` | Not supported in templates |
| **Per-file templates for external archives** | `.chezmoiexternal.toml` with templated URLs | `extfile_` supports SHA verification; no templated URLs yet |

If any of these are blocking your migration, open an issue on GitHub or continue
using chezmoi for that file while managing the rest with haven.

---

## Setting up Homebrew with modules and profiles

This is where haven diverges from chezmoi. chezmoi tracks dotfiles; managing
packages is out of scope. haven tracks both in the same repo.

### Brewfiles

haven Brewfiles live at `brew/Brewfile` (master) and `brew/Brewfile.<module>`
(per-module). Create them manually or use `haven brew install` to add packages
and keep them in sync automatically:

```sh
# Add a formula to the master Brewfile
haven brew install ripgrep

# Add a formula to a named module's Brewfile
haven brew install ripgrep --module shell

# Add a cask
haven brew install iterm2 --cask

# Remove a package from all Brewfiles
haven brew uninstall ripgrep
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

Profiles live in `haven.toml` and control which modules apply on each machine:

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
haven apply --profile work
haven status --profile personal
```

**Workflow tip:** commit `haven.toml` with all your profiles. On a new machine,
run `haven init gh:yourname/haven --apply --profile <name>` to clone the repo
and apply the right profile in one step.

---

## Custom template variables

If you used `.chezmoidata.yaml` or `.chezmoidata.toml` for machine-specific
variables, the importer writes them into `[data]` in `haven.toml`:

```toml
# haven.toml
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

Run `haven data` to see all variables in scope, including the built-ins
(`os`, `hostname`, `username`, `home_dir`, `source_dir`).

---

## Importing AI skills

If you already have Claude Code skills installed under `~/.claude/skills/`,
haven can detect them and bring them under management:

```sh
# Scan and interactively import unmanaged skills
haven ai scan ~/.claude/skills
```

For each unmanaged skill, haven inspects the git remote or searches the
skills.sh registry to identify the source. You confirm, edit, or skip each one.
Confirmed skills are appended to `ai/skills.toml`.

After scanning:

```sh
haven apply --ai    # deploy and write ~/.claude/CLAUDE.md
```

### Adding new skills

```sh
# Browse what's available
haven ai search browser
haven ai search pdf

# Add a skill from GitHub
haven ai add gh:anthropics/skills/pdf-processing@v1.0

# Add a locally developed skill
haven ai add-local ~/projects/my-skill

# Deploy
haven apply --ai
```

### Skills in ai/skills.toml

```toml
[[skill]]
name      = "pdf-processing"
source    = "gh:anthropics/skills/pdf-processing@v1.0"
platforms = "all"

[[skill]]
name      = "my-commands"
source    = "repo:"             # bundled in this haven repo
platforms = ["claude-code"]
```

Every `gh:` source is SHA-pinned in `haven.lock`. Run `haven ai update` to
pull updates and refresh the pinned SHAs.

---

## Setting up a new machine

With chezmoi you run `chezmoi init --apply gh:yourname/dotfiles`. haven is similar:

```sh
# Install haven
curl -fsSL https://raw.githubusercontent.com/johnstegeman/haven/main/install.sh | sh

# Clone your repo and apply
haven init gh:yourname/haven --apply

# Or with a specific profile
haven init gh:yourname/haven --apply --profile work
```

`haven init --apply` clones the repo, then runs `haven apply`. If you also want
Homebrew packages and AI skills:

```sh
haven init gh:yourname/haven --apply
haven apply --brews          # install Homebrew packages
haven apply --ai             # deploy AI skills
```

Or all at once:

```sh
haven apply                  # applies everything (files + brews + mise + ai)
```

---

## Daily workflow after migration

After the initial migration, your workflow is almost identical to chezmoi's:

```sh
# Something drifted — check what changed
haven status
haven diff

# Snap the live file back into source
haven add ~/.zshrc --update

# Push the update
cd ~/.local/share/haven && git add -A && git commit -m "update zshrc" && git push

# Pull and apply on another machine
cd ~/.local/share/haven && git pull && haven apply
```

There is no `haven update` shortcut (equivalent to `chezmoi update`) — you pull
the repo manually. This is intentional: haven does not assume which VCS workflow
you prefer. If you use jj, `jj git fetch && jj rebase -d main@origin && haven apply`.

---

## Troubleshooting migration

### Templates that didn't convert cleanly

The importer emits a warning for each Go template construct it could not convert.
Look for `# haven: TODO` comments in the converted `.tmpl` files. Each one
marks a construct you need to translate manually.

Check the [template syntax table](#template-syntax-conversion) above and the
[Tera documentation](https://keats.github.io/tera/docs/) for reference.

### Files that were skipped

Run with `--dry-run` to see the full list of what would be skipped and why:

```sh
haven import --from chezmoi --dry-run 2>&1 | grep -i skip
```

For `modify_` scripts: the typical chezmoi use case is injecting an environment
variable or secret. Replace the `modify_` script with a `.tmpl` file that reads
the value via `{{ get_env(name="VAR") }}` or `{{ op(path="op://...") }}`.

### File applies but looks wrong

```sh
haven diff ~/.zshrc      # see exact diff between source and destination
haven apply --dry-run    # preview full apply plan
```

### Check if anything in your home dir is still untracked

```sh
haven unmanaged
haven unmanaged --path ~/.config --depth 4
```

### Security scan before committing

Run a scan on the imported source files to catch anything that shouldn't be in
the repo:

```sh
haven security-scan
haven security-scan --entropy    # also flag high-entropy strings
```

Add false positives to `[security] allow` in `haven.toml`.

---

## Keeping chezmoi around during the transition

You do not have to switch all at once. chezmoi and haven can coexist: they manage
different files. A pragmatic approach:

1. Run `haven import --from chezmoi --dry-run` to see the full picture.
2. Move your most-used dotfiles to haven first.
3. For any file that uses a chezmoi feature haven doesn't support yet (e.g.
   `modify_` scripts, age encryption), leave it in chezmoi.
4. Add the chezmoi source directory to `config/ignore` in haven so it isn't
   accidentally imported twice.

When you are ready to cut over fully, run `chezmoi unmanage` on each file
you've migrated and eventually `rm -rf ~/.local/share/chezmoi`.
