# chezmoi Feature Gap Analysis

This document maps every significant chezmoi feature against haven's current implementation.
Use it to decide which features are worth building.

**Key:**
- ✅ Already in haven
- 📋 Already tracked in TODOS.md (not yet implemented)
- ⬜ Not in haven, not yet tracked

---

## Source State Attributes

### File prefixes

| Attribute | chezmoi behavior | haven status |
|-----------|-----------------|---------------|
| `dot_` | Renames `.dot_foo` → `.foo` | ✅ Implemented |
| `private_` | chmod 0600 (files), 0700 (dirs) | ✅ Implemented |
| `executable_` | chmod 0755 (or 0700 with private) | ✅ Implemented |
| `symlink_` | Create symlink; file content = link target | ✅ Implemented |
| `readonly_` | Remove all write permissions (0444 files, 0555 dirs) | ⬜ Not tracked |
| `empty_` | Ensure file exists even when empty (chezmoi removes empty files by default) | ⬜ Not tracked |
| `remove_` | Remove the file/symlink/dir from the target during apply | ⬜ Not tracked |
| `encrypted_` | File is encrypted in the repo; decrypted on apply | ⬜ Not tracked |
| `create_` | Write only if dest doesn't exist (skip if present) | 📋 TODOS.md P1 |
| `exact_` | Delete anything in dest dir not tracked by haven | 📋 TODOS.md P1 |
| `modify_` | Script whose stdout replaces dest file (stdin = current content) | 📋 TODOS.md P2 (note only) |
| `run_` | Execute as a script on every apply | 📋 TODOS.md P1 |
| `run_once_` | Execute once per machine (tracked by content hash) | 📋 TODOS.md P1 |
| `run_onchange_` | Execute when content changes (tracked by hash per filename) | ⬜ Not tracked |
| `before_` | Run script before file updates | ⬜ Not tracked |
| `after_` | Run script after file updates | ⬜ Not tracked |
| `literal_` | Stop parsing all prefix attributes at this point | ⬜ Not tracked |
| `external_` | Mark dir as external (don't recurse for attribute parsing) | ⬜ Not tracked |

### File suffixes

| Attribute | chezmoi behavior | haven status |
|-----------|-----------------|---------------|
| `.tmpl` | Render as Go template (haven uses Tera) | ✅ Implemented |
| `.literal` | Stop parsing suffix attributes | ⬜ Not tracked |
| `.age` | Strip when age encryption is active | ⬜ (part of encryption feature) |
| `.asc` | Strip when GPG encryption is active | ⬜ (part of encryption feature) |

### Prefix combinations for scripts

chezmoi supports stacking `run_`, `once_`/`onchange_`, and `before_`/`after_`:
- `run_once_before_script.sh` — run once, before file updates
- `run_once_after_script.sh` — run once, after file updates
- `run_onchange_before_script.sh` — run on change, before file updates
- `run_onchange_after_script.sh` — run on change, after file updates

haven status: ⬜ Not tracked (depends on script execution feature in TODOS.md P1)

---

## Template System

### Template variables

| Variable | chezmoi | haven |
|----------|---------|--------|
| OS name | `.chezmoi.os` (`darwin`, `linux`, …) | ✅ `{{ os }}` |
| Short hostname | `.chezmoi.hostname` | ✅ `{{ hostname }}` |
| Username | `.chezmoi.username` | ✅ `{{ username }}` |
| Home directory | `.chezmoi.homeDir` | ✅ via `get_env(name="HOME")` |
| CPU architecture | `.chezmoi.arch` (`amd64`, `arm64`, …) | ⬜ Not tracked |
| FQDN hostname | `.chezmoi.fqdnHostname` | ⬜ Not tracked |
| User ID | `.chezmoi.uid` | ⬜ Not tracked |
| Group ID | `.chezmoi.gid` | ⬜ Not tracked |
| Group name | `.chezmoi.group` | ⬜ Not tracked |
| Destination dir | `.chezmoi.destDir` | ⬜ Not tracked |
| Source dir | `.chezmoi.sourceDir` | ✅ `{{ source_dir }}` |
| OS release info | `.chezmoi.osRelease.*` (Linux `/etc/os-release`) | ⬜ Not tracked |
| Kernel info | `.chezmoi.kernel.*` (Linux `/proc/sys/kernel`) | ⬜ Not tracked (Linux-only) |
| Windows version | `.chezmoi.windowsVersion.*` | ⬜ Not tracked (Windows-only) |
| Path separator | `.chezmoi.pathSeparator` | ⬜ Not tracked |
| chezmoi version | `.chezmoi.version.version` | ⬜ Not tracked |
| Current target file | `.chezmoi.targetFile` | ⬜ Not tracked |
| stdin (modify_ scripts) | `.chezmoi.stdin` | ⬜ (part of modify_ feature) |
| Active profile | — | ✅ `{{ profile }}` (haven-specific) |
| Env variable | — | ✅ `{{ get_env(name="X") }}` |
| 1Password secret | — | ✅ `{{ op(path="...") }}` |

### Template functions (chezmoi-specific, beyond Tera builtins)

| Function | Purpose | haven status |
|----------|---------|---------------|
| `exec cmd args...` / `output` | Run an external command; return stdout | ⬜ Not tracked |
| `glob pattern` | Glob filesystem paths | ⬜ Not tracked |
| `stat path` / `lstat path` | Stat a file path | ⬜ Not tracked |
| `lookPath name` | Find executable in `$PATH` | ⬜ Not tracked |
| `findExecutable` | Find executable in given dirs | ⬜ Not tracked |
| `fromYaml`/`toYaml` | Parse/serialize YAML | ⬜ Not tracked |
| `fromJson`/`toJson`/`toPrettyJson` | Parse/serialize JSON | ⬜ Not tracked |
| `fromToml`/`toToml` | Parse/serialize TOML | ⬜ Not tracked |
| `fromIni`/`toIni` | Parse/serialize INI | ⬜ Not tracked |
| `jq query value` | Apply jq query to data | ⬜ Not tracked |
| `gitHubKeys username` | Get GitHub user's public SSH keys | ⬜ Not tracked |
| `gitHubLatestRelease owner/repo` | Latest GitHub release info | ⬜ Not tracked |
| `gitHubLatestReleaseAssetURL` | URL of latest release asset | ⬜ Not tracked |
| `gitHubLatestTag owner/repo` | Latest tag string | ⬜ Not tracked |
| `comment prefix text` | Prefix each line with a comment char | ⬜ Not tracked |
| `include path` | Include file contents (relative to source dir) | ⬜ Not tracked |
| `includeTemplate name data` | Include a named shared template | ⬜ (part of templates dir feature) |
| `encrypt`/`decrypt` | Encrypt/decrypt in templates | ⬜ (part of encryption feature) |
| `ioreg key` | Read macOS ioreg values | ⬜ Not tracked |
| `warnf format args...` | Print warning without failing | ⬜ Not tracked |
| Sprig library | 100+ additional Go utility functions (string, math, date, etc.) | ⬜ Not tracked (Tera has its own set) |
| `promptStringOnce` etc. | Prompt user at init time; cache answer | ⬜ Not tracked |

### Template directives (in-file control comments)

chezmoi supports special comments inside `.tmpl` files to control rendering behavior:

| Directive | Effect | haven status |
|-----------|--------|---------------|
| `chezmoi:template:left-delimiter=X right-delimiter=Y` | Change `{{ }}` to custom delimiters | ⬜ Not tracked |
| `chezmoi:template:encoding=utf-8-bom` etc. | Control output encoding | ⬜ Not tracked |
| `chezmoi:template:line-endings=crlf\|lf\|native` | Control line endings | ⬜ Not tracked |
| `chezmoi:template:missing-key=zero\|error` | Behavior on undefined vars | ⬜ Not tracked |

---

## Special Files and Directories

| File/Dir | chezmoi purpose | haven status |
|----------|----------------|---------------|
| `.chezmoiexternal.toml` | Define external git repos, archives, files | ✅ Parsed (git-repo type only) |
| `.chezmoiignore` | gitignore-style patterns to skip during apply | ⬜ Not tracked |
| `.chezmoiremove` | List of target paths to delete during apply | ⬜ Not tracked |
| `.chezmoidata.$FORMAT` | Structured data (JSON/TOML/YAML) merged into template vars | ⬜ Not tracked |
| `.chezmoidata/` dir | Directory of data files (same as above) | ⬜ Not tracked |
| `.chezmoitemplates/` dir | Shared templates accessible via `includeTemplate` | ⬜ Not tracked |
| `.chezmoiscripts/` dir | Scripts that run without creating a target directory | ⬜ Not tracked |
| `.chezmoiroot` | Redirect source root to a subdirectory of the repo | ⬜ Not tracked |
| `.chezmoiversion` | Minimum required chezmoi version; refuses if older | ⬜ Not tracked |
| `.chezmoi.$FORMAT.tmpl` | Config template: generates config file during `chezmoi init` | ⬜ Not tracked |

---

## External Sources

haven `extdir_` marker files currently support only `type = "git"`. chezmoi's `.chezmoiexternal` supports:

| External type | chezmoi behavior | haven status |
|--------------|-----------------|---------------|
| `git-repo` | Clone/pull a git repository | ✅ Implemented (`extdir_` marker files) |
| `file` | Download a single file from a URL | ⬜ Not tracked |
| `archive` | Download + extract a tarball/zip into a directory | ⬜ Not tracked |
| `archive-file` | Extract a single file from within an archive | ⬜ Not tracked |

### Common external options not yet in haven

| Option | Purpose | haven status |
|--------|---------|---------------|
| `urls: []` | Fallback URL list tried in order | ⬜ Not tracked |
| `refreshPeriod` | Re-download periodically (e.g., `"168h"` = weekly) | ⬜ Not tracked |
| `checksum.sha256` etc. | Verify download integrity | ⬜ Not tracked (related to haven.lock SHA verification in TODOS.md) |
| `filter.command` | Pipe downloaded content through external command | ⬜ Not tracked |
| `stripComponents` | Strip N leading path components from archive | ⬜ Not tracked |
| `include`/`exclude` | Glob filter for archive members | ⬜ Not tracked |

---

## Encryption

chezmoi supports transparent encryption of source files with `encrypted_` prefix:

| Feature | chezmoi | haven status |
|---------|---------|---------------|
| age encryption | `encrypted_` prefix + `encryption = "age"` in config | ⬜ Not tracked |
| GPG encryption | `encrypted_` prefix + `encryption = "gpg"` in config | ⬜ Not tracked |
| git-crypt / transcrypt | Transparent encryption at the git layer | ⬜ Not tracked |
| `encrypt`/`decrypt` in templates | Encrypt/decrypt values inline in templates | ⬜ Not tracked |

---

## Secret Manager Integrations

haven supports 1Password via `{{ op(path="...") }}`. chezmoi supports many more:

| Manager | haven status |
|---------|---------------|
| 1Password | ✅ Implemented (`op()` function) |
| Bitwarden / rbw | ⬜ Not tracked |
| HashiCorp Vault | ⬜ Not tracked |
| AWS Secrets Manager | ⬜ Not tracked |
| Azure Key Vault | ⬜ Not tracked |
| macOS Keychain / Windows Credentials | ⬜ Not tracked |
| pass / gopass | ⬜ Not tracked |
| KeePassXC | ⬜ Not tracked |
| LastPass | ⬜ Not tracked |
| Dashlane | ⬜ Not tracked |
| Doppler | ⬜ Not tracked |
| Proton Pass | ⬜ Not tracked |
| ejson (encrypted JSON files) | ⬜ Not tracked |
| Generic/custom (`secret` backend) | ⬜ Not tracked |

---

## Commands

| Command | chezmoi | haven status |
|---------|---------|---------------|
| `init` | Initialize repo, generate config via template | ✅ Implemented |
| `add` | Track a file or directory (copy into source with magic-name encoding; directories prompt for extdir or recursive) | ✅ Implemented |
| `apply` | Apply source state to target | ✅ Implemented |
| `status` | Show drift between source and target | ✅ Implemented |
| `import --from chezmoi` | Migrate from chezmoi | ✅ Implemented |
| `brew install/uninstall` | Manage Homebrew + keep Brewfile in sync | ✅ Implemented |
| `bootstrap` | Bootstrap from local or remote package | ✅ Implemented |
| `diff` | Show diff between source state and live files | ✅ Implemented |
| `edit` | Edit a source file (handles .tmpl and encryption transparently) | ⬜ Not tracked |
| `re-add` | Copy a modified live file back into source (reverse of apply) | ⬜ Not tracked |
| `forget` / `unmanage` | Stop tracking a file (remove from source, leave target alone) | ⬜ Not tracked |
| `managed` | List all files currently managed by haven | ⬜ Not tracked |
| `chattr` | Change attributes of a source file (rename with new prefixes) | ⬜ Not tracked |
| `doctor` | Check environment (installed tools, permissions, integrations) | ⬜ Not tracked |
| `verify` | Assert target state exactly matches source state | ⬜ Not tracked |
| `update` | Pull remote changes and apply (git pull + haven apply) | ⬜ Not tracked |
| `data` | Print all template variables as JSON (debugging) | ⬜ Not tracked |
| `completions` | Generate shell completion scripts | ⬜ Not tracked |
| `merge` / `merge-all` | Three-way merge source and target | ⬜ Not tracked |
| `archive` | Create an archive of the rendered target state | ⬜ Not tracked |
| `cd` | Start a shell in the source directory | ⬜ Not tracked |
| `git` | Run git in the source directory | ⬜ Not tracked |
| `purge` | Remove haven and all its data | ⬜ Not tracked |
| `state` | Inspect/reset the persistent state database | ⬜ Not tracked |

---

## Configuration and Behavior

| Feature | chezmoi | haven status |
|---------|---------|---------------|
| `mode = "symlink"` | Manage all files as symlinks pointing into source (instead of copying) | ⬜ Not tracked |
| Auto-commit | Automatically `git commit` after apply | ⬜ Not tracked |
| Auto-push | Automatically `git push` after apply | ⬜ Not tracked |
| Hooks | Pre/post hooks for any command | ⬜ Not tracked |
| `--one-shot` | Apply then delete all traces (for ephemeral environments) | ⬜ Not tracked |
| `--interactive` | Prompt before each change | ⬜ Not tracked |
| `--refresh-externals` | Force re-download of all externals | ⬜ Not tracked |
| Diff tool | Configurable external diff tool | ⬜ Not tracked |
| Merge tool | Configurable three-way merge tool | ⬜ Not tracked |
| Script interpreters | Configurable per-extension interpreter (`[interpreters.py]`) | ⬜ Not tracked |
| Script environment | Extra env vars injected into scripts (`[scriptEnv]`) | ⬜ Not tracked |
| `update --init` | Pull remote changes + re-run init template | ⬜ Not tracked |
| Plugin system | `haven-foo` binaries in PATH are auto-discovered as subcommands | ⬜ Not tracked |
| `haven.boltdb` state | chezmoi uses a BoltDB for run_once_ tracking (haven uses state.json) | ✅ state.json is haven's equivalent |

---

## Features haven Has That chezmoi Doesn't

For completeness — features that are haven-specific and have no chezmoi equivalent:

| Feature | Description |
|---------|-------------|
| Module system | Namespaced Homebrew/mise/AI/externals config per module |
| Profile system | Named sets of modules with inheritance (`extends`) |
| AI skills/commands | First-class `[ai]` section for Claude Code skills and commands |
| `haven-manifest.json` | Package manifest for `haven bootstrap gh:owner/repo` |
| `haven.lock` | SHA pinning for fetched GitHub sources |
| `gh:owner/repo@ref` source format | Shorthand for GitHub sources in AI and externals |
| `haven brew install` | `brew install` + auto-update Brewfile in one command |
| `--dest` staging flag | Apply to a staging directory for testing without touching real home |
| Auto-generated CLAUDE.md | Regenerates `~/.claude/CLAUDE.md` listing installed skills after every apply |

---

## Prioritization Notes

Features most likely worth implementing (high signal-to-noise for haven users):

1. **`haven diff`** — the most-missed command after `apply`/`status`. Shows what would change.
2. **`haven edit`** — second most-missed. Edit source files without manually navigating to `~/haven/source/`.
3. **`haven managed`** — list what's being tracked. Useful for auditing.
4. **`haven forget`** — stop tracking a file without deleting it. Pair with `managed`.
5. **`haven re-add`** — copy a modified live file back into source (the reverse of apply).
6. **`arch` template variable** — commonly needed for OS-conditional tool install paths.
7. **`haven update`** — `git pull` + `haven apply` in one command. Near-essential for daily use.
8. **`.havenignore`** — ignore patterns for source files. Useful for excluding machine-specific files.
9. **`readonly_` prefix** — simple to implement; occasionally needed.
10. **`empty_` prefix** — simple to implement; useful for placeholder files.
11. **`remove_` prefix** — declaratively remove files during apply.
12. **`refreshPeriod` for externals** — auto-update external sources periodically.
13. **`file` and `archive` external types** — download single files or archives from URLs (very useful for binary tools).
14. **Additional secret managers** — Bitwarden and Vault are the most commonly used after 1Password.
15. **Shell completions** — quality-of-life; `haven completions --shell zsh`.

Features likely **not** worth implementing (too complex or mismatched to haven's design):

- **Encryption** — haven's design uses 1Password for secrets; `encrypted_` adds major complexity for marginal gain.
- **`modify_` scripts** — fundamentally incompatible with static file management; already documented as note-only.
- **`mode = "symlink"`** — haven supports explicit `symlink_` prefix; a global symlink mode adds complexity.
- **Merge tool** — haven tracks source as truth; conflicts don't arise in the same way.
- **`--one-shot` mode** — rarely needed; can be approximated by running haven and then deleting the repo.
- **Plugin system** — premature generalization; add commands directly.
- **`purge`** — destructive; not obviously useful in a dotfiles manager.
- **Sprig library** — Tera already provides a rich set of filters; full Sprig parity not needed.
- **Template directives** (delimiter changes, encoding) — edge cases for exotic file formats.
- **chezmoi init config template** (`.chezmoi.$FORMAT.tmpl`) — haven's `haven init` is simpler by design.
