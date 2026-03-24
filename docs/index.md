# haven

**haven** is a declarative, AI-first developer environment manager. It tracks your dotfiles, Homebrew packages, language runtimes, and AI agent skills in a single git repository, and reproduces your complete development environment on any machine from a single command.

```sh
haven init gh:you/my-env --apply  # new machine: full environment in minutes
haven apply                        # sync changes to this machine
haven status                       # see what's drifted
```

---

## What it manages

| Thing | How |
|-------|-----|
| **Dotfiles** | Copied (or symlinked) to their destinations; flags encoded in the source filename |
| **Homebrew packages** | Brewfile-driven; `haven brew install` keeps Brewfiles in sync |
| **Language runtimes** | Via [mise](https://mise.jdx.dev/) config files |
| **AI skills** | Declared in `ai/skills/<name>/skill.toml`, fetched from GitHub, pinned in `haven.lock` |
| **Secrets** | Read from 1Password at apply time via `{{ op(path="...") }}` templates |
| **External git repos** | Cloned/pulled to a destination directory via `extdir_` markers |

---

## Why haven?

The AI era needs a tool that treats AI configurations as first-class citizens alongside dotfiles and packages. Nothing else does all three in one place.

When you set up a new machine today, you need your `.zshrc` and `.gitconfig` — but you also need the right Claude Code skills installed, the right MCP servers wired up, your Homebrew packages, your mise runtimes. chezmoi handles files. It has no concept of skills, commands, or AI tooling.

There's a bootstrap paradox baked into every dotfile manager: to be productive on a new machine you need your tools, but to get your tools you need to already be somewhat set up. haven solves this with one command:

```sh
haven init gh:me/my-env --apply
```

haven also keeps your repo in sync with reality. `haven brew install ripgrep` runs the install *and* adds it to your Brewfile. No manual commits after every `brew install`.

---

## Quick start

**New repo:**

```sh
haven init
haven add ~/.zshrc
haven add ~/.gitconfig
haven apply
```

**New machine:**

```sh
# Install haven
curl -fsSL https://raw.githubusercontent.com/johnstegeman/haven/main/install.sh | sh

# Clone and apply your environment
haven init gh:you/my-env --apply
```

**Migrating from chezmoi:**

```sh
haven init
haven import --from chezmoi
haven apply --dry-run
haven apply
```

---

## Key concepts

**Repo** — a git repository (default: `~/.local/share/haven`) containing all your config. You commit and push it like any other repo.

**Magic-name encoding** — all file metadata lives in the source filename itself, with no separate registry. `source/dot_zshrc` → `~/.zshrc`, `source/private_dot_ssh/id_rsa` → `~/.ssh/id_rsa` (chmod 0600). The same encoding chezmoi uses.

**Modules** — named groups of Homebrew packages and mise runtimes, declared in `modules/<name>.toml`. They control packages only — files are tracked entirely via their encoded filenames.

**Profiles** — named sets of modules declared in `haven.toml`. Different machines activate different profiles.

**Skills** — AI agent skills (Claude Code, Codex, etc.) declared in `ai/skills/`, fetched from GitHub, and deployed to platform-specific directories on apply.

---

## Next steps

- [Install haven](getting-started/installation.md) and set up your first repo
- Already using chezmoi? See [For chezmoi Users](for-chezmoi-users/index.md)
- Browse the [Command Reference](reference/commands.md)
