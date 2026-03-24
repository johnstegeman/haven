# Quick Start

This guide walks you through setting up a haven repository from scratch.

## Step 1: Initialize a repo

```sh
haven init
```

This creates `~/.local/share/haven` with:

- `haven.toml` — profile and module configuration
- `source/` — where tracked dotfiles live
- `brew/Brewfile` — master Homebrew package list
- `modules/shell.toml` — example module
- `.gitignore` — excludes `~/.haven/` (state, backups, cache)

!!! tip "Custom location"
    Override the default repo location:
    ```sh
    haven --dir /path/to/repo init
    # or permanently:
    export HAVEN_DIR=/path/to/repo
    ```

## Step 2: Track your dotfiles

```sh
haven add ~/.zshrc
haven add ~/.gitconfig
haven add ~/.config/git/config
```

haven copies each file into `source/` with its metadata encoded in the filename:

- `~/.zshrc` → `source/dot_zshrc`
- `~/.gitconfig` → `source/dot_gitconfig`
- `~/.ssh/config` → `source/private_dot_ssh/config` (chmod 0600 auto-detected)

No TOML registry needed — the filename is the complete record.

## Step 3: Apply to this machine

```sh
haven apply
```

haven copies source files to their destinations, installing any backups to `~/.haven/backups/` first. Preview without writing:

```sh
haven apply --dry-run
```

## Step 4: Check for drift

After you've made manual changes to live files:

```sh
haven status           # quick overview
haven diff             # full unified diff
```

Status markers:

| Marker | Meaning |
|--------|---------|
| `✓` | File is in sync |
| `M` | Destination exists but differs from source |
| `?` | Destination does not exist |
| `!` | Source file is missing |

## Step 5: Commit to git

```sh
cd ~/.local/share/haven
git init
git add -A
git commit -m "initial haven setup"
git remote add origin git@github.com:you/my-env.git
git push -u origin main
```

## Step 6: Add Homebrew packages

Instead of bare `brew install`, use haven so your Brewfile stays in sync:

```sh
haven brew install ripgrep
haven brew install bat
haven brew install iterm2 --cask
```

## What next?

- [Tracking Files](../guides/tracking-files.md) — symlinks, templates, external repos
- [Managing Packages](../guides/packages.md) — modules, profiles, mise runtimes
- [AI Skills](../guides/ai-skills.md) — manage Claude Code and other AI agent skills
- [New Machine Setup](new-machine.md) — apply this repo to a new machine
