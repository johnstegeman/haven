# Command Equivalence

Quick reference mapping chezmoi commands to their haven equivalents.

## Daily workflow

| What you want to do | chezmoi | haven |
|--------------------|---------|-------|
| Apply all config | `chezmoi apply` | `haven apply` |
| Preview changes | `chezmoi diff` | `haven diff` |
| Summary of drift | `chezmoi status` | `haven status` |
| See tracked files | `chezmoi managed` | `haven list` |
| Find untracked dotfiles | — | `haven unmanaged` |
| Track a new file | `chezmoi add ~/.foo` | `haven add ~/.foo` |
| Re-track a changed file | `chezmoi re-add ~/.foo` | `haven add ~/.foo --update` |
| Stop tracking a file | `chezmoi forget ~/.foo` | `haven remove ~/.foo` |
| Edit source file | `chezmoi edit ~/.foo` | `$EDITOR $(haven source-path)/source/dot_foo` |
| Go to source dir | `chezmoi cd` | `cd $(haven source-path)` |
| Dry run | `chezmoi apply --dry-run` | `haven apply --dry-run` |
| Check for drift (CI) | `chezmoi verify` | `haven diff` (exits 1 on drift) |

## Migration and updates

| What you want to do | chezmoi | haven |
|--------------------|---------|-------|
| Import from chezmoi | — | `haven import --from chezmoi` |
| Pull latest + apply | `chezmoi update` | `cd ~/.local/share/haven && git pull && haven apply` |
| Upgrade the binary | `chezmoi upgrade` | `haven upgrade` |
| Clone + apply on new machine | `chezmoi init --apply gh:you/dotfiles` | `haven init gh:you/my-env --apply` |

## Templates and data

| What you want to do | chezmoi | haven |
|--------------------|---------|-------|
| Check template variables | `chezmoi data` | `haven data` |
| Preview a rendered template | `chezmoi cat ~/.foo` | `haven apply --dry-run --dest /tmp/staging` |
| Evaluate template expression | `chezmoi execute-template '{{ .chezmoi.os }}'` | *(not available)* |

## Secret management

| What you want to do | chezmoi | haven |
|--------------------|---------|-------|
| Read from 1Password | `{{ onepasswordField "..." "..." }}` | `{{ op(path="op://...") }}` |
| Read from environment | `{{ env "VAR" }}` | `{{ get_env(name="VAR") }}` |
| Bitwarden / LastPass / Vault | Built-in integrations | Not supported |
| age / GPG encryption in repo | Supported | Not supported |

## Features unique to haven

These don't have chezmoi equivalents:

```sh
haven brew install ripgrep     # install + add to Brewfile
haven brew uninstall ripgrep   # uninstall + remove from Brewfiles
haven apply --profile work     # apply a named profile
haven ai add gh:owner/repo     # add an AI skill
haven apply --ai               # deploy AI skills
haven security-scan            # scan tracked files for secrets
haven unmanaged                # find untracked dotfiles
haven vcs                      # show active VCS backend (git or jj)
```

## The `chezmoi cd` equivalent

```sh
# Go to source dir
cd $(haven source-path)

# Or set a shell alias
alias hcd='cd $(haven source-path)'

# Edit haven.toml directly
$EDITOR $(haven source-path)/haven.toml
```
