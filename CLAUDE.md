# Haven — Claude Code Orientation

## What Haven Is

Haven is a declarative developer environment manager written in Rust. It tracks and syncs dotfiles (with templating), Homebrew packages, mise runtimes, Claude Code skills, secrets, and external git repos — all from one versioned repository.

The vision: npm for AI developer environments.

## Repository Layout

```
haven.toml              # Root config: profiles, data variables, VCS, security
haven.local.toml        # Gitignored per-machine data overrides (merged over haven.toml)
haven.lock              # Auto-generated SHA pins for all fetched external sources
modules/<name>.toml     # Per-module: Brewfile path, mise config path
source/                 # Dotfiles, encoded via magic-name prefixes
brew/                   # Brewfiles referenced by module configs
mise/                   # Mise config files referenced by module configs (mise/mise.<module>.toml)
ai/
  platforms.toml        # Active platforms + per-platform overrides
  skills/<name>/
    skill.toml          # Skill declaration: source, platforms, deploy method
    all.md              # Snippet injected into every platform's config
    <platform>.md       # Platform-specific snippet
    files/              # Skill content (for repo: source type)
src/                    # Rust source (see below)
tests/integration.rs    # All integration tests
docs/                   # User-facing docs (mkdocs)
```

State that lives outside the repo (never committed):
- `~/.local/state/haven/state.json` — last apply, profile, applied file SHAs, deployed skills, run_once_ history
- `~/.local/state/haven/apply.lock` — process coordination lock
- `~/.local/state/haven/backups/` — backups of overwritten files
- `~/.cache/haven/` — skill fetch cache

## Source Layout (`src/`)

```
main.rs                     # CLI entry point (clap); all subcommand definitions
commands/
  apply.rs                  # Core pipeline — see below
  init.rs, add.rs, remove.rs, status.rs, diff.rs, list.rs
  brew.rs, pkg.rs, ai.rs, import.rs, security_scan.rs, unmanaged.rs, upgrade.rs
config/
  haven.rs                  # HavenConfig: haven.toml + haven.local.toml merge
  module.rs                 # ModuleConfig: modules/<name>.toml
source.rs                   # Magic-name decode: SourceEntry, FileFlags
template.rs                 # Tera engine: TemplateContext, variable injection
state.rs                    # State struct, state.json read/write
lock.rs                     # haven.lock: source + skill SHA pins
fs.rs                       # File I/O helpers: copy, symlink, sha256, permissions
homebrew.rs                 # brew bundle execution
mise.rs                     # mise runtime management
onepassword.rs              # op() Tera function
github.rs                   # GitHub source fetch (tarball, sparse checkout)
ignore.rs                   # config/ignore pattern matching
vcs.rs                      # git vs jj resolution
ai_skill.rs                 # SkillDeclaration, SkillSource parsing
ai_platform.rs              # Platform registry (3-layer: embedded → machine → repo)
ai_config.rs                # ai/config.toml
skill_backend.rs            # SkillBackend trait
skill_backend_native.rs     # Default: GitHub sparse checkout + SHA verify
skill_backend_agentskills.rs
skill_backend_factory.rs
skill_cache.rs              # ~/.cache/haven/
claude_md.rs                # Generates ~/.claude/CLAUDE.md
config_injection.rs         # Injects skill snippets into platform configs
drift.rs                    # Conflict detection logic
chezmoi.rs / chezmoi_template.rs  # Chezmoi import + Go→Tera template conversion
telemetry.rs                # Local opt-in telemetry
diff_util.rs, util.rs
```

## The Apply Pipeline (`commands/apply.rs`)

