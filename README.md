```
██████╗ ███████╗██╗██╗     ███████╗███████╗
██╔══██╗██╔════╝██║██║     ██╔════╝██╔════╝
██║  ██║█████╗  ██║██║     █████╗  ███████╗
██║  ██║██╔══╝  ██║██║     ██╔══╝  ╚════██║
██████╔╝██║     ██║███████╗███████╗███████║
╚═════╝ ╚═╝     ╚═╝╚══════╝╚══════╝╚══════╝

  dotfiles · packages · AI tools · managed as one
```

**dfiles** is a declarative, AI-first developer environment manager. It tracks your
dotfiles, Homebrew packages, language runtimes, and Claude Code skills in a single
git repository, and reproduces your complete development environment on any machine
from a single command.

```sh
dfiles bootstrap gh:you/my-env   # new machine: full environment in minutes
dfiles apply                      # sync changes to this machine
dfiles status                     # see what's drifted
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
working environment — `dfiles bootstrap gh:me/my-env`. That's the bar I set for this.

The other thing that bothered me: when I run `brew install` or install a new Claude
skill, my dotfiles repo doesn't know. It drifts. I have to manually update my
Brewfile and commit it. With dfiles, `dfiles brew install ripgrep` runs the install
*and* adds it to your Brewfile in one step. The repo stays in sync with reality.

Finally, I wanted the ability to share a complete, versioned AI development environment
as a package — the way npm packages share a JavaScript project. Something where someone
can say "here's my Claude Code setup, including all the skills and configs I use daily"
and others can install the whole thing with one command. That's the long-term vision
for dfiles: the package registry for AI developer environments.

So: I built dfiles because the AI era needs a tool that treats AI configurations as
first-class citizens alongside dotfiles and packages. Nothing else does all three
in one place.

---

## What it manages

| Thing | How |
|-------|-----|
| Dotfiles | Copied (or symlinked) to their destinations; template rendering via Tera |
| Homebrew packages | Brewfile-driven; `dfiles brew install` keeps Brewfiles in sync |
| Language runtimes | Via [mise](https://mise.jdx.dev/) config files |
| Claude Code skills | Fetched from `gh:owner/repo[@ref]`, pinned in `dfiles.lock` |
| Claude Code commands | Same `gh:` source format |
| Secrets | Read from 1Password at apply time via `{{ op(path="...") }}` |
| External git repos | Cloned/pulled to a destination directory |

---

## Quick start

### New repo

```sh
dfiles init
dfiles add ~/.zshrc
dfiles add ~/.gitconfig --module git
dfiles apply
```

### New machine

```sh
# Install dfiles first:
curl -fsSL https://dfiles.sh/install | sh    # coming soon
brew install dfiles-sh/tap/dfiles            # coming soon

# Then bootstrap your environment:
dfiles bootstrap gh:you/my-env
```

### Importing from chezmoi

```sh
dfiles import --from chezmoi          # auto-detects chezmoi source dir
dfiles import --from chezmoi --dry-run
```

Converts `dot_` prefixes, `private_`, `executable_`, `symlink_` entries, and
`.tmpl` Go templates (converted to Tera syntax). See [`docs/guide.md`](docs/guide.md)
for what's imported and what's skipped.

---

## Key commands

```sh
dfiles init                    # initialize a new dfiles repo
dfiles add ~/.zshrc            # start tracking a file
dfiles apply                   # deploy tracked files to this machine
dfiles apply --dry-run         # preview without writing anything
dfiles status                  # show drift between source and live files
dfiles brew install <formula>  # brew install + update Brewfile
dfiles brew uninstall <formula># brew uninstall + remove from Brewfile
dfiles import --from chezmoi   # migrate from chezmoi
dfiles bootstrap               # apply + status (+ optional remote fetch)
```

---

## Module config example

```toml
# config/modules/shell.toml
[[files]]
source = "zshrc"
dest   = "~/.zshrc"

[[files]]
source = "gitconfig.tmpl"
dest   = "~/.gitconfig"
template = true

[homebrew]
brewfiles = ["source/Brewfile"]

[mise]
config = "source/mise.toml"

[ai]
skills   = ["gh:gstack/standard-skills@v2"]
commands = ["gh:me/my-commands@main"]
```

---

## Documentation

- **[User guide](docs/guide.md)** — full reference: modules, profiles, templates,
  1Password, Brewfiles, externals, bootstrapping, importing
- **[Roadmap](TODOS.md)** — what's coming: SHA verification, `dfiles publish`,
  cross-machine diff, additional import formats

---

## Repo layout

```
~/dfiles/
├── dfiles.toml          # profiles: which modules each profile activates
├── dfiles.lock          # pinned SHA-256 for every fetched GitHub source
│
├── source/              # your dotfiles, stored verbatim
│   ├── zshrc
│   ├── gitconfig.tmpl   # .tmpl → rendered by Tera before writing
│   └── Brewfile
│
└── config/
    └── modules/
        ├── shell.toml
        ├── git.toml
        └── packages.toml
```

---

## License

MIT
