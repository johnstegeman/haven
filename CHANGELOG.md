# Changelog

All notable changes to haven are documented here.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

---

## [v0.4.0] — 2026-03-23

### Breaking

- **Project renamed from `dfiles` to `haven`** — the binary, config file, state
  directory, and all environment variables have changed:

  | Before | After |
  |--------|-------|
  | `dfiles` (binary) | `haven` |
  | `dfiles.toml` | `haven.toml` |
  | `~/.dfiles/` | `~/.haven/` |
  | `DFILES_DIR` | `HAVEN_DIR` |
  | `DFILES_VCS` | `HAVEN_VCS` |
  | `DFILES_TELEMETRY` | `HAVEN_TELEMETRY` |
  | `DFILES_PROFILE` | `HAVEN_PROFILE` |
  | GitHub repo `johnstegeman/dfiles` | `johnstegeman/haven` |

  The source encoding (`dot_`, `private_`, `.tmpl`, etc.) and repo layout are
  unchanged — existing repos continue to work by moving `dfiles.toml` →
  `haven.toml` and `~/.dfiles/` → `~/.haven/`.

### Added

- **`config/ignore` is now a Tera template** — the ignore file is rendered against
  the current machine context (`os`, `hostname`, `username`, `profile`, `data.*`)
  before patterns are evaluated. This matches how chezmoi treats `.chezmoiignore`:
  conditional patterns like `{% if os == "macos" %}.DS_Store{% endif %}` work as
  expected. If rendering fails, haven warns and falls back to ignoring nothing.
  (`haven import --from chezmoi` now converts Go template syntax in
  `.chezmoiignore` to Tera syntax rather than stripping template lines.)

### Fixed

- **`haven init` in an already-initialised repo** no longer errors — it prints an
  informational message and exits 0. The VCS resolution prompt (Jujutsu vs Git) now
  only runs when a source URL is provided, not during a blank scaffold init.
- **`haven --version`** now shows the full `<semver>+<short-commit>` build identity
  (e.g. `0.4.0+abc1234`), matching the version field written to telemetry events.

---

## [v0.3.0] — 2026-03-23

### Security

- **Fixed HIGH: tarball symlink traversal in `extract_tarball`** — a malicious
  `gh:` skill package could plant a symlink entry pointing outside the cache
  directory, then write a file through it on extraction. Symlink and hard-link
  tar entries are now unconditionally skipped before extraction. Users on v0.2.0
  who fetch skills from untrusted `gh:` sources should upgrade before running
  `haven apply`. See `.gstack/security-reports/2026-03-23.json` for the full
  audit record.

### Added

- **`haven security-scan`** — audit all tracked source files for secrets,
  sensitive filenames, and credential paths. Checks filename patterns (`.env`,
  `id_rsa`, `.pem`…), sensitive paths (`~/.aws/credentials`, `~/.kube/**`,
  `~/.ssh/**`…), and content patterns (GitHub tokens, AWS keys, PEM private
  keys, OpenAI/Anthropic API keys, generic password assignments). Optional
  `--entropy` flag adds high-entropy string detection. Exits 1 when findings
  are found (CI-friendly). False positives suppressed via `[security] allow`
  in `haven.toml`.
- **`haven add` content scan** — when adding a file, its content is
  automatically scanned for secrets. A prompt is shown before saving; declining
  removes the file from `source/` with no partial state left behind.
- **`haven completions <shell>`** — print shell completion scripts to stdout.
  Supports `fish`, `zsh`, and `bash`. All subcommands and flags are included.
- **`haven list`** — list all tracked files with decoded destination paths and
  flag annotations (`template`, `private`, `symlink`, `extdir`, `extfile`, etc.).
- **`haven telemetry`** — manage local telemetry from the CLI. `--enable` /
  `--disable` flip `[telemetry] enabled` in `haven.toml` using surgical
  `toml_edit` (preserves comments and formatting). `--note "<text>"` appends a
  `{"kind":"note"}` entry to `~/.haven/telemetry.jsonl` regardless of whether
  telemetry is enabled — useful for annotating test runs, onboarding sessions,
  or observed issues so the log has context during analysis. Bare `haven
  telemetry` prints the current enabled/disabled status.