```
haven apply
  1. Acquire apply.lock (prevent concurrent runs)
  2. Load HavenConfig (haven.toml + haven.local.toml [data] merge)
  3. Build TemplateContext (os, hostname, username, profile, home_dir, source_dir, data.*)

  ── FILES ──────────────────────────────────────────────
  4. Scan source/ → decode magic names → build SourceEntry list
  5. Load state.json for conflict detection (live SHA vs state SHA)
  6. For each entry (skipping ignored paths):
       a. Render if .tmpl (Tera with TemplateContext)
       b. Create symlink (symlink_), clone repo (extdir_), download (extfile_)
       c. Copy with permissions (private_=0600, executable_=0755)
       d. Skip if dest exists (create_)
       e. On conflict: prompt / skip / overwrite per --on-conflict flag
  7. Enforce exact_ dirs (remove untracked files in dest, back up first)
  8. Save applied_files SHAs to state.json

  ── MODULES ────────────────────────────────────────────
  9. Resolve profile → collect modules (flattening extends)
  10. Sort by MODULE_ORDER: shell → git → packages → secrets → ai
  11. For each module: check op guard → run mise install → run brew bundle

  ── AI SKILLS ──────────────────────────────────────────
  12. Load ai/skills/*/skill.toml declarations
  13. For each skill: fetch → verify SHA vs haven.lock → deploy (symlink/copy)
  14. Update haven.lock; hard error on SHA mismatch (require `haven ai update`)
  15. Inject all.md / platform.md snippets into platform config files (idempotent)

  ── SCRIPTS ────────────────────────────────────────────
  16. Run run_once_ scripts (tracked in state.json; skip if already run)
  17. Run run_ scripts every apply

  ── FINISH ─────────────────────────────────────────────
  18. Generate ~/.claude/CLAUDE.md (skills + commands list)
  19. Save full state.json
  20. Print summary
```

## Magic-Name Encoding

Source filenames encode all metadata. Prefixes are processed left-to-right:

| Prefix/Suffix | Effect |
|---|---|
| `dot_` | Destination gets `.` prefix |
| `private_` | chmod 0600 (file) / 0700 (dir) |
| `executable_` | chmod 0755 |
| `symlink_` | Create symlink, not copy |
| `extdir_` | Git clone remote repo into dest (file contains URL + ref) |
| `extfile_` | Download file/archive into dest |
| `create_` | Skip if dest already exists |
| `exact_` | Remove untracked files in dest dir |
| `.tmpl` suffix | Render with Tera before writing |

Examples: `dot_zshrc` → `~/.zshrc`, `private_dot_ssh/config.tmpl` → `~/.ssh/config` (0600, templated), `executable_dot_local/bin/foo` → `~/.local/bin/foo` (0755).

Format is chezmoi-compatible.

## Templates (Tera / Jinja2-style)

Variables always available in `.tmpl` files:

| Variable | Value |
|---|---|
| `os` | `"macos"` or `"linux"` |
| `hostname` | Machine hostname |
| `username` | `$USER` |
| `profile` | Active haven profile |
| `home_dir` | `/Users/you` |
| `source_dir` | Haven repo root |
| `get_env(name="X", default="y")` | Read env var |
| `op(path="vault/item/field")` | 1Password secret (lazy) |
| `data.*` | Custom vars from `[data]` in haven.toml / haven.local.toml |

Autoescape is disabled (dotfiles contain shell syntax).

## Key Config Structures

**`haven.toml`** — shared, committed
```toml
[profile.work]
extends = "default"
modules = ["secrets"]

[data]
work_email = "alice@corp.com"    # {{ data.work_email }} in templates

[packages]
backends = ["brew", "mise"]      # first entry is the default; omit for ["brew","mise"]

[vcs]
backend = "jj"
```

**`haven.local.toml`** — gitignored, per-machine
```toml
[data]                           # merged over haven.toml [data]; local keys win
machine_name = "air-m3"
```

**`modules/<name>.toml`**
```toml
[homebrew]
brewfile = "brew/Brewfile.shell"

[mise]
config = "source/mise.toml"

requires_op = false              # if true, skip brew/mise when op is unavailable
```

**`ai/skills/<name>/skill.toml`**
```toml
source    = "gh:anthropics/skills/pdf-processing@v1.0"
platforms = "all"                # or "cross-client" or ["claude-code"]
deploy    = "symlink"            # or "copy"
```

