# Design: `dfiles diff`

**Status:** Reviewed (plan-eng-review 2026-03-20)
**Date:** 2026-03-20
**Author:** jstegeman

---

## Problem

`dfiles status` tells you *that* drift exists (modified, missing, source-missing) but not
*what changed*. To investigate drift today you must open the source file in `~/dfiles/source/`
and the live destination file side-by-side manually. For template files this is especially
painful — you have to mentally render the template to understand what the destination
*should* look like.

`dfiles diff` closes this gap: it shows the line-level delta between what dfiles would apply
and what is currently on disk.

---

## Goals

1. Show a unified diff for each source file that differs from its destination.
2. Render templates before diffing — compare the rendered output against the live file.
3. Show which brew packages are declared but not installed, and which are installed but not
   in any Brewfile (mirroring `brew bundle check`'s output, but more granular).
4. Show which AI skills and commands are declared but missing on disk.
5. Accept `--files`, `--brews`, `--ai` section flags (same semantics as `apply`).
6. Accept `--profile` flag (same resolution: explicit → state.json → "default").
7. Accept `--module` flag for scoping to a single module's brew/AI config.
8. Exit 0 when everything is clean, exit 1 when drift exists (script-friendly).
9. No side effects — diff is always read-only.
10. Color output: ANSI colors when stdout is a tty (`+` green, `-` red, `@@` cyan).
11. `--stat` flag: summary mode showing only filenames and change counts.

---

## Non-Goals

- Applying or reversing changes. Use `dfiles apply` / `dfiles re-add` (future).
- Three-way merge. Use an external merge tool.
- Binary file diffing. Binary files get a "binary files differ" notice only.
- Configurable external diff tool (nice-to-have, deferred — see TODOS).
- Cross-machine diff (`dfiles diff --machine <name>`) — P3, see TODOS.md.
- AI version drift (pinned SHA vs installed SHA) — blocked on SHA verification TODO.

---

## Output Design

### Files section

```
[files]

  --- ~/.gitconfig
  +++ ~/.gitconfig  (source: dot_gitconfig.tmpl → rendered)
  @@ -3,7 +3,7 @@
     [user]
  -      name = Old Name
  +      name = John Stegeman
         email = john@example.com

  ? ~/.ssh/config   (missing — never applied)
  ! source/dot_vimrc  (source missing)

```

Markers:
- Unified diff block → file is **Modified** (content differs)
- `?` line → file is **Missing** (dest doesn't exist)
- `!` line → **SourceMissing** (source file removed from repo but was tracked)
- No entry → file is clean

Symlinks: instead of a diff, show:
```
  M ~/.config/nvim  (symlink: points to /wrong/path, expected /Users/jstegeman/dfiles/source/symlink_dot_config/nvim)
```

Binary files: instead of a diff, show:
```
  M ~/.config/some-app/prefs.bin  (binary files differ)
```

### Brew section

```
[brew]

  brew/Brewfile
  + ripgrep     (in Brewfile, not installed)
  + bat         (in Brewfile, not installed)
  - htop        (installed, not in Brewfile)

```

The `+` lines are packages that `dfiles apply --brews` would install.
The `-` lines are packages that `dfiles apply --brews --remove-unreferenced-brews` would remove.

When the master Brewfile is clean: no section printed (or `✓ brew/Brewfile clean` if
only brew section was requested).

### AI section

```
[ai]

  [shell]
  - fetch skill: gh:anthropics/claude-code-gstack   (not installed)
  - fetch command: gh:jstegeman/dfiles-commands       (not installed)

```

AI drift is currently binary (present/absent) — no version comparison is designed for v1.
Version drift is deferred to a future TODO.

### Clean output

```
✓ Everything up to date (profile: default)
```

(Same phrasing as `dfiles status`, consistent UX.)

---

## Architecture

### Data flow

```
dfiles diff
     │
     ├── [files]   (only if diff_files — source::scan() is inside this gate)
     │     │
     │     ├── source::scan(source/)             ← existing
     │     │         │
     │     │         ▼
     │     │   for each SourceEntry:
     │     │     ├── drift::check_drift_*()       ← new shared module
     │     │     │       DriftKind: Clean/Modified/Missing/SourceMissing
     │     │     │
     │     │     └── if Modified:
     │     │           ├── template? → render() → diff_util::unified_diff()
     │     │           ├── symlink?  → show target mismatch
     │     │           ├── binary?   → "binary files differ"
     │     │           └── plain?    → diff_util::unified_diff()
     │
     ├── [brew]    (only if diff_brews)
     │     │
     │     └── homebrew::brewfile_diff()          ← new (replaces inline logic
     │               BrewfileDiff {                    in purge_unreferenced_brews)
     │                 missing_formulas/casks,
     │                 extra_formulas/casks
     │               }
     │
     └── [ai]      (only if diff_ai)
           │
           └── same logic as status.rs AI check   ← reused pattern
```

### New module: `src/drift.rs`

Extracted from `status.rs`. Makes drift detection available to both `status` and `diff`:

```rust
pub enum DriftKind { Clean, Modified, Missing, SourceMissing }

pub fn check_drift(src: &Path, dest: &Path) -> DriftKind
pub fn check_drift_template(src: &Path, ctx: &TemplateContext, dest: &Path) -> Result<DriftKind>
pub fn check_drift_link(source_abs: &Path, dest: &Path) -> DriftKind
pub fn drift_marker(kind: DriftKind) -> &'static str   // "M" / "?" / "!"
```

`status.rs` is refactored to import from `drift.rs`. No behavior change.

### New module: `src/commands/diff.rs`  (after drift.rs extraction)

Primary implementation. Mirrors the structure of `apply.rs`/`status.rs`:

```rust
pub struct DiffOptions<'a> {
    pub repo_root: &'a Path,
    pub dest_root: &'a Path,
    pub state_dir: &'a Path,
    pub claude_dir: &'a Path,
    pub profile: &'a str,
    pub module_filter: Option<&'a str>,
    pub diff_files: bool,
    pub diff_brews: bool,
    pub diff_ai: bool,
    pub stat_only: bool,   // --stat: show summary, not diff content
    pub color: ColorMode,  // Always | Never | Auto
}

pub enum ColorMode { Always, Never, Auto }

pub fn run(opts: &DiffOptions<'_>) -> Result<bool>
// Returns true if any drift found (used to set exit code 1 in main).
```

### New helper: `src/diff_util.rs`

```rust
/// Compute a unified diff between two strings.
/// Returns None if the strings are equal.
/// `label_a` / `label_b` are used in the --- / +++ header lines.
pub fn unified_diff(
    a: &str,
    b: &str,
    label_a: &str,
    label_b: &str,
    context_lines: usize,
) -> Option<String>

/// Apply ANSI color codes to a unified diff string.
/// Lines starting with + → green, - → red, @@ → cyan, ---/+++ → bold.
/// No-op if the string is empty.
pub fn colorize_diff(diff: &str) -> String

/// Compute a --stat summary line for a file: "path | N +--"
pub fn stat_line(path: &str, diff: &str) -> String
```

Implementation: pure Rust using the `similar` crate (already widely used, small, no deps).
Produces standard unified diff format. Does not shell out to `diff(1)`.

#### Why `similar` and not `diff(1)`?

- Zero external process spawning — no `Command::new("diff")`.
- Works identically across platforms (including macOS where `diff` behavior differs).
- Produces `String` output directly — easy to colorize later.
- The crate is battle-tested (powers Cargo's output, used by `cargo-expand`).
- `similar = "2"` is a single crate with no transitive deps of concern.

### Changes to `src/homebrew.rs`

Add one new function:

```rust
/// Compare Brewfile declarations against installed packages.
///
/// Returns:
///   missing_from_system: formulas/casks in Brewfile but not installed
///   unreferenced: installed leaf formulas/casks not in any Brewfile
pub fn brewfile_diff(
    brew: &Path,
    brewfile_paths: &[&Path],
) -> Result<BrewfileDiff>

pub struct BrewfileDiff {
    pub missing_formulas: Vec<String>,   // in Brewfile, not installed
    pub missing_casks: Vec<String>,      // in Brewfile, not installed
    pub extra_formulas: Vec<String>,     // installed leaf, not in Brewfile
    pub extra_casks: Vec<String>,        // installed, not in Brewfile
}
```

`brew bundle check` only gives a boolean. The new function builds on the
existing `brew_leaves`, `brew_list_casks`, and `collect_brewfile_entries` to produce
the richer output needed for diff display.

**Note:** `brew list --formula` (all installed formulas, not just leaves) is used to
determine `missing_formulas` (packages declared in Brewfile but not yet installed). This
is distinct from `brew leaves` which is used for the "extra" direction.

### Changes to `src/commands/mod.rs`

Add `pub mod diff;`.

### Changes to `src/main.rs`

Add `Diff` command variant and dispatch:

```rust
/// Show differences between tracked source files/packages and live state.
///
/// Exits 0 if everything is clean, 1 if drift is found.
///
/// Examples:
///   dfiles diff
///   dfiles diff --files
///   dfiles diff --brews
///   dfiles diff --profile work
Diff {
    #[arg(long)] profile: Option<String>,
    #[arg(long)] module: Option<String>,
    #[arg(long)] files: bool,
    #[arg(long)] brews: bool,
    #[arg(long)] ai: bool,
    #[cfg_attr(debug_assertions, arg(long, value_name = "DIR"))]
    #[cfg_attr(not(debug_assertions), arg(skip))]
    dest: Option<PathBuf>,
},
```

Dispatch:
```rust
Commands::Diff { profile, module, files, brews, ai, dest } => {
    let resolved = resolve_profile(profile.as_deref(), &state_dir);
    let dest_root_buf = dest.as_deref().map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    let none_specified = !files && !brews && !ai;
    let has_drift = commands::diff::run(&commands::diff::DiffOptions {
        repo_root: &repo,
        dest_root: &dest_root_buf,
        state_dir: &state_dir,
        claude_dir: &claude_dir,
        profile: &resolved,
        module_filter: module.as_deref(),
        diff_files: *files || none_specified,
        diff_brews: *brews || none_specified,
        diff_ai: *ai || none_specified,
    })?;
    if has_drift { std::process::exit(1); }
}
```

---

## Implementation details

### Binary file detection

Before diffing, check if either file contains a null byte. If so, print:
```
  M ~/.config/prefs.bin  (binary files differ)
```
Skip the unified diff for that file.

### Template rendering errors during diff

If a template fails to render (e.g., `op()` call and 1Password not signed in), print:
```
  ~ ~/.gitconfig  (template render failed: op not authenticated — skipping diff)
```
Continue to the next file. Do not abort the entire diff run.

### Large diffs

No truncation. If a file is very large and fully replaced, the diff will be large.
Deferred: pager support.

### Colors

When stdout is a tty (`isatty(STDOUT_FILENO)`), apply ANSI codes:
- `+` lines → green
- `-` lines → red
- `@@ ... @@` hunk headers → cyan
- `---` / `+++` header lines → bold

When stdout is piped (e.g. `dfiles diff | grep ...`), no ANSI codes.

Use the `is-terminal` crate for cross-platform tty detection. Add a
`--color=always|never|auto` flag (default: `auto`).

### `--stat` summary mode

When `--stat` is given, instead of showing diff content, show a compact summary:
```
~/.gitconfig     | 3 +--
~/.zshrc         | 12 ++++++------
brew/Brewfile    | M (packages not installed)
[shell] skill    | ? (not installed)
```
The `--stat` flag is mutually exclusive with content display but can be combined with
section flags (`dfiles diff --files --stat`).

### Symlinks

For symlink entries, diff is not meaningful — the "content" is just the link target path.
Show a one-liner:
- If link target matches: clean (no output).
- If link target differs or dest is not a symlink: show the expected vs actual target.

### Section flag semantics

Identical to `apply`: `--files`, `--brews`, `--ai`. No flags = all sections.
`--module` scopes only brew/AI (same as apply).

---

## File structure

```
src/
  drift.rs            ← new: DriftKind + check_drift* extracted from status.rs
  diff_util.rs        ← new: unified_diff(), colorize_diff(), stat_line()
  commands/
    diff.rs           ← new: DiffOptions, run()
    status.rs         ← refactored: imports from drift.rs (no behavior change)
    mod.rs            ← add pub mod diff
  homebrew.rs         ← add brewfile_diff() / BrewfileDiff; apply.rs refactored to use it
  main.rs             ← add Diff variant + dispatch

tests/
  integration.rs      ← add diff tests (see Test Plan)
```

**Total new code:** ~420 lines (excluding tests; +70 for colors, stat, drift extraction).
**Files touched:** 7 (drift.rs, diff_util.rs, diff.rs, status.rs, homebrew.rs, apply.rs, main.rs).

---

## Dependencies

Add to `Cargo.toml`:
```toml
similar = "2"        # unified diff generation (no transitive deps)
is-terminal = "0.4"  # cross-platform isatty() for --color=auto
```

---

## Test plan

### Unit tests (`src/diff_util.rs`)

| Test | What it verifies |
|------|-----------------|
| `identical_strings_returns_none` | No diff when content is the same |
| `single_line_change_produces_diff` | `+`/`-` lines appear correctly |
| `added_lines_only` | Dest shorter than source |
| `removed_lines_only` | Source shorter than dest |
| `context_lines_respected` | Surrounding unchanged lines are included |
| `empty_vs_nonempty` | One side empty |
| `multiline_file_with_one_change` | Only the changed hunk is shown |
| `colorize_diff_adds_ansi_green_to_plus_lines` | ANSI codes applied correctly |
| `colorize_diff_adds_ansi_red_to_minus_lines` | ANSI codes applied correctly |
| `colorize_diff_noop_on_empty` | No panic/corruption on empty input |
| `stat_line_shows_plus_minus_counts` | e.g. `~/.gitconfig | 3 +--` |

### Unit tests (`src/homebrew.rs` additions)

| Test | What it verifies |
|------|-----------------|
| `brewfile_diff_empty_brewfile_all_extra` | Every installed pkg shows as extra |
| `brewfile_diff_all_declared_installed` | Empty diff when everything matches |
| `brewfile_diff_missing_formula` | Formula in Brewfile but not installed shows as missing |
| `brewfile_diff_extra_cask` | Installed cask not in Brewfile shows as extra |

### Integration tests

All integration tests use `--dest <tmpdir>` to avoid touching the real home directory.

#### Files section

| Test | Setup | Expected |
|------|-------|----------|
| `diff_clean_file_no_output` | source file applied to dest | no `[files]` section |
| `diff_modified_file_shows_diff` | source and dest differ by one line | diff block with `+`/`-` |
| `diff_missing_dest_shows_missing` | source exists, dest not applied | `? ~/.foo` line |
| `diff_source_missing_shows_source_missing` | dest exists, source deleted | `! ~/.foo` line |
| `diff_template_rendered_before_compare` | `.tmpl` source, rendered dest matches | no output |
| `diff_template_rendered_diff_shows_delta` | `.tmpl` source, dest has old value | diff block |
| `diff_template_render_error_shows_tilde_marker` | `.tmpl` with `{{ undefined }}`, dest exists | `~` marker, exit 0 |
| `diff_binary_file_shows_notice` | binary source and dest differ | `(binary files differ)` |
| `diff_symlink_correct_target_no_output` | symlink_ entry, dest points to source | no output |
| `diff_symlink_wrong_target_shows_mismatch` | symlink_ entry, dest points elsewhere | `M` + path detail |
| `diff_files_flag_only` | `--files` flag | only files section |
| `diff_no_flags_shows_all` | no flags, mixed drift | all sections present |
| `diff_stat_shows_summary_line` | `--stat`, one modified file | `~/.foo | N +-` line, no diff block |

#### Brew section

| Test | Setup | Expected |
|------|-------|----------|
| `diff_brews_no_brew_installed_skips_gracefully` | brew not found | `[brew] skipped` |
| `diff_brews_flag_only` | `--brews` | only brew section |

#### AI section

| Test | Setup | Expected |
|------|-------|----------|
| `diff_ai_missing_skill_shows_minus` | skill declared, not installed | `- fetch skill: ...` |
| `diff_ai_installed_skill_no_output` | skill declared and installed | clean |
| `diff_ai_flag_only` | `--ai` | only AI section |

#### Exit code

| Test | Setup | Expected |
|------|-------|----------|
| `diff_exits_0_when_clean` | all clean | exit code 0 |
| `diff_exits_1_when_drift` | one file differs | exit code 1 |

---

## State machine: file diff outcomes

```
SourceEntry
     │
     ├── flags.symlink == true
     │         │
     │         ├── dest is correct symlink ──────────────────────── [clean]
     │         ├── dest is wrong symlink / regular file ─────────── [show target mismatch]
     │         └── dest missing ──────────────────────────────────── [?]
     │
     ├── flags.template == true
     │         │
     │         ├── src missing ───────────────────────────────────── [!]
     │         ├── render fails ─────────────────────────────────── [~ render error]
     │         ├── dest missing ──────────────────────────────────── [?]
     │         ├── rendered == dest ──────────────────────────────── [clean]
     │         └── rendered != dest ──────────────────────────────── [unified diff]
     │
     └── plain file
               │
               ├── src missing ───────────────────────────────────── [!]
               ├── dest missing ──────────────────────────────────── [?]
               ├── either is binary & differs ────────────────────── [binary notice]
               ├── content equal ─────────────────────────────────── [clean]
               └── content differs ───────────────────────────────── [unified diff]
```

---

## Relationship to existing commands

| Command | Purpose | Shows what |
|---------|---------|------------|
| `dfiles status` | Quick drift check | Which files/modules have drift (no content) |
| `dfiles diff` | Detailed drift inspection | What the actual changes are |
| `dfiles apply` | Resolve drift | Apply source state to machine |

`diff` is intentionally complementary to `status`, not a replacement. `status` is fast
and good for "is anything out of date?" checks. `diff` is for "what exactly changed?"

---

## Deferred (TODOS candidates)

- **Configurable external diff tool**: `dfiles.toml` `[diff] tool = "delta"` — pipe output through an external colorizer. (Captured in TODOS.md)
- **`dfiles diff <path>`**: diff a single file by destination path.
- **AI version drift**: compare installed skill commit SHA against `dfiles.lock` pin. Blocked on SHA verification. (Captured in TODOS.md)
- **Cross-machine diff**: `dfiles diff --machine <hostname>` — P3, see TODOS.md.
- **Pager support**: pipe long output through `$PAGER` automatically.

---

## Open questions

1. **`--color` flag vs auto-detect?** Always auto-detect `isatty` (consistent with `git diff`), or require explicit `--color`?
   _Leaning toward: auto-detect in v1 once colors are added._

2. **Template render errors — hard fail or skip?** If `op()` isn't authenticated and a template uses it, should we fail the whole diff or skip that file with a warning?
   _Leaning toward: skip with `~` marker (see Template rendering errors section)._

3. **`dfiles diff` vs `dfiles status --diff`?** Could be a flag on status instead of a new subcommand.
   _Leaning toward: separate command — cleaner UX, separate exit code semantics, mirrors chezmoi's design._
