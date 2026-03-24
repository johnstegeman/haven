<p align="center">
  <img src="assets/logo-wide.svg" width="520" height="100" alt="haven — AI-first dotfiles &amp; environment manager"/>
</p>

<p align="center">
  <a href="https://johnstegeman.github.io/haven">Documentation</a>
</p>

---

**haven** is a declarative, AI-first developer environment manager. It tracks your
dotfiles, Homebrew packages, language runtimes, and Claude Code skills in a single
git repository, and reproduces your complete development environment on any machine
from a single command.

```sh
haven init gh:you/my-env --apply  # new machine: full environment in minutes
haven apply                        # sync changes to this machine
haven status                       # see what's drifted
```

---

## Why I built this

I was already managing dotfiles with chezmoi and liked it fine — until I started using
AI coding assistants seriously.

**The problem isn't the dotfiles. It's everything else that makes you productive.**

When I set up a new machine today, I need my `.zshrc` and `.gitconfig` — but I also
need the right Claude Code skills installed, the right MCP servers wired up, my
Homebrew packages, my mise runtimes, my editor config fetched from GitHub. chezmoi
handles files. It has no concept of skills, commands, or AI tooling. So I was
running half a dozen tools and still doing a bunch of manual steps after.

There's a bootstrap paradox baked into every dotfile manager: to be productive on a
new machine you need your tools, but to get your tools you need to already be
somewhat set up. The ideal is one command that takes you from fresh OS to fully
working environment — `haven init gh:me/my-env --apply`. That's the bar I set for this.

The other thing that bothered me: when I run `brew install` or install a new Claude
skill, my dotfiles repo doesn't know. It drifts. I have to manually update my
Brewfile and commit it. With haven, `haven brew install ripgrep` runs the install
*and* adds it to your Brewfile in one step. The repo stays in sync with reality.

Finally, I wanted the ability to share a complete, versioned AI development environment
as a package — the way npm packages share a JavaScript project. Something where someone
can say "here's my Claude Code setup, including all the skills and configs I use daily"
and others can install the whole thing with one command. That's the long-term vision
for haven: the package registry for AI developer environments.

So: I built haven because the AI era needs a tool that treats AI configurations as
first-class citizens alongside dotfiles and packages. Nothing else does all three
in one place.

---

## What it manages

