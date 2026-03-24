# haven for chezmoi Users

If you're a chezmoi user looking at haven, here's the short version: **most of your source files work as-is.** haven uses the same magic-name encoding as chezmoi (`dot_`, `private_`, `executable_`, `symlink_`, `.tmpl`). The importer handles the rest.

The reason to switch is what chezmoi doesn't do: haven adds Homebrew and mise management, AI skill management, and profiles — all in the same repo, applied from the same command.

## The key differences

| | chezmoi | haven |
|--|---------|-------|
| **Dotfile encoding** | `dot_`, `private_`, `executable_`, `.tmpl` | Same encoding |
| **Template engine** | Go templates | Tera (Jinja2-compatible) |
| **Package management** | Not supported | Homebrew + mise, per-module |
| **AI skill management** | Not supported | First-class, SHA-pinned |
| **Profiles** | Not supported | Named module sets in `haven.toml` |
| **Secret backends** | Bitwarden, LastPass, Vault, 1Password, age, GPG | 1Password only |
| **Run scripts** | `run_`, `run_once_`, `run_onchange_` | Not supported |
| **In-repo encryption** | age + GPG | Not supported |

## Why switch?

**You already manage dotfiles with chezmoi and it's working fine** — until you start using AI coding assistants seriously, or find yourself running separate tools for packages, runtimes, and AI config.

The problem isn't dotfiles. It's everything else that makes you productive. haven is a single repo, a single command, a single source of truth for your entire development environment.

**If your workflow is:** dotfiles in chezmoi, packages via bare `brew install`, AI skills installed manually — haven collapses that into one.

**If you need:** age/GPG encryption in the repo, Bitwarden/LastPass integration, `modify_` scripts, `run_onchange_` hooks — chezmoi is the better choice today. See [Feature Gaps](gaps.md).

## What chezmoi concepts map to in haven

| chezmoi | haven |
|---------|-------|
| `~/.local/share/chezmoi` | `~/.local/share/haven` |
| `source/` tree | `source/` tree (same encoding) |
| `dot_`, `private_`, `executable_`, `.tmpl` | Identical |
| `.chezmoiexternal.toml` (git-repo entries) | `extdir_` marker files in `source/` |
| `.chezmoiignore` | `config/ignore` (Tera template) |
| `.chezmoidata.yaml` | `[data]` section in `haven.toml` |
| `chezmoi apply` | `haven apply` |
| `chezmoi diff` | `haven diff` |
| `chezmoi status` | `haven status` |
| `chezmoi managed` | `haven list` |
| `chezmoi add` | `haven add` |
| `chezmoi forget` | `haven remove` |
| `chezmoi upgrade` | `haven upgrade` |
| `chezmoi data` | `haven data` |
| Go templates | Tera templates (Jinja2-compatible syntax) |

## Getting started

1. [Migration Guide](migration.md) — step-by-step import from chezmoi
2. [Command Equivalence](commands.md) — your chezmoi commands, haven style
3. [Template Conversion](templates.md) — Go template → Tera syntax mapping
4. [Feature Gaps](gaps.md) — what chezmoi does that haven doesn't (yet)