## Package Management (`haven pkg`)

`haven pkg` is the unified package command. It replaces the former `haven brew`.

```
haven pkg install <name> [--cask] [--module <m>] [--brew] [--mise]
haven pkg uninstall <name> [--cask] [--brew] [--mise]
```

Backend resolution order: explicit `--brew`/`--mise` flag → `[packages] backends[0]` in haven.toml → `"brew"` (hard default).

`--cask` always implies the brew backend. Forcing a backend not listed in `allowed_backends()` is an error. The mise backend writes to `mise/mise[.<module>].toml`; supports `name@version` tool-spec syntax (bare name defaults to `"latest"`). When both backends are configured and no flag is passed, `uninstall` removes from both.

## Key Data Structures

| Struct | File | What it holds |
|---|---|---|
| `HavenConfig` | `config/haven.rs` | Parsed haven.toml + local merge |
| `ModuleConfig` | `config/module.rs` | Parsed modules/<name>.toml |
| `TemplateContext` | `template.rs` | Variables for Tera render |
| `SourceEntry` | `source.rs` | Decoded file: dest path + FileFlags |
| `State` | `state.rs` | state.json: last_apply, applied file SHAs, skill deployment records, scripts_run |
| `LockFile` | `lock.rs` | haven.lock: source + skill SHA pins |
| `SkillDeclaration` | `ai_skill.rs` | One skill from ai/skills/*/skill.toml |
| `PlatformPlugin` | `ai_platform.rs` | Resolved platform (skills_dir, config_file, binary) |

## Profiles

Profiles control which modules are active. Files are always applied in full (not per-module).

```toml
[profile.default]
modules = ["shell", "git", "packages"]

[profile.work]
extends = "default"      # inherits default's modules
modules = ["secrets"]    # appended; result: shell + git + packages + secrets
```

Profile stored in `state.json` after first apply. Override with `--profile`.

## AI Skill System

Platform registry is three layers (first match wins, but repo layer can add active platforms):
1. **Embedded** — `src/data/platforms.toml` (shipped defaults)
2. **Machine-local** — `~/.local/state/haven/platforms.toml`
3. **Repo** — `ai/platforms.toml`

Skill backends are pluggable (trait `SkillBackend`). Default: `NativeBackend` (GitHub sparse checkout + SHA verify). Supply chain protection: SHA mismatch on fetch = hard error; user must run `haven ai update` to accept changes.

## Tests

All integration tests live in `tests/integration.rs`. They use:
- `assert_cmd` — invoke haven binary with assertions
- `tempfile::TempDir` — isolated temp repos and home dirs
- `predicates` — stdout/stderr matching

Helper functions:
- `cmd(repo)` — haven command with `--dir` set, HAVEN_DIR unset
- `cmd_home(repo, home)` — also pins HOME so `~` expands correctly
- `make_local_git_repo(files)` — create a test git remote

Tests cover: init, add, apply, status, templates, magic names, profiles, brew/mise, AI skills, 1Password, chezmoi import, conflict detection, security scan.

## VCS

Haven works with both git and jj (Jujutsu colocated). Resolution order:
1. `--vcs` flag
2. `HAVEN_VCS` env var
3. `haven.toml [vcs] backend`
4. Interactive prompt (if jj detected)
5. Default: git

The repo itself uses jj. Always use `jj` commands for VCS operations in this project.

## Common Patterns When Working on Haven

- **Adding a new template variable**: add field to `TemplateContext` in `template.rs`, populate in `from_env()`, document in `docs/reference/template-variables.md`
- **Adding a new magic-name prefix**: decode in `source.rs` (`decode_magic_name`), add field to `FileFlags`, handle in apply pipeline in `commands/apply.rs`
- **Adding a new command**: add variant to clap enum in `main.rs`, create `commands/<name>.rs`, wire up dispatch
- **Adding config to haven.toml**: add field to `HavenConfig` in `config/haven.rs` (serde default so existing files parse fine)
- **Adding a new platform**: add entry to `src/data/platforms.toml`
