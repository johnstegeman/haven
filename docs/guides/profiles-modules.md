# Profiles & Modules

Profiles and modules give you a single repo that applies the right config to each machine — work laptop, personal Mac, minimal server.

## The model

**Modules** control *packages* (Homebrew and mise). A module is a TOML file at `modules/<name>.toml` that points to a Brewfile and/or a mise config.

**Profiles** control *which modules* are active on a given machine. Profiles are declared in `haven.toml`.

**Files are not scoped to modules.** Every file in `source/` is applied on every `haven apply`, regardless of module or profile. Profiles only affect which packages get installed.

## Modules

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
```

```toml
# modules/secrets.toml
requires_op = true    # skip if 1Password isn't available
```

### Module fields

| Field | Description |
|-------|-------------|
| `[homebrew] brewfile` | Path to Brewfile, relative to repo root |
| `[mise] config` | Path to mise config file, relative to repo root |
| `requires_op` | If `true`, skip brew/mise if `op` is not installed/signed in |

### Applying a single module

```sh
haven apply --module shell      # brew + mise for the shell module only
haven apply --brews --module work  # Homebrew only, work module
```

!!! note
    `--module` scopes brew and mise operations only. Dotfiles in `source/` are always applied globally.

## Profiles

### Declaring profiles

```toml
# haven.toml

[profile.default]
modules = ["shell", "git", "packages"]

[profile.work]
extends = "default"           # inherits all modules from default
modules = ["work", "secrets"] # then adds these

[profile.personal]
extends = "default"
modules = ["personal"]

[profile.minimal]
modules = ["shell"]           # just the essentials
```

`extends` gives you single-level inheritance: the parent's modules come first, then the child's are appended with duplicates removed.

### Applying profiles

```sh
haven apply --profile work
haven apply --profile minimal
haven status --profile work
haven diff --profile personal
```

The last-used profile is saved in `~/.haven/state.json` and reused automatically on subsequent commands.

## Practical patterns

### Work vs. personal

```toml
[profile.default]
modules = ["shell", "git", "editor"]

[profile.work]
extends = "default"
modules = ["work-tools", "vpn-config", "secrets"]

[profile.personal]
extends = "default"
modules = ["games", "media"]
```

### Server profile

```toml
[profile.server]
modules = ["shell"]    # no GUI apps, no casks, minimal tooling
```

### Secrets isolated in a module

```toml
# modules/secrets.toml
requires_op = true

[homebrew]
brewfile = "brew/Brewfile.secrets"
```

The `requires_op = true` flag means this module is silently skipped on machines where you're not signed into 1Password.

## Custom data per profile

You can't currently set different `[data]` values per profile — `[data]` in `haven.toml` is global. Use template conditionals on the `profile` variable instead:

```
# source/dot_gitconfig.tmpl
[user]
{% if profile == "work" %}
  email = alice@corp.example
{% else %}
  email = alice@personal.example
{% endif %}
```

## Committing profiles

Commit `haven.toml` with all your profiles. On a new machine:

```sh
haven init gh:yourname/haven --apply --profile work
```

This clones the repo and applies the right profile in one step — no manual configuration needed.
