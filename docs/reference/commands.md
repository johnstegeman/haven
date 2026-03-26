# Command Reference

## Quick reference

```
haven init [source] [--branch <b>] [--apply] [--profile <p>]
haven list [--profile <p>] [--files] [--brews] [--ai]
haven add <file> [--link] [--apply] [--update]
haven remove <file> [--dry-run]
haven apply [--profile <p>] [--module <m>] [--dry-run]
            [--files] [--brews] [--ai]
            [--apply-externals]
            [--remove-unreferenced-brews] [--interactive] [--zap]
haven diff  [--profile <p>] [--module <m>]
            [--files] [--brews] [--ai]
            [--stat] [--color always|never|auto]
haven status [--profile <p>] [--files] [--brews] [--ai]
haven source-path
haven brew install <name> [--cask] [--module <m>]
haven brew uninstall <name> [--cask]
haven import --from chezmoi [--source <dir>] [--dry-run]
             [--include-ignored-files]
haven ai discover
haven ai add <source> [--name <n>] [--platforms <p>] [--deploy symlink|copy]
haven ai fetch [<name>]
haven ai update [<name>]
haven ai remove <name> [--yes]
haven ai search <query> [--limit <n>]
haven ai scan <path> [--dry-run]
haven data
haven unmanaged [--path <p>] [--depth <n>]
haven upgrade [--check] [--force]
haven telemetry [--enable] [--disable]
                [--note|--action|--bug|--question "<message>"]
                [--list] [--list-notes] [--list-actions] [--list-bugs] [--list-questions]
haven security-scan [--entropy]
haven completions fish|zsh|bash
haven vcs
```

## Global options

| Option | Default | Description |
|--------|---------|-------------|
| `--dir <path>` | `~/.local/share/haven` | haven repo directory. Also read from `HAVEN_DIR` env var. |

---

## `haven init`

Create or clone a haven repository. Use this once, on first-time setup. For subsequent re-provisioning of an already-initialized machine, use `haven apply`.

```sh
haven init [source] [--branch <b>] [--apply] [--profile <p>]
```

| Argument/Option | Description |
|-----------------|-------------|
| `source` | Optional. Git URL or `gh:owner/repo[@ref]`. Omit to create a blank scaffold. |
| `--branch <b>` | Branch to clone. Overrides any `@ref` in source. |
| `--apply` | Apply the cloned repo immediately after cloning. Requires a source. |
| `--profile <p>` | Profile to apply. Requires `--apply`. |

---

## `haven add`

Start tracking a dotfile by copying it into `source/`.

For directories: if the directory is a git repo, interactively prompts to track it as an external clone (re-cloned on apply) or recursively copy its files.

```sh
haven add <file> [--link] [--apply] [--update]
```

| Argument/Option | Description |
|-----------------|-------------|
| `file` | Path to the file or directory to track. |
| `--link` | Track as a symlink: on apply, the destination is symlinked back to `source/`. |
| `--apply` | Immediately install after adding. Only valid with `--link`. |
| `--update` | Re-copy the file even if already tracked. Without this flag, adding an already-tracked file is an error. |

---

## `haven remove`

Stop tracking a dotfile by removing it from `source/`. The live file on disk is **not** touched.

```sh
haven remove <file> [--dry-run]
```

| Argument/Option | Description |
|-----------------|-------------|
| `file` | Destination path to stop tracking. |
| `--dry-run` | Print what would be removed without deleting any files. |

---

## `haven apply`

Apply tracked files and packages to this machine.

By default all sections are applied. Use `--files`, `--brews`, and/or `--ai` to apply only specific sections.

```sh
haven apply [--profile <p>] [--module <m>] [--dry-run]
            [--files] [--brews] [--ai]
            [--apply-externals]
            [--remove-unreferenced-brews] [--interactive] [--zap]
```

| Option | Description |
|--------|-------------|
| `--profile <p>` | Profile to apply. Default: last-used profile, or `default`. |
| `--module <m>` | Scope brew and mise to this module. **File operations are always global.** |
| `--dry-run` | Print the plan without writing any files. |
| `--files` | Apply dotfile copies/symlinks. |
| `--brews` | Run `brew bundle install`. |
| `--ai` | Deploy AI skills. |
| `--apply-externals` | Pull (update) existing `extdir_` git clones. Without this, existing clones are left as-is. |
| `--remove-unreferenced-brews` | After installing, uninstall leaf formula/cask not in any active Brewfile. |
| `--interactive` | Like `--remove-unreferenced-brews` but prompts before removing. |
| `--zap` | Like `--remove-unreferenced-brews` but also passes `--zap` to cask uninstall, removing app data. |

---

## `haven diff`

Show the diff between tracked source files/packages and live state. Exits 0 when up to date, 1 when drift is found.

```sh
haven diff [--profile <p>] [--module <m>]
           [--files] [--brews] [--ai]
           [--stat] [--color always|never|auto]
```

| Option | Description |
|--------|-------------|
| `--profile <p>` | Profile to diff. |
| `--module <m>` | Scope brew and AI diff to this module. |
| `--files` | Diff dotfiles. |
| `--brews` | Diff Homebrew packages. |
| `--ai` | Diff AI skills. |
| `--stat` | Show summary (file names + change counts) instead of full diff. |
| `--color <mode>` | `always`, `never`, or `auto` (default). |

---

## `haven status`

Show drift between tracked source files and live destinations.

Drift markers: `✓` clean · `M` modified · `?` missing · `!` source missing

```sh
haven status [--profile <p>] [--files] [--brews] [--ai]
```

---

## `haven list`

