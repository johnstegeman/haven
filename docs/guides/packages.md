# Managing Packages

haven manages Homebrew packages and mise language runtimes alongside your dotfiles, keeping everything in sync in a single repo.

## Homebrew

### Installing packages

Use `haven brew install` instead of bare `brew install` — it runs the install *and* updates your Brewfile so the package is tracked:

```sh
haven brew install ripgrep
haven brew install bat
haven brew install iterm2 --cask          # GUI apps and fonts
haven brew install ripgrep --module shell  # add to a specific module's Brewfile
```

### Uninstalling packages

```sh
haven brew uninstall ripgrep
haven brew uninstall iterm2 --cask
```

Removes the formula from **all** Brewfiles in the repo, then runs `brew uninstall`.

### Brewfile layout

| Path | Used when |
|------|-----------|
| `brew/Brewfile` | `haven brew install` with no `--module` |
| `brew/Brewfile.<name>` | `haven brew install --module <name>` |

You can also write Brewfiles directly:

```
# brew/Brewfile.shell
brew "fish"
brew "starship"
brew "ripgrep"
brew "fd"
brew "bat"
cask "iterm2"
```

### Applying Brewfiles

```sh
haven apply --brews                        # install packages from all active Brewfiles
haven apply --brews --module shell         # only the shell module's Brewfile
```

### Removing unreferenced packages

```sh
haven apply --brews --remove-unreferenced-brews   # auto-remove anything not in a Brewfile
haven apply --brews --interactive                  # prompt before removing each package
haven apply --brews --zap                          # remove + zap cask app data
```

### Checking Homebrew drift

```sh
haven status --brews    # which packages are present / missing
haven diff --brews      # detailed diff
```

## mise (language runtimes)

[mise](https://mise.jdx.dev/) manages language runtimes (Node.js, Python, Ruby, Go, etc.). haven integrates with it via module configuration.

### Configuring mise in a module

```toml
# modules/shell.toml
[mise]
config = "source/mise.toml"    # path relative to repo root
```

On apply, haven runs `mise install` using the specified config file. If mise is not installed, the section is skipped with a hint.

### Example mise.toml

```toml
# source/mise.toml
[tools]
node = "lts"
python = "3.12"
go = "latest"
```

Track this file with haven:

```sh
haven add ~/.mise.toml
# or reference source/mise.toml directly in your module
```

## Modules

Modules group Homebrew packages and mise configs under a name. They control *packages only* — dotfiles are tracked via magic-name encoding in `source/`, not modules.

### Creating a module

```toml
# modules/shell.toml
[homebrew]
brewfile = "brew/Brewfile.shell"

[mise]
config = "source/mise.toml"
```

```toml
# modules/work.toml
[homebrew]
brewfile = "brew/Brewfile.work"

requires_op = true    # skip if 1Password isn't available
```

### 1Password guard

Adding `requires_op = true` to a module causes haven to skip that module's brew and mise steps if the `op` CLI is not installed or the user is not signed in — instead of failing hard:

```toml
# modules/secrets.toml
requires_op = true

[homebrew]
brewfile = "brew/Brewfile.secrets"
```

## Profiles

Profiles control which modules are active on each machine. Declared in `haven.toml`:

```toml
[profile.default]
modules = ["shell", "git", "packages"]

[profile.work]
extends = "default"         # inherits all modules from default
modules = ["work", "secrets"]

[profile.personal]
extends = "default"
modules = ["personal"]

[profile.minimal]
modules = ["shell"]
```

`extends` gives you single-level inheritance — the parent's modules are applied first, then the child's are appended (duplicates removed).

### Applying a profile

```sh
haven apply --profile work
haven apply --profile minimal
haven status --profile work
```

The last-used profile is saved in `~/.haven/state.json` and reused automatically unless overridden.

### New machine tip

Commit `haven.toml` with all your profiles. On a new machine:

```sh
haven init gh:yourname/haven --apply --profile work
```

One command clones the repo and applies the right profile.

## Custom template variables

Add machine-specific variables to `haven.toml` for use in `.tmpl` files:

```toml
[data]
work_email    = "alice@corp.example"
kanata_path   = "/usr/local/bin/kanata"
homebrew_path = "/opt/homebrew"
```

Access them in any `.tmpl` file as `{{ data.<key> }}`. Run `haven data` to see all variables in scope.
