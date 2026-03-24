# VCS Backend

By default haven uses `git` for clone and init operations. If you use [Jujutsu (jj)](https://jj-vcs.github.io/jj/), you can tell haven to use `jj git clone --colocate` instead, so all repos managed by haven have both `.jj/` and `.git/` directories and work with both tools.

## Choosing a backend

Haven resolves the VCS backend in this priority order (first match wins):

| Source | Example |
|--------|---------|
| `--vcs` CLI flag | `haven init gh:alice/dotfiles --vcs jj` |
| `HAVEN_VCS` env var | `HAVEN_VCS=jj haven apply --apply-externals` |
| `vcs.backend` in `haven.toml` | `[vcs] backend = "jj"` |
| Interactive detection | jj is on PATH, no config set → prompt |
| Default | `git` |

## Persisting the choice

Set `vcs.backend` in `haven.toml` so every command uses jj without extra flags:

```toml
[vcs]
backend = "jj"
```

Or let haven prompt you: if jj is installed but nothing is configured, the next command that needs a VCS backend will ask which to use and offer to save the choice.

## What uses the configured backend

| Operation | git | jj |
|-----------|-----|----|
| `haven init --source <url>` | `git clone` | `jj git clone --colocate` |
| `haven apply --apply-externals` (new extdir) | `git clone --depth 1` | `jj git clone --colocate --depth 1` |
| `haven apply --apply-externals` (existing extdir, pull) | `git pull --ff-only` | `git pull --ff-only` |
| Skill cache | `git` sparse checkout | `git` sparse checkout (always) |

!!! note
    Pulling existing extdirs always uses `git pull --ff-only` — this works in colocated repos and jj has no equivalent single-command pull.

    Skill cache cloning always uses git regardless of `vcs.backend`, because jj doesn't yet support git sparse checkout.

## Inspecting the active backend

```sh
haven vcs
```

Output:

```
VCS backend: jj (colocated)  (set in haven.toml [vcs])
jj:          installed
haven.toml: /Users/alice/.local/share/haven/haven.toml
```

## Migrating existing plain-git extdirs to jj

When `vcs.backend = "jj"` is set and `haven apply --apply-externals` encounters an extdir that exists on disk without a `.jj/` directory, haven prompts you to run `jj git init --colocate` in that directory. Choose "always" to apply the migration to all remaining extdirs without further prompts.