List tracked files, Homebrew packages, and AI skills.

```sh
haven list [--profile <p>] [--files] [--brews] [--ai]
```

| Option | Description |
|--------|-------------|
| `--profile <p>` | Scope to a specific profile. |
| `--files` | Show tracked files only. |
| `--brews` | Show Homebrew packages only. |
| `--ai` | Show AI skills only. |

---

## `haven source-path`

Print the absolute path to the haven repo directory and exit.

```sh
haven source-path
# Use in scripts:
cd $(haven source-path)
$EDITOR $(haven source-path)/haven.toml
```

---

## `haven data`

Show all template variables available in `.tmpl` files on this machine.

```sh
haven data
```

---

## `haven brew install`

Install a Homebrew formula or cask and record it in a Brewfile.

```sh
haven brew install <name> [--cask] [--module <m>]
```

| Option | Description |
|--------|-------------|
| `--cask` | Install as a cask. |
| `--module <m>` | Record in this module's Brewfile. Default: master `brew/Brewfile`. |

## `haven brew uninstall`

Uninstall a formula or cask and remove it from all Brewfiles.

```sh
haven brew uninstall <name> [--cask]
```

---

## `haven import`

Import dotfiles from another dotfile manager (one-time migration). Currently only `chezmoi` is supported.

```sh
haven import --from chezmoi [--source <dir>] [--dry-run] [--include-ignored-files]
```

| Option | Description |
|--------|-------------|
| `--from <manager>` | Source format. Currently only `chezmoi`. |
| `--source <dir>` | Path to source directory. Auto-detected if not given. |
| `--dry-run` | Preview without writing any files. |
| `--include-ignored-files` | Import files matching `.chezmoiignore` patterns. |

---

## `haven ai discover`

Scan this machine for installed AI agent platforms and update `ai/platforms.toml`.

## `haven ai add`

Add a skill declaration.

```sh
haven ai add <source> [--name <n>] [--platforms <p>] [--deploy symlink|copy]
```

| Option | Description |
|--------|-------------|
| `source` | `gh:owner/repo[/subpath][@ref]` or `dir:~/path`. |
| `--name <n>` | Local name. Default: inferred from source. |
| `--platforms <p>` | `all`, `cross-client`, or comma-separated platform IDs. Default: `all`. |
| `--deploy <method>` | `symlink` (default) or `copy`. |

## `haven ai fetch`

Download skills to cache without deploying.

```sh
haven ai fetch [<name>]
```

## `haven ai update`

Pull latest versions + update lock SHAs.

```sh
haven ai update [<name>]
```

## `haven ai remove`

Remove a skill.

```sh
haven ai remove <name> [--yes]
```

## `haven ai search`

Search the [skills.sh](https://skills.sh) registry.

```sh
haven ai search <query> [--limit <n>]
```

## `haven ai scan`

Interactively import unmanaged skills from a directory.

```sh
haven ai scan <path> [--dry-run]
```

---

## `haven telemetry`

Manage local telemetry. Without flags, prints current status.

```sh
haven telemetry [--enable] [--disable]
                [--note|--action|--bug|--question "<message>"]
                [--list] [--list-notes] [--list-actions] [--list-bugs] [--list-questions]
```

| Flag | Description |
|------|-------------|
| `--enable` | Enable telemetry in `haven.toml`. |
| `--disable` | Disable telemetry in `haven.toml`. |
| `--note "<text>"` | Append a free-form note (ID prefix `N`). |
| `--action "<text>"` | Record a deliberate action (ID prefix `A`). |
| `--bug "<text>"` | Record a bug observed (ID prefix `B`). |
| `--question "<text>"` | Record a question for later investigation (ID prefix `Q`). |
| `--list` | Print all entries. |
| `--list-notes` | Print only notes. |
| `--list-actions` | Print only actions. |
| `--list-bugs` | Print only bugs. |
| `--list-questions` | Print only questions. |

Annotation flags always write to `~/.haven/telemetry.jsonl` regardless of whether telemetry is enabled. Each annotation gets an auto-generated sequenced ID (e.g. `B000001`).

---

## `haven upgrade`

Upgrade haven to the latest version.

```sh
haven upgrade [--check] [--force]
```

| Flag | Description |
|------|-------------|
| `--check` | Check for update without installing. Exits 0 when up to date, 1 when update is available. |
| `--force` | Install even if already on latest. |

If haven is installed in a system directory (e.g. `/usr/local/bin`), the write
will fail with a permission error. The command detects this and prompts:

```
error: Permission denied writing to /usr/local/bin/haven.
Retry with sudo? [y/N]
```

Answering `y` runs `sudo mv` + `sudo chmod 755` to complete the install without
repeating the download.

---

## `haven unmanaged`

Find files in `~` not tracked by haven.

```sh
haven unmanaged [--path <p>] [--depth <n>]
```

| Option | Description |
|--------|-------------|
| `--path <p>` | Directory to scan. Default: `~`. |
| `--depth <n>` | Maximum scan depth. Default: 3. |

---

## `haven security-scan`

Scan all tracked source files for secrets and sensitive content.

```sh
haven security-scan [--entropy]
```

| Option | Description |
|--------|-------------|
| `--entropy` | Also flag high-entropy strings (≥16 chars, Shannon entropy >4.5 bits/char). |

Exits 0 when clean, 1 when findings are reported.

---

## `haven completions`

Print a shell completion script to stdout.

```sh
haven completions <shell>    # fish, zsh, or bash
```

---

## `haven vcs`

Show the active VCS backend and its configuration source.

```sh
haven vcs
```
