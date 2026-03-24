# Repo Layout

The default haven repo lives at `~/.local/share/haven`. Everything here is committed to git — except `~/.haven/` (state, backups, skill cache), which is added to `.gitignore` by `haven init`.

```
~/.local/share/haven/
│
├── haven.toml                     ← profiles, data variables, security allow-list, VCS, telemetry
├── haven.lock                     ← SHA-pinned sources for all fetched external content
│
├── source/                        ← dotfiles with magic-name encoded filenames
│   ├── dot_zshrc                  → ~/.zshrc
│   ├── dot_gitconfig.tmpl         → ~/.gitconfig  (rendered through Tera)
│   ├── private_dot_ssh/
│   │   └── config                 → ~/.ssh/config  (chmod 0600)
│   └── dot_config/
│       ├── git/config             → ~/.config/git/config
│       └── extdir_nvim            → ~/.config/nvim  (git clone on apply)
│
├── ai/                            ← AI skill declarations
│   ├── platforms.toml             ← active platforms (claude-code, codex, etc.)
│   └── skills/
│       ├── pdf-processing/
│       │   ├── skill.toml         ← source, platforms, deploy method
│       │   ├── all.md             ← snippet → every active platform's config
│       │   └── claude-code.md     ← snippet → Claude Code's CLAUDE.md only
│       └── my-commands/
│           ├── skill.toml
│           └── all.md
│
├── brew/                          ← Homebrew Brewfiles
│   ├── Brewfile                   ← master (used with no --module flag)
│   ├── Brewfile.shell             ← module-specific
│   └── Brewfile.work
│
├── modules/                       ← per-module package configuration
│   ├── shell.toml                 ← [homebrew] + [mise] for the "shell" module
│   ├── git.toml
│   └── work.toml
│
└── config/
    └── ignore                     ← gitignore-style patterns (Tera template)
```

## State directory (`~/.haven/`)

Not committed. Local to each machine.

```
~/.haven/
├── state.json       ← last-applied profile and file state
├── backups/         ← copies of files overwritten by apply
└── skills/          ← cached skill content fetched from GitHub
```

## What to commit

Everything in the repo root: `haven.toml`, `haven.lock`, `source/`, `ai/`, `brew/`, `modules/`, `config/`.

Do **not** commit `~/.haven/` — haven adds it to `.gitignore` automatically.
