# Changelog

All notable changes to dfiles are documented here.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased] → v0.3.0

### Security

- **Fixed HIGH: tarball symlink traversal in `extract_tarball`** — a malicious
  `gh:` skill package could plant a symlink entry pointing outside the cache
  directory, then write a file through it on extraction. Symlink and hard-link
  tar entries are now unconditionally skipped before extraction. Users on v0.2.0
  who fetch skills from untrusted `gh:` sources should upgrade before running
  `dfiles apply`. See `.gstack/security-reports/2026-03-23.json` for the full
  audit record.

### Added

- **`dfiles security-scan`** — audit all tracked source files for secrets,
  sensitive filenames, and credential paths. Checks filename patterns (`.env`,
  `id_rsa`, `.pem`…), sensitive paths (`~/.aws/credentials`, `~/.kube/**`,
  `~/.ssh/**`…), and content patterns (GitHub tokens, AWS keys, PEM private
  keys, OpenAI/Anthropic API keys, generic password assignments). Optional
  `--entropy` flag adds high-entropy string detection. Exits 1 when findings
  are found (CI-friendly). False positives suppressed via `[security] allow`
  in `dfiles.toml`.
- **`dfiles add` content scan** — when adding a file, its content is
  automatically scanned for secrets. A prompt is shown before saving; declining
  removes the file from `source/` with no partial state left behind.
- **`dfiles completions <shell>`** — print shell completion scripts to stdout.
  Supports `fish`, `zsh`, and `bash`. All subcommands and flags are included.
- **`dfiles list`** — list all tracked files with decoded destination paths and
  flag annotations (`template`, `private`, `symlink`, `extdir`, `extfile`, etc.).
- **`dfiles telemetry`** — manage local telemetry from the CLI. `--enable` /
  `--disable` flip `[telemetry] enabled` in `dfiles.toml` using surgical
  `toml_edit` (preserves comments and formatting). `--note "<text>"` appends a
  `{"kind":"note"}` entry to `~/.dfiles/telemetry.jsonl` regardless of whether
  telemetry is enabled — useful for annotating test runs, onboarding sessions,
  or observed issues so the log has context during analysis. Bare `dfiles
  telemetry` prints the current enabled/disabled status.
- **`dfiles upgrade`** — self-update command. Downloads the latest release
  tarball from GitHub, verifies the SHA256 checksum, extracts the binary, and
  atomically replaces the running executable. `--check` flag exits 0 when
  up to date, 1 when an update is available (CI-friendly). Supports macOS
  (arm64, x86_64) and Linux (x86_64, aarch64, armv7, i686 musl).
- **Jujutsu (jj) VCS backend** — configure dfiles to use `jj git clone
  --colocate` for all new clone and init operations. Set via `--vcs jj` flag,
  `DFILES_VCS=jj` env var, or `[vcs] backend = "jj"` in `dfiles.toml`. On
  first use without a config, dfiles detects whether jj is on your PATH and
  prompts once. `dfiles vcs` shows the active backend and how it was resolved.
- **`[data]` custom template variables** — define arbitrary string variables
  in `dfiles.toml` under `[data]` (e.g. `host = "my-laptop"`). Available in all
  `.tmpl` files as `{{ data.host }}`. `dfiles import --from chezmoi` automatically
  migrates `.chezmoidata.yaml` / `.chezmoidata.toml` into `[data]` entries.
  `dfiles data` prints all resolved variables.
- **`dfiles data`** — show all template variables in scope: built-in variables
  (`os`, `hostname`, `username`, `home_dir`, `source_dir`) and custom `[data]`
  entries from `dfiles.toml`. Useful for debugging templates.
- **`config/ignore` warning on `dfiles add`** — if a file being added matches
  an ignore pattern, the add is skipped with a clear message explaining how to
  remove the pattern.

### Fixed

- Installer URL typo (affected the guide; the binary download URL was correct).
- Pre-existing build warnings: unused import and dead code in internal modules.

---

## [v0.2.0] — 2026-03-21

### Added

- **`dfiles ai add-local`** — import a locally developed skill directory into
  the dfiles repo. Moves the skill into `ai/skills/<name>/files/`, writes
  `skill.toml` with `source = "repo:"`, creates a blank snippet stub, and
  removes the original directory. Run `dfiles apply --ai` afterward to deploy.
- **`extfile_` source encoding** — track single-file and archive remote
  downloads in `source/`. Two types: `type = "file"` (plain download) and
  `type = "archive"` (tarball extract with optional subpath). SHA-256
  verification supported. `dfiles diff` shows `?` when the destination is
  absent.
- **`dfiles ai search`** — search the skills.sh registry for available skills.
  Results show source in `gh:owner/repo/skill` format and install count.
- **`dfiles ai scan`** — scan an existing skills directory for unmanaged skills
  and offer to add them to `ai/skills.toml`. Detects source via git remote or
  skills.sh fuzzy search.
