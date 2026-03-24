# Migration Guide

Step-by-step instructions for moving your chezmoi setup to haven.

## Before you start

Run `haven import --from chezmoi --dry-run` first to see exactly what would be imported and what would be skipped. No files are written in dry-run mode.

```sh
haven import --from chezmoi --dry-run
```

Review the output. Anything marked `skip` has a reason attached — use this to identify files that need manual attention.

## Step 1: Install haven

```sh
curl -fsSL https://raw.githubusercontent.com/johnstegeman/haven/main/install.sh | sh
```

## Step 2: Initialize a haven repo

```sh
haven init
```

Creates `~/.local/share/haven` with: `haven.toml`, `source/`, `brew/Brewfile`, `modules/shell.toml`, `.gitignore`.

## Step 3: Run the importer

```sh
haven import --from chezmoi
```

The importer locates your chezmoi source directory automatically (checks `~/.local/share/chezmoi`). To point it elsewhere:

```sh
haven import --from chezmoi --source ~/my-chezmoi-dir
```

The importer is idempotent — safe to re-run if you need to tweak and retry.

## What the importer does

| Input | Action |
|-------|--------|
| Dotfiles (`dot_`, `private_`, `executable_`) | Copied to `source/` unchanged |
| Template files (`.tmpl` suffix) | Copied, Go template syntax converted to Tera |
| `.chezmoiexternal.toml` git repos | Converted to `extdir_` marker files in `source/` |
| `.chezmoiignore` | Converted to Tera syntax, written to `config/ignore` |
| `.chezmoidata.yaml` / `.chezmoidata.toml` | Flat string values written to `[data]` in `haven.toml` |
| `symlink_` + `.tmpl` files | Template renders to symlink target path |

## What the importer skips

| chezmoi item | Status | Notes |
|-------------|--------|-------|
| `modify_` scripts | Skipped | Convert to `.tmpl` file using `get_env()` or `op()` |
| `run_`, `run_once_`, `run_onchange_` | Skipped | No equivalent in haven |
| `exact_` prefix | Skipped | Use `haven add` after migration |
| `create_` prefix | Skipped | Use `haven add` after migration |
| `.chezmoi*` internal files | Skipped | chezmoi-internal only |
| Nested data in `.chezmoidata.*` | Partially | Only flat string values are migrated |

!!! tip "Handling `modify_` scripts"
    The typical chezmoi `modify_` use case is injecting a secret into an existing file. Replace the script with a `.tmpl` file that reads the value via `{{ get_env(name="VAR") }}` or `{{ op(path="op://...") }}`.

## Step 4: Check for template conversion issues

After importing, look for `# haven: TODO` comments in `.tmpl` files — each one marks a Go template construct the importer could not convert automatically:

```sh
grep -r "haven: TODO" ~/.local/share/haven/source/
```

Refer to the [Template Conversion](templates.md) guide for the full syntax mapping.

## Step 5: Verify

```sh
haven apply --dry-run
```

Review the plan. If everything looks right:

```sh
haven apply
```

Your files are now managed by haven.

## Step 6: Run a security scan

```sh
haven security-scan
```

A good sanity check before committing — catches sensitive files that may have been imported accidentally.

## Step 7: Commit

```sh
cd ~/.local/share/haven
git init    # if not already
git add -A
git commit -m "initial haven setup (migrated from chezmoi)"
git remote add origin git@github.com:you/my-env.git
git push -u origin main
```

## Coexisting with chezmoi during transition

You don't have to switch all at once. chezmoi and haven can coexist — they manage different files.

1. Run `haven import --from chezmoi --dry-run` to see the full picture
2. Move your most-used dotfiles to haven first
3. For files using chezmoi features haven doesn't support yet (age encryption, `modify_` scripts), leave them in chezmoi
4. Add the chezmoi source directory to `config/ignore` in haven so it isn't accidentally imported again

When ready to cut over fully:

```sh
# For each migrated file, un-manage it from chezmoi
chezmoi forget ~/.zshrc
chezmoi forget ~/.gitconfig

# Eventually
rm -rf ~/.local/share/chezmoi
```

## Troubleshooting

### Templates that didn't convert cleanly

Look for `# haven: TODO` comments in converted `.tmpl` files and check the [Template Conversion](templates.md) guide.

### Files that were skipped

```sh
haven import --from chezmoi --dry-run 2>&1 | grep -i skip
```

### Check what's still untracked

```sh
haven unmanaged
haven unmanaged --path ~/.config --depth 4
```

### A file looks wrong after apply

```sh
haven diff ~/.zshrc       # exact diff between source and destination
haven apply --dry-run     # preview full apply plan
```