- **`haven upgrade`** — self-update command. Downloads the latest release
  tarball from GitHub, verifies the SHA256 checksum, extracts the binary, and
  atomically replaces the running executable. `--check` flag exits 0 when
  up to date, 1 when an update is available (CI-friendly). Supports macOS
  (arm64, x86_64) and Linux (x86_64, aarch64, armv7, i686 musl).
- **Jujutsu (jj) VCS backend** — configure haven to use `jj git clone
  --colocate` for all new clone and init operations. Set via `--vcs jj` flag,
  `HAVEN_VCS=jj` env var, or `[vcs] backend = "jj"` in `haven.toml`. On
  first use without a config, haven detects whether jj is on your PATH and
  prompts once. `haven vcs` shows the active backend and how it was resolved.
- **`haven unmanaged`** — walk `~` and report files not tracked by haven.
  Only dotfiles and dotdirs are examined at the home root. High-noise directories
  (`.cache`, `.cargo`, `node_modules`, `.git`, `Library`, etc.) are skipped.
  `--path <dir>` scans a specific directory; `--depth <n>` controls recursion depth
  (default: 3). Useful for discovering dotfiles to add.
- **`[data]` custom template variables** — define arbitrary string variables
  in `haven.toml` under `[data]` (e.g. `host = "my-laptop"`). Available in all
  `.tmpl` files as `{{ data.host }}`. `haven import --from chezmoi` automatically
  migrates `.chezmoidata.yaml` / `.chezmoidata.toml` into `[data]` entries.
  `haven data` prints all resolved variables.
- **`haven data`** — show all template variables in scope: built-in variables
  (`os`, `hostname`, `username`, `home_dir`, `source_dir`) and custom `[data]`
  entries from `haven.toml`. Useful for debugging templates.
- **`config/ignore` warning on `haven add`** — if a file being added matches
  an ignore pattern, the add is skipped with a clear message explaining how to
  remove the pattern.

### Added (docs)

- **`docs/from-chezmoi.md`** — getting started guide for chezmoi users. Covers
  the automated migration path, template syntax conversion table, command
  equivalence table, Brewfile/module/profile setup, AI skill import, new-machine
  bootstrap, daily workflow comparison, and a gap table of chezmoi features not
  yet in haven.

### Fixed

- Installer URL typo (affected the guide; the binary download URL was correct).
- Pre-existing build warnings: unused import and dead code in internal modules.

---

## [v0.2.0] — 2026-03-21

### Added

- **`haven ai add-local`** — import a locally developed skill directory into
  the haven repo. Moves the skill into `ai/skills/<name>/files/`, writes
  `skill.toml` with `source = "repo:"`, creates a blank snippet stub, and
  removes the original directory. Run `haven apply --ai` afterward to deploy.
- **`extfile_` source encoding** — track single-file and archive remote
  downloads in `source/`. Two types: `type = "file"` (plain download) and
  `type = "archive"` (tarball extract with optional subpath). SHA-256
  verification supported. `haven diff` shows `?` when the destination is
  absent.
- **`haven ai search`** — search the skills.sh registry for available skills.
  Results show source in `gh:owner/repo/skill` format and install count.
- **`haven ai scan`** — scan an existing skills directory for unmanaged skills
  and offer to add them to `ai/skills.toml`. Detects source via git remote or
  skills.sh fuzzy search.
- **Managed config section injection** — `haven apply --ai` injects skill
  snippets from `all.md` / `<platform>.md` into platform config files (e.g.
  `~/.claude/CLAUDE.md`) between HTML comment markers. Idempotent.
- **`haven diff --ai`** — shows stale skill SHA drift and missing deployments.
- **`haven diff` extdir ref drift** — shows when a cloned external repo is
  behind the pinned `ref` in the marker file.
- **`haven.lock` SHA verification** — on cache miss, the freshly-fetched SHA
  is compared against the lock entry. Mismatch is a hard error; run
  `haven ai update` to accept intentional upgrades.
- **Parallel skill fetches** — `haven apply --ai` fetches multiple `gh:`
  skills concurrently using `std::thread::scope`. A single miss fetches
  inline; 2+ prints "Fetching N skills in parallel…" with per-skill results.