| Thing | How |
|-------|-----|
| Dotfiles | Copied (or symlinked) to their destinations; flags encoded in the source filename |
| Homebrew packages | Brewfile-driven; `haven brew install` keeps Brewfiles in sync |
| Language runtimes | Via [mise](https://mise.jdx.dev/) config files |
| Claude Code skills | Declared in `ai/skills/<name>/skill.toml`, fetched from `gh:owner/repo[@ref]`, pinned in `haven.lock` |
| Secrets | Read from 1Password at apply time via `{{ op(path="...") }}` templates |
| External git repos | Cloned/pulled to a destination directory via `extdir_` markers |

---

## Quick start

### New repo

```sh
haven init
haven add ~/.zshrc
haven add ~/.gitconfig
haven apply
```

### New machine

```sh
# Install haven:
curl -fsSL https://raw.githubusercontent.com/johnstegeman/haven/main/install.sh | sh

# Clone and apply your environment:
haven init gh:you/my-env --apply
```

### Importing from chezmoi

```sh
haven import --from chezmoi          # auto-detects chezmoi source dir
haven import --from chezmoi --dry-run
```

Converts `dot_` prefixes, `private_`, `executable_`, `symlink_` entries, and
`.tmpl` Go templates (converted to Tera syntax). See the
[migration guide](https://johnstegeman.github.io/haven/for-chezmoi-users/migration/)
for what's imported and what's skipped.

---

## Key commands

```sh
haven init                    # initialize a new haven repo
haven add ~/.zshrc            # start tracking a file
haven remove ~/.zshrc         # stop tracking a file (live file untouched)
haven apply                   # deploy tracked files to this machine
haven apply --dry-run         # preview without writing anything
haven apply --dest ~/staging  # apply to a staging directory (for testing)
haven status                  # show drift between source and live files
haven diff                    # show file-level diff between source and live
haven source-path             # print the path to the haven repo
haven brew install <formula>  # brew install + update Brewfile
haven brew uninstall <formula># brew uninstall + remove from Brewfile
haven security-scan           # scan tracked files for secrets and credentials
haven completions fish        # generate fish shell completions
haven import --from chezmoi   # migrate from chezmoi
haven init gh:you/env --apply # new machine: clone and apply in one command
```

See the [command reference](https://johnstegeman.github.io/haven/reference/commands/) for the full list of flags.

---

## How files are tracked

haven uses **magic-name encoding** — all file metadata lives in the source filename
itself, with no separate TOML registry. The same encoding chezmoi uses, so migrating
is straightforward.

```
source/dot_zshrc                       →  ~/.zshrc
source/dot_config/git/config           →  ~/.config/git/config
source/private_dot_ssh/id_rsa          →  ~/.ssh/id_rsa          (chmod 0600)
source/executable_dot_local/bin/foo    →  ~/.local/bin/foo        (chmod 0755)
source/symlink_vscode_settings.json    →  ~/vscode_settings.json  (symlink)
source/dot_gitconfig.tmpl              →  ~/.gitconfig            (Tera template)
```

| Prefix/suffix | Meaning |
|---------------|---------|
| `dot_` | Replace with `.` |
| `private_` | chmod 0600 for files, 0700 for directories |
| `executable_` | chmod 0755 |
| `symlink_` | Create a symlink instead of copying |
| `extdir_` | Clone a remote git repo into this directory on apply |
| `.tmpl` suffix | Render through the Tera template engine before writing |

---

## AI skills

Each skill lives in its own directory under `ai/skills/`. Declare it with
`haven ai add` and then edit the generated `all.md` to add agent instructions.

```
ai/skills/
  pdf-processing/
    skill.toml       ← source, platforms, deploy method
    all.md           ← snippet injected into every platform's config file
    claude-code.md   ← snippet injected only into Claude Code's CLAUDE.md
  my-commands/
    skill.toml
    all.md
```

```toml
# ai/skills/pdf-processing/skill.toml
source    = "gh:anthropics/skills/pdf-processing@v1.0"
platforms = "all"
```

```toml
# ai/skills/my-commands/skill.toml
source    = "gh:me/my-commands@main"
platforms = ["claude-code"]
```

Fetched skills are pinned by SHA in `haven.lock` — a mismatch between the fetched
content and the recorded SHA is treated as an error (supply chain protection). Use
`haven ai update` to accept an intentional upgrade.

`haven apply` automatically injects skill snippets (from `all.md` /
`<platform>.md`) into platform config files (e.g. `~/.claude/CLAUDE.md`) using
HTML comment markers. The markers are added to your source file once and kept
up to date on every subsequent apply.

```sh
haven ai discover          # detect installed AI platforms
haven ai add gh:owner/repo # add a skill to ai/skills/<name>/
haven ai fetch             # download skills to cache without deploying
haven ai update            # re-fetch + update lock SHAs
haven ai search <query>    # search skills.sh registry
haven ai scan ~/.claude/skills  # import existing unmanaged skills
```

---

## Module and profile config

Modules control **packages and mise** — not files. Files are tracked entirely through
their encoded filenames in `source/`.

```toml
# modules/shell.toml

[homebrew]
brewfile = "brew/Brewfile.shell"

[mise]
config = "source/mise.toml"
```

Profiles control which modules are active on a given machine:

```toml
# haven.toml

[profile.default]
modules = ["shell", "git", "packages"]

[profile.work]
extends = "default"
modules = ["secrets"]       # work profile = default + secrets

[profile.minimal]
modules = ["shell"]
```

---

## Repo layout

```
~/.local/share/haven/       # default repo location
├── haven.toml              # profiles — which modules each profile activates
├── haven.lock              # pinned SHA for every fetched GitHub source
│
├── source/                  # dotfiles with magic-name encoded filenames
│   ├── dot_zshrc
│   ├── dot_gitconfig.tmpl          # .tmpl → rendered by Tera before writing
│   ├── private_dot_ssh/
│   │   └── id_rsa                  # private_ → chmod 0600
│   └── dot_config/
│       ├── git/config
│       └── extdir_nvim             # extdir_ → git clone into ~/.config/nvim
│
├── ai/                      # AI skill declarations and snippets
│   ├── platforms.toml              # active AI platforms
│   └── skills/
│       ├── pdf-processing/
│       │   ├── skill.toml          # source, platforms, deploy
│       │   ├── all.md              # snippet → every platform's config file
│       │   └── claude-code.md      # snippet → Claude Code only
│       └── my-commands/
│           ├── skill.toml
│           └── all.md
│
├── brew/                    # Homebrew Brewfiles
│   ├── Brewfile                    # master
│   └── Brewfile.shell              # module-specific
│
└── modules/                 # per-module package config
    ├── shell.toml
    ├── git.toml
    └── packages.toml
```

---

## Documentation

Full documentation at **[johnstegeman.github.io/haven](https://johnstegeman.github.io/haven)**:

- **[Concepts](https://johnstegeman.github.io/haven/concepts/)** — how haven models your environment
- **[Getting started](https://johnstegeman.github.io/haven/getting-started/installation/)** — install, quick start, new machine setup
- **[Guides](https://johnstegeman.github.io/haven/guides/tracking-files/)** — task-oriented how-tos
- **[For chezmoi users](https://johnstegeman.github.io/haven/for-chezmoi-users/)** — migration guide, command equivalence, feature gaps
- **[Command reference](https://johnstegeman.github.io/haven/reference/commands/)** — every flag for every command

---

## Security

- **Secret scanning** — `haven security-scan` checks every tracked file for secrets,
  sensitive filenames (`.env`, `id_rsa`, `.pem`…), and credential paths
  (`~/.aws/credentials`, `~/.kube/**`, `~/.ssh/**`…). Content patterns cover GitHub
  tokens, AWS keys, PEM private keys, OpenAI/Anthropic keys, and generic password
  assignments. `haven add` also runs a content scan automatically and prompts before
  saving any file that matches.
- **Supply chain protection** — `haven.lock` pins the SHA of every fetched skill.
  A mismatch between the live fetch and the recorded SHA is an error; you must run
  `haven ai update` to explicitly accept changed content.
- **No telemetry by default** — telemetry is off unless you enable it in `haven.toml`
  (`[telemetry] enabled = true`) or set `HAVEN_TELEMETRY=1`. When enabled, events
  are written locally to `~/.haven/telemetry.jsonl` — nothing leaves your machine.

---

## License

MIT
