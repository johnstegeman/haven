# Concepts

Understanding haven's model makes everything else click. The core idea is simple: **the haven repo is the desired state of your machine**.

## The haven repo is the source of truth

The haven repo — stored at `~/.local/share/haven` by default — is a git repository that describes everything your machine should have:

- Which dotfiles should exist and what their contents should be
- Which Homebrew packages should be installed
- Which language runtimes should be available
- Which AI agent skills should be deployed

Running `haven apply` is the act of making your machine match that desired state. `haven status` and `haven diff` show you where reality has drifted from it.

This is a **declarative** model: you describe what you want, haven figures out what needs to happen to get there.

## The repo layout

Each directory in the repo has a distinct purpose:

```
~/.local/share/haven/
│
├── haven.toml          ← profiles and settings
├── haven.lock          ← pinned SHAs for all fetched external sources
│
├── source/             ← dotfiles: what files should exist on your machine
├── ai/                 ← AI skills: what agent skills should be installed
├── brew/               ← Homebrew: what packages should be installed
├── modules/            ← module definitions: which Brewfiles and runtimes belong together
└── config/             ← haven settings: ignore patterns, etc.
```

### `source/` — dotfiles

`source/` is the heart of the repo. Every file here represents a file that should exist on your machine. The destination path and behavior are encoded directly in the filename using **magic-name encoding**:

```
source/dot_zshrc                    →  ~/.zshrc
source/dot_config/git/config        →  ~/.config/git/config
source/private_dot_ssh/config       →  ~/.ssh/config  (chmod 0700 on the .ssh directory)
source/dot_local/bin/executable_foo →  ~/.local/bin/foo  (chmod 0755)
source/dot_gitconfig.tmpl           →  ~/.gitconfig  (rendered via Tera before writing)
source/dot_config/extdir_nvim       →  ~/.config/nvim  (cloned from git on apply)
```

There's no registry, no TOML file listing which files are tracked. The `source/` tree *is* the manifest — haven walks it to discover what exists.

### `ai/` — AI skills

`ai/` declares which AI agent skills should be installed on your machine.

```
ai/
├── platforms.toml          ← which AI platforms are active (Claude Code, Codex, etc.)
└── skills/
    └── pdf-processing/
        ├── skill.toml      ← source URL and target platforms
        ├── all.md          ← content injected into every platform's config
        └── claude-code.md  ← content injected only into Claude Code's CLAUDE.md
```

Each skill directory declares where the skill comes from (`gh:owner/repo@ref` or `dir:~/local/path`) and which platforms it should be deployed to. On `haven apply --ai`, skills are fetched, cached, and deployed to the appropriate platform directories.

Every `gh:` source is SHA-pinned in `haven.lock` — the lock file ensures you get exactly the skill version you declared.

### `brew/` — Homebrew packages

`brew/` holds Homebrew Brewfiles that declare which packages should be installed.

```
brew/
├── Brewfile              ← master Brewfile (all-machine packages)
├── Brewfile.shell        ← packages for the "shell" module
└── Brewfile.work         ← packages for the "work" module
```

Use `haven brew install` to install a package *and* record it in the Brewfile in one step. On `haven apply --brews`, haven runs `brew bundle install` for every active module's Brewfile.

### `modules/` — module definitions

A module is a named group of related packages. Modules tie a Brewfile (and optional mise config) to a name, so they can be activated per-profile:

```toml
# modules/shell.toml
[homebrew]
brewfile = "brew/Brewfile.shell"

[mise]
config = "source/mise.toml"
```

Modules control *packages only* — they don't scope dotfiles. Files are applied globally regardless of which profile is active.

### `config/` — haven settings

```
config/
└── ignore    ← gitignore-style patterns for files to exclude from apply/status/diff
```

`config/ignore` is a Tera template, so you can use conditionals like `{% if os == "macos" %}` to make ignore patterns platform-specific.

### `haven.toml` — top-level configuration

`haven.toml` ties everything together:

```toml
[profile.default]
modules = ["shell", "git", "packages"]

[profile.work]
extends = "default"
modules = ["work", "secrets"]

[data]
work_email = "alice@corp.example"

[security]
allow = ["~/.config/gh/hosts.yml"]

[vcs]
backend = "git"

[telemetry]
enabled = false
```

Profiles declare which modules are active on each machine. `[data]` provides custom variables for templates. `[security] allow` suppresses false positives in security scanning.

### `haven.lock` — pinned SHAs

`haven.lock` is auto-generated. It records the SHA256 hash of every fetched external source (AI skills, external files). Commit it alongside your config.

On apply, haven verifies fetched content against the recorded SHAs. A mismatch is an error — run `haven ai update` to explicitly accept new content. This is the supply chain protection model.

---

## How apply works

`haven apply` reads the repo and writes your machine toward the desired state:

1. **Files** — for each file in `source/`, decode its magic name, render templates if `.tmpl`, and write to the destination (backing up any existing file first)
2. **Externals** — for each `extdir_` marker, clone the referenced git repo if the destination doesn't exist
3. **Homebrew** — for each active module with a `[homebrew]` section, run `brew bundle install`
4. **mise** — for each active module with a `[mise]` section, run `mise install`
5. **AI skills** — for each declared skill, fetch/verify from cache and deploy to platform directories

Apply is **idempotent** — running it multiple times produces the same result.

## State

`~/.haven/state.json` records what was last applied. It's not committed — it's local to each machine. `haven status` compares the live files against `source/` to detect drift; it doesn't use state for file comparison, only for remembering which profile was last used.

Backups of overwritten files go to `~/.haven/backups/`. The cache of fetched skills lives in `~/.haven/skills/`.

## The mental model

Think of your haven repo as a **package** for your development environment — the way `package.json` + `yarn.lock` describe a JavaScript project's dependencies. You declare what you want, lock the versions, and anyone (or any machine) can reproduce the exact environment from that declaration.

`haven init gh:you/my-env --apply` is the developer environment equivalent of `git clone && npm install`.