- **Managed config section injection** — `dfiles apply --ai` injects skill
  snippets from `all.md` / `<platform>.md` into platform config files (e.g.
  `~/.claude/CLAUDE.md`) between HTML comment markers. Idempotent.
- **`dfiles diff --ai`** — shows stale skill SHA drift and missing deployments.
- **`dfiles diff` extdir ref drift** — shows when a cloned external repo is
  behind the pinned `ref` in the marker file.
- **`dfiles.lock` SHA verification** — on cache miss, the freshly-fetched SHA
  is compared against the lock entry. Mismatch is a hard error; run
  `dfiles ai update` to accept intentional upgrades.
- **Parallel skill fetches** — `dfiles apply --ai` fetches multiple `gh:`
  skills concurrently using `std::thread::scope`. A single miss fetches
  inline; 2+ prints "Fetching N skills in parallel…" with per-skill results.
- **Apply file lock** — writes PID to `~/.dfiles/apply.lock` to prevent
  concurrent runs. Stale locks (dead PID) are cleaned up automatically.
- **`dfiles source-path`** — print the repo directory path. Useful in scripts
  and shell aliases.
- **XDG default repo directory** — new installs default to
  `~/.local/share/dfiles`. Existing `~/dfiles` repos are still detected and
  used without migration.
- **`dfiles add --update`** — re-copy a file into `source/` even if already
  tracked.
- **`home_dir` template variable** — available in `.tmpl` files alongside
  `hostname`, `username`, `os`, etc.
- **`create_` prefix support** — `create_` files are written on apply only if
  the destination does not already exist.
- **`exact_` directory prefix** — on apply, files in the destination that are
  not tracked in `source/` are removed (backed up first).
- **Script execution** — `dfiles apply --run-scripts` runs scripts from
  `source/scripts/`. `run_once_` / `once_` scripts are tracked in state and
  skipped on subsequent runs.
- **Opt-in local telemetry** — disabled by default. When enabled, events are
  written to `~/.dfiles/telemetry.jsonl` (command name, flags, duration, exit
  status — no paths or personal data).

### Fixed

- `modify_` scripts in chezmoi import now emit a clear skip message with
  guidance instead of silently disappearing.
- Config structs use forward-compatible deserialization (unknown fields are
  ignored rather than erroring).

---

## [v0.1.0] — 2026-03-20 (initial release)

### Added

- **`dfiles init`** — create a blank scaffold or clone an existing dfiles repo.
  Supports `gh:owner/repo[@ref]` shorthand, `--apply`, `--profile`, `--branch`.
- **`dfiles add`** — track a dotfile by copying it into `source/`. Sensitive
  filename detection prompts before saving. Directory add: recursively adds
  files or tracks the directory as an `extdir_` external git clone.
- **`dfiles apply`** — copy source files to their destinations, install
  Homebrew packages via `brew bundle`, run mise, deploy AI skills.
  `--dry-run` mode, `--files`/`--brews`/`--ai` section filters,
  `--remove-unreferenced-brews`, `--interactive`, `--apply-externals`,
  `--run-scripts`.
- **`dfiles diff`** — show file-level diff between `source/` and live files.
  `--stat`, `--color`, `--profile`, `--module` flags. Exit 1 on drift.
- **`dfiles status`** — concise drift summary with `✓ M ? !` markers.
- **`dfiles brew install/uninstall`** — run brew operations and keep Brewfiles
  in sync in one step.
- **`dfiles import --from chezmoi`** — migrate from chezmoi. Handles `dot_`,
  `private_`, `executable_`, `symlink_` prefixes; converts Go templates to
  Tera; imports `.chezmoiexternal.toml` git entries as `extdir_` markers;
  imports `.chezmoiignore` as `config/ignore`.
- **`dfiles ai add/fetch/update/remove/discover`** — manage AI agent skills
  across platforms. Skills declared in `ai/skills/<name>/skill.toml`, fetched
  from `gh:owner/repo[@ref]`, pinned by SHA in `dfiles.lock`.
- **Magic-name file encoding** — `dot_`, `private_`, `executable_`,
  `symlink_`, `extdir_`, `.tmpl` suffix. Compatible with chezmoi's encoding.
- **Tera templates** — `.tmpl` files rendered with `hostname`, `username`,
  `os`, `arch`, `env`, `profile`, `source_dir` variables.
- **1Password integration** — `{{ op(path="op://vault/item/field") }}`
  template function reads secrets at apply time.
- **Profiles** — `[profile.<name>]` in `dfiles.toml` with `modules` list and
  optional `extends` inheritance.
- **Modules** — group Homebrew packages and mise configs. `--module` flag on
  `apply`/`diff` to scope package operations.
- **`extdir_` externals** — track remote git repos as clone markers; cloned on
  `dfiles apply`.
- **Cross-platform binaries** — macOS (x86_64, arm64) and Linux (x86_64,
  aarch64, i686, armv7 musl).
- **Shell installer** — `curl -fsSL .../install.sh | sh` with SHA256
  verification and automatic PATH detection.
