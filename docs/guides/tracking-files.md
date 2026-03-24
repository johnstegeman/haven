# Tracking Files

## Adding files

```sh
haven add ~/.zshrc
haven add ~/.gitconfig
haven add ~/.config/git/config
```

haven copies the file into `source/` with its destination path and metadata encoded in the filename. Permissions are auto-detected: a file with mode 0600 gets `private_` prefix, executable bits get `executable_` prefix.

### Re-adding a changed file

```sh
haven add ~/.zshrc --update
```

Without `--update`, adding an already-tracked file is an error.

### Tracking as a symlink

```sh
haven add ~/.config/vscode/settings.json --link
```

On apply, the destination is symlinked back into `source/` rather than copied. Use this for files that applications manage themselves — symlinks mean the app's writes go directly into your repo.

### Adding directories

```sh
haven add ~/.config/nvim
```

If the directory is a git repository with remotes, haven prompts you:

```
~/.config/nvim is a git repository with 1 remote:

  1) origin   https://github.com/user/nvim-config

How to add?
  1) Add as external (cloned on apply)
  f) Add all files recursively
  q) Skip
[1/f/q]:
```

Choose **external** to track it as an `extdir_` entry (re-cloned on apply) or **files** to copy everything recursively.

## Stopping tracking

```sh
haven remove ~/.zshrc
haven remove ~/.zshrc --dry-run    # preview first
```

Deletes the `source/` copy. The live file on disk is **not** touched.

## Viewing tracked files

```sh
haven list                # all tracked files, packages, and skills
haven list --files        # files only
```

Output shows file annotations in parentheses:

```
~/.zshrc
~/.gitconfig          (template)
~/.ssh/config         (private)
~/.local/bin/delta    (extfile)
~/.config/nvim        (extdir)
~/.vimrc              (symlink)
```

## Ignoring files

Create `config/ignore` in your repo root to exclude files from `apply`, `status`, and `diff`:

```
# config/ignore

# Skip a specific file
.zshenv

# Glob patterns
.ssh/id_*
.config/*/history

# Match everything under a directory
.local/share/some-app/**

# Negate a previous match (un-ignore)
!.local/share/some-app/keep-this
```

`config/ignore` is a [Tera template](templates.md), so you can use conditionals:

```
{% if os == "macos" %}
.DS_Store
{% endif %}

# Always ignored
.ssh/id_*
```

### Pattern rules

| Syntax | Meaning |
|--------|---------|
| `#` at start | Comment |
| `*` | Any non-`/` characters |
| `**` | Any characters including `/` |
| `?` | Any single non-`/` character |
| `!` prefix | Negate — un-ignores a previously matched path |
| Pattern with no `/` | Matches **basename** only |
| Pattern with `/` | Matches **full path** from home root |

## Finding untracked files

```sh
haven unmanaged                   # scan ~ up to depth 3
haven unmanaged --path ~/.config  # scan a specific directory
haven unmanaged --depth 5         # scan deeper
```

Skips noisy directories automatically (caches, `.git`, `node_modules`, etc.). Use the output to decide what to start tracking:

```sh
haven unmanaged | head -10
haven add ~/.config/bat/config
```

## External git repos (extdir_)

External directories are git repos that haven clones into your home directory — plugin managers, separate config repos, etc. They live as `extdir_` marker files in `source/`.

### Example

```
source/dot_config/extdir_nvim
```

Content of the marker file:

```toml
type = "git"
url  = "https://github.com/user/nvim-config"
ref  = "main"   # optional: branch, tag, or commit SHA
```

On apply, haven clones this repo into `~/.config/nvim`.

### Apply behavior

| State | What happens |
|-------|--------------|
| Destination absent | `git clone [--branch ref] url dest` |
| Destination is a git repo | Skipped (use `--apply-externals` to pull) |
| Destination is not a git repo | Error — remove manually first |

```sh
haven apply --apply-externals    # also pull existing clones
```