- **Apply file lock** — writes PID to `~/.haven/apply.lock` to prevent
  concurrent runs. Stale locks (dead PID) are cleaned up automatically.
- **`haven source-path`** — print the repo directory path. Useful in scripts
  and shell aliases.
- **XDG default repo directory** — new installs default to
  `~/.local/share/haven`. Existing `~/haven` repos are still detected and
  used without migration.
- **`haven add --update`** — re-copy a file into `source/` even if already
  tracked.
- **`home_dir` template variable** — available in `.tmpl` files alongside
  `hostname`, `username`, `os`, etc.
- **`create_` prefix support** — `create_` files are written on apply only if
  the destination does not already exist.
- **`exact_` directory prefix** — on apply, files in the destination that are
  not tracked in `source/` are removed (backed up first).
- **Script execution** — `haven apply --run-scripts` runs scripts from
  `source/scripts/`. `run_once_` / `once_` scripts are tracked in state and
  skipped on subsequent runs.
- **Opt-in local telemetry** — disabled by default. When enabled, events are
  written to `~/.haven/telemetry.jsonl` (command name, flags, duration, exit
  status — no paths or personal data).

### Fixed

- `modify_` scripts in chezmoi import now emit a clear skip message with
  guidance instead of silently disappearing.
- Config structs use forward-compatible deserialization (unknown fields are
  ignored rather than erroring).

---

## [v0.1.0] — 2026-03-20 (initial release)

### Added

- **`haven init`** — create a blank scaffold or clone an existing haven repo.
  Supports `gh:owner/repo[@ref]` shorthand, `--apply`, `--profile`, `--branch`.
- **`haven add`** — track a dotfile by copying it into `source/`. Sensitive
  filename detection prompts before saving. Directory add: recursively adds
  files or tracks the directory as an `extdir_` external git clone.
- **`haven apply`** — copy source files to their destinations, install
  Homebrew packages via `brew bundle`, run mise, deploy AI skills.
  `--dry-run` mode, `--files`/`--brews`/`--ai` section filters,
  `--remove-unreferenced-brews`, `--interactive`, `--apply-externals`,
  `--run-scripts`.
- **`haven diff`** — show file-level diff between `source/` and live files.
  `--stat`, `--color`, `--profile`, `--module` flags. Exit 1 on drift.
- **`haven status`** — concise drift summary with `✓ M ? !` markers.
- **`haven brew install/uninstall`** — run brew operations and keep Brewfiles
  in sync in one step.
- **`haven import --from chezmoi`** — migrate from chezmoi. Handles `dot_`,
  `private_`, `executable_`, `symlink_` prefixes; converts Go templates to
  Tera; imports `.chezmoiexternal.toml` git entries as `extdir_` markers;
  imports `.chezmoiignore` as `config/ignore`.
- **`haven ai add/fetch/update/remove/discover`** — manage AI agent skills
  across platforms. Skills declared in `ai/skills/<name>/skill.toml`, fetched
  from `gh:owner/repo[@ref]`, pinned by SHA in `haven.lock`.
- **Magic-name file encoding** — `dot_`, `private_`, `executable_`,
  `symlink_`, `extdir_`, `.tmpl` suffix. Compatible with chezmoi's encoding.
- **Tera templates** — `.tmpl` files rendered with `hostname`, `username`,
  `os`, `arch`, `env`, `profile`, `source_dir` variables.
- **1Password integration** — `{{ op(path="op://vault/item/field") }}`
  template function reads secrets at apply time.
- **Profiles** — `[profile.<name>]` in `haven.toml` with `modules` list and
  optional `extends` inheritance.
- **Modules** — group Homebrew packages and mise configs. `--module` flag on
  `apply`/`diff` to scope package operations.
- **`extdir_` externals** — track remote git repos as clone markers; cloned on
  `haven apply`.
- **Cross-platform binaries** — macOS (x86_64, arm64) and Linux (x86_64,
  aarch64, i686, armv7 musl).
- **Shell installer** — `curl -fsSL .../install.sh | sh` with SHA256
  verification and automatic PATH detection.
