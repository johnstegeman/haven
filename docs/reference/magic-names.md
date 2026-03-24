# Magic-Name Encoding

haven encodes all file metadata directly in source filenames. There's no separate registry — the `source/` directory tree is the complete manifest.

This is the same encoding chezmoi uses, so migrating is straightforward.

## Prefixes

Prefixes appear at the beginning of any path component (file or directory name) and can be stacked in any order.

| Prefix | Effect on file | Effect on directory |
|--------|---------------|---------------------|
| `dot_` | Replace with `.` | Replace with `.` |
| `private_` | chmod 0600 | chmod 0700 |
| `executable_` | chmod 0755 | — |
| `symlink_` | Create a symlink pointing back into `source/` | — |
| `extdir_` | Marker file — clone a remote git repo into this directory | — |
| `create_only` | Only write if destination does not already exist | — |
| `exact_` | On apply, remove files in destination dir not present in source | — |

## Suffixes

| Suffix | Effect |
|--------|--------|
| `.tmpl` | Render through the Tera template engine before writing. Strip suffix from destination filename. |

## Stacking prefixes

Prefixes can be combined in any order. `private_executable_` and `executable_private_` are equivalent.

```
private_executable_dot_local/bin/s  →  ~/.local/bin/s  (chmod 0700)
```

## Examples

| `source/` path | Destination | Permissions |
|----------------|-------------|-------------|
| `dot_zshrc` | `~/.zshrc` | unchanged |
| `dot_config/git/config` | `~/.config/git/config` | unchanged |
| `private_dot_ssh/id_rsa` | `~/.ssh/id_rsa` | 0600 |
| `private_dot_ssh/` (dir) | `~/.ssh/` | 0700 |
| `executable_dot_local/bin/foo` | `~/.local/bin/foo` | 0755 |
| `private_executable_dot_local/bin/s` | `~/.local/bin/s` | 0700 |
| `symlink_vscode_settings.json` | `~/vscode_settings.json` | symlink |
| `dot_gitconfig.tmpl` | `~/.gitconfig` | unchanged (rendered) |
| `dot_config/extdir_nvim` | `~/.config/nvim` | git clone |

## How `haven add` encodes files

When you run `haven add ~/.ssh/config`, haven:

1. Inspects the file's permissions
2. Inspects the parent directory's permissions
3. Builds the encoded path: `private_dot_ssh/config`
4. Copies the file to `source/private_dot_ssh/config`

The encoding is deterministic — you can predict and construct it manually.

## Editing source files

You can edit source files directly in the repo:

```sh
# Open the source file for ~/.zshrc
$EDITOR $(haven source-path)/source/dot_zshrc

# After editing, apply to this machine
haven apply
```

Or see exactly where a destination maps to:

```sh
# Source path for ~/.gitconfig
ls $(haven source-path)/source/dot_gitconfig*
```
