# Changelog

All notable changes to haven are documented here.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Removed

- **SkillKit backend** — the `skillkit` backend has been removed. The `native`
  backend remains the only supported backend. Any `ai/config.toml` with
  `backend = "skillkit"` will error with an "unknown skill backend" message;
  remove that line or set `backend = "native"` to continue.

---

## [v0.7.1] — 2026-03-26

### Fixed

- **`hv upgrade` checksum verification** — the upgrade command was requesting
  `SHA256SUMS` but the release workflow publishes `haven-vX.Y.Z-SHA256SUMS`.
  This caused all self-upgrade attempts from v0.7.0 to fail with a 404 error.

---

## [v0.7.0] — 2026-03-26

### Added

- **Conflict detection** — `haven apply --files` now detects when you've edited a
  deployed file since the last apply and asks what to do: `[s]kip`, `[o]verwrite`,
  `[A]pply all`, or `[d]iff` (view a diff before deciding). Haven records a SHA-256
  fingerprint of every file it writes, so it can tell the difference between "I
  updated this in source" and "you edited this live copy". The `--on-conflict=<mode>`
  flag skips the prompt: `skip` (CI-friendly, exits 1 when anything was skipped),
  `overwrite` (always clobber), or `prompt` (default on a TTY).

- **`C` marker in `haven status`** — files you've edited since the last apply now show
  a `C` marker. Combined with the source-drift marker: `MC` means both the source and
  your live copy have diverged. Run `haven status` before `haven apply` to see exactly
  what you've locally modified.

---

## [v0.6.0] — 2026-03-24

### Added

- **Swappable skill backends** — the AI skills pipeline now delegates to a
  configurable backend. Configure via `ai/config.toml`:
  ```toml
  [skills]
  backend = "native"   # default
  ```
  Existing repos need no changes — the `native` backend is the default.

- **`haven ai backends`** — new subcommand that lists all known backends with
  availability status and the currently active backend marked.

- **`docs/reference/skill-backends.md`** — new reference page covering the
  native backend configuration.

---

## [v0.5.5] — 2026-03-24

### Added

- **Brewfile auto-sort** — set `[homebrew] sort = true` in any module config to
  keep the Brewfile sorted alphabetically after every `haven brew install` or
  `haven brew uninstall`. Each kind (`tap`, `brew`, `cask`) is sorted
  independently; blank lines and comments are preserved in place.
- **`haven apply` Brewfile summary** — the apply summary now reports how many
  Brewfiles were run: `Applied N file(s), M Brewfile(s) across K module(s)`.
- **`haven status` improvements** — the profile name is now printed at the top
  of status output. A `✓ Everything up to date` message is shown when no drift
  is found.
- **`haven import` symlink warning** — after importing from chezmoi, a summary
  of all imported `symlink_` entries is printed with a reminder to verify
  symlink targets before running `haven apply`.
- **Documentation site** — full MkDocs documentation at
  [johnstegeman.github.io/haven](https://johnstegeman.github.io/haven), covering
  concepts, task-oriented guides, a reference section, and a dedicated
  "For chezmoi Users" section. Deployed automatically via GitHub Actions.

### Fixed

- **Symlink idempotency** — `haven apply` no longer re-writes a symlink that
  already points to the correct target. Previously it would overwrite and
  count the file even when nothing changed.
- **Dangling symlink backup** — `haven apply` no longer attempts to back up a
  dangling symlink (one whose target has been deleted) before overwriting it.
  `std::fs::copy` follows symlinks and would fail; the dangling link is now
  removed directly.
- **`brew bundle install` output** — spurious `Using <formula>` lines from
  `brew bundle` are now suppressed. Output is streamed in real time instead of
  being buffered until completion.
- **Unused import** — removed unused `serde_yaml` import in `chezmoi.rs`.

---

## [v0.5.0] — 2026-03-23

### Added

- **`haven apply --zap`** — when removing unreferenced casks, also delete their
  associated app data and support files (`brew uninstall --cask --zap`). Implies
  `--remove-unreferenced-brews`.
- **`haven telemetry --action/--bug/--question "<text>"`** — typed telemetry
  annotations with auto-generated, sequenced IDs (`A000001`, `B000001`,
  `Q000001`). The ID is printed after the command so you can reference it in
  follow-up notes. Complements `--note` (which uses `N` prefix).
- **`haven telemetry --list-notes/--list-bugs/--list-actions/--list-questions`**
  — filter the telemetry log to a specific annotation kind.
- **`haven list`** — rewritten to show files, Homebrew packages, and AI skills
  in a unified view. `[files]`, `[brew]`, and `[ai]` section headers appear when
  no filter is active. `--files`, `--brews`, and `--ai` flags scope the output
  to one section. `--profile <p>` scopes to a specific profile.
- **`haven status --brews`** — now shows both missing packages (`?`) and
  packages installed but not in any Brewfile (`+`), giving a unified
  install/extra drift view in one pass.

### Fixed

- **`haven apply` file counter** (B000002) — the `Applied N file(s)` summary
  now counts only files actually written. Previously it counted every entry
  processed, including files silently skipped because the destination was
  already identical to the source.
- **`haven apply --brews`** — now runs `brew bundle install` for the master
  Brewfile and every active module Brewfile. Previously only the master
  `brew/Brewfile` was used, silently ignoring module-level packages.
- **`brew leaves` tap-qualified names** — `haven status --brews` and
  `haven apply --remove-unreferenced-brews` no longer report tap formulae (e.g.
  `qmk/qmk/qmk`) as extra when their short name (`qmk`) is declared in a
  Brewfile. Matching is now done by short name in both directions.
- **`haven brew install`** — when there is no master `brew/Brewfile` but exactly
  one module Brewfile exists, it is used automatically. If multiple module
  Brewfiles exist and `--module` is not given, haven errors with a clear hint.
- **`haven init`** — scaffold always appends a `[profile.default]` section to
  `haven.toml` if one is not present, so bare `haven apply` works out of the
  box.
- **Removed `--no-lock` from `brew bundle install`** — the flag was removed from
  modern Homebrew and caused an error on recent versions.

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
