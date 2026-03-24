# New Machine Setup

This is where haven shines. One command takes you from a fresh OS install to a fully configured development environment.

## The one-command setup

```sh
# Install haven
curl -fsSL https://raw.githubusercontent.com/johnstegeman/haven/main/install.sh | sh

# Clone your environment repo and apply everything
haven init gh:you/my-env --apply
```

`haven init --apply` clones your repo, then runs `haven apply` — deploying your dotfiles, running Homebrew, triggering mise, and deploying AI skills in sequence.

## Selecting a profile

If you have multiple profiles (work, personal, minimal), specify which to use:

```sh
haven init gh:you/my-env --apply --profile work
haven init gh:you/my-env --apply --profile personal
```

See [Profiles & Modules](../guides/profiles-modules.md) for how to set up profiles.

## Applying sections selectively

If you want to apply only certain sections:

```sh
haven apply              # everything: files + brews + mise + ai skills
haven apply --files      # dotfiles only
haven apply --brews      # Homebrew packages only
haven apply --ai         # AI skills only
```

## Keeping your environment in sync

After the initial setup, staying in sync is straightforward:

```sh
# Pull the latest changes from your repo
cd ~/.local/share/haven && git pull

# Apply changes
haven apply
```

!!! note "No `haven update` shortcut"
    haven does not have a `haven update` command that wraps pull + apply. This is intentional: haven does not assume your VCS workflow. If you use jj:
    ```sh
    jj git fetch && jj rebase -d main@origin && haven apply
    ```

## Checking for drift after a pull

```sh
haven status    # quick overview of what's changed
haven diff      # full diff between source and live files
```

## Re-applying externals

External git repos (`extdir_` entries, like plugin managers) are cloned on first apply but not updated automatically. To pull updates:

```sh
haven apply --apply-externals
```

## Complete new machine checklist

1. Install haven
2. `haven init gh:you/my-env --apply --profile <name>`
3. Set up shell completions (see [Installation](installation.md))
4. Verify: `haven status`
5. If using 1Password: `op signin && haven apply` (to render secrets into templates)
6. If using AI skills: `haven apply --ai`
