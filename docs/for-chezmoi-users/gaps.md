# Feature Gaps

haven is younger than chezmoi. These are the known gaps as of v0.5.0.

## What chezmoi does that haven doesn't (yet)

| Feature | chezmoi | haven | Workaround |
|---------|---------|-------|-----------|
| **In-repo encryption** | age and GPG encryption for files before committing | Not supported | Use `{{ op(path="...") }}` to read secrets from 1Password at apply time |
| **`modify_` scripts** | Scripts that transform the existing destination file | Skipped on import | Convert to a `.tmpl` file using `get_env()` or `op()` |
| **`run_onchange_` scripts** | Re-run a script when its content changes | Not supported | |
| **`run_once_` scripts** | Run a script only on first apply | Not supported | |
| **`chezmoi cat`** | Print the rendered output of a template without applying | Not implemented | `haven apply --dry-run --dest /tmp/staging` |
| **`chezmoi execute-template`** | Evaluate a template expression from the CLI | Not implemented | |
| **`chezmoi chattr`** | Change magic-name attributes of a tracked file | Not implemented | Rename the source file manually |
| **`chezmoi merge`** | Three-way merge when source and destination have both changed | Not implemented | |
| **Multiple secret backends** | Bitwarden, LastPass, Vault, Keeper, Passbolt, 1Password | Only 1Password via `op()` | |
| **`chezmoi doctor`** | Diagnostic check of the environment | Not implemented | |
| **Interactive template prompts** | `promptString`, `promptBool`, `promptChoice` | Not supported | Use `get_env()` with a pre-set environment variable |
| **Templated external URLs** | `.chezmoiexternal.toml` with template expressions in URLs | Not supported | Hardcode the URL or use a branch ref |
| **`exact_` prefix** | Remove untracked files in a directory on apply | Not imported | Add directory manually with `haven add` |
| **`create_` prefix** | Create file only if it doesn't exist | Partially supported | Use `create_only` suffix |

## What haven does that chezmoi doesn't

These are the reasons to use haven if you're a chezmoi user:

| Feature | Description |
|---------|-------------|
| **Homebrew management** | `haven brew install` installs and tracks in Brewfile simultaneously. Module-scoped Brewfiles. `--remove-unreferenced-brews` to clean up drift. |
| **mise integration** | Language runtimes declared per-module, installed on `haven apply`. |
| **AI skill management** | `haven ai add/fetch/update/remove` manages Claude Code and other agent skills with SHA-pinned supply chain protection. |
| **Profiles** | Named module sets in `haven.toml`. `haven init gh:you/repo --apply --profile work` on new machines. |
| **`haven unmanaged`** | Find dotfiles in `~` that aren't tracked yet. |
| **`haven security-scan`** | Scan tracked files for accidentally committed secrets. |
| **Telemetry annotations** | `haven telemetry --bug "..."` — local-only log with typed, sequenced IDs. |
| **jj VCS backend** | Use Jujutsu for the haven repo and all managed extdirs. |

## If these gaps are blocking you

- **age/GPG encryption:** Continue using chezmoi for encrypted files while managing the rest with haven. The two tools can coexist.
- **`modify_` scripts:** The typical use case is secret injection — convert to `.tmpl` + `op()` or `get_env()`.
- **Other backends:** If you rely on Bitwarden, LastPass, or Vault, haven isn't ready for full migration yet. Open an issue on GitHub if this is blocking you.

If any of these are blockers, open an issue at [github.com/johnstegeman/haven](https://github.com/johnstegeman/haven).
