# Design: Source-encoded externals (`extdir_` prefix)

**Status:** Implemented
**Branch:** main
**Replaces:** `[[externals]]` entries in `config/modules/*.toml`

---

## Problem

External directories (git repos cloned into the live filesystem — e.g. tmux plugin
managers, Neovim configs) are currently tracked as `[[externals]]` entries in
`config/modules/<module>.toml`:

```toml
[[externals]]
dest = ".tmux/plugins/tpm"
type = "git"
url  = "https://github.com/tmux-plugins/tpm.git"
```

This has two friction points:

1. **Split source of truth.** Files live under `source/`, but externals live in
   `config/modules/`. There is no single place to look to understand everything
   haven manages.

2. **`haven diff` cannot see externals via `source::scan()`.** The diff command
   scans `source/` for files to compare; externals require a separate code path.
   This makes the diff/status/apply logic inconsistent — externals are second-class
   citizens.

---

## Proposed solution

Encode external directories as marker files inside `source/`, using the magic prefix
`extdir_`. The file's location encodes the destination parent, and its name (after
stripping the prefix) encodes the destination directory name.

### Naming convention

```
source/<encoded-parent-path>/extdir_<dest-dir-name>
```

**Example:**

| source/ path                              | live destination              |
|-------------------------------------------|-------------------------------|
| `dot_tmux/plugins/extdir_tpm`             | `~/.tmux/plugins/tpm`         |
| `dot_config/nvim/extdir_lazy.nvim`        | `~/.config/nvim/lazy.nvim`    |
| `dot_oh-my-zsh/custom/extdir_plugins`     | `~/.oh-my-zsh/custom/plugins` |

The parent path uses the same `dot_` / `private_` encoding rules as all other
`source/` entries. The `extdir_` file itself is a regular file (not a directory).

After stripping `extdir_`, the remainder is run through the normal `decode_component()`
logic — so `extdir_dot_tpm` → dest name `.tpm`. This is consistent with the rest of
the `source/` naming system.

### File content

The file uses the same TOML schema as the existing `[[externals]]` entries in
`config/modules/*.toml`, minus the `dest` field (which is now encoded in the path):

```toml
type = "git"
url  = "https://github.com/tmux-plugins/tpm.git"
```

With an optional pinned ref (branch, tag, or SHA):

```toml
type = "git"
url  = "https://github.com/tmux-plugins/tpm.git"
ref  = "v3.0.0"
```

This reuses the existing schema users already know, avoids URL parsing hacks
(no `url@ref` split), and leaves room for future fields (`depth`, `sparse-checkout`,
etc.) without a format change.

---

## Data flow

```
source/
  dot_tmux/
    plugins/
      extdir_tpm          ← "clone tmux-plugins/tpm into ~/.tmux/plugins/tpm"

haven apply
  source::scan()
    ├─ plain files  → copy to dest
    ├─ templates    → render + copy
    ├─ symlinks     → symlink to source
    └─ extdir_*     → git clone/pull url into dest directory
                       (skip if already present and up to date)

haven diff
  source::scan()
    └─ extdir_*     → check if dest dir exists
                       missing → "? ~/.tmux/plugins/tpm"
                       present → (v2: check HEAD vs pinned ref)

haven status
  source::scan()
    └─ extdir_*     → ✓ / ? / M markers (same as files)
```

---

## SourceEntry changes

`source::scan()` returns `Vec<SourceEntry>`. Add a new flag:

```rust
pub struct SourceFlags {
    pub symlink:  bool,
    pub template: bool,
    pub private:  bool,
    pub executable: bool,
    pub extdir:   bool,   // ← new
}
```

When `extdir = true`:
- `entry.src` points to the `extdir_<name>` marker file in `source/`
- `entry.dest_tilde` is the live destination directory (e.g. `~/.tmux/plugins/tpm`)
- The URL (and optional ref) are read from the marker file's content at apply/diff time

---

## apply behavior

```
for each SourceEntry where flags.extdir:
  toml = parse_extdir_content(entry.src)   // hard error on invalid TOML or missing url
  url  = toml.url
  ref  = toml.ref  // optional
  type = toml.type ?? "git"   // default to "git" if omitted
  dest = resolve_dest(entry.dest_tilde)
  if dest.exists() and is_git_repo(dest):
    if --apply-externals flag set:
      git pull --ff-only  (or fetch + checkout if ref is pinned)
    else:
      skip (already present)
  elif not dest.exists():
    git clone url [--branch ref] dest
  else:
    warn "dest exists but is not a git repo — manual resolution required"
    continue
```

**Default behavior:** skip externals that are already present as git repos.
**Opt-in update:** `--apply-externals` flag triggers pull/checkout.

For `--dry-run`, print `  [extdir] clone <url> → <dest>` without running git.

**Error handling:** Invalid TOML in an `extdir_` marker file is a hard error that aborts apply with a clear message identifying the offending file.

---

## diff behavior

**Phase 1 (this PR):** Existence check only.
```
  ? ~/.tmux/plugins/tpm   (extdir: not cloned)
```

**Phase 2 (future TODO):** Pinned-ref drift.
```
  M ~/.tmux/plugins/tpm   (extdir: at abc1234, expected v3.0.0)
```

---

## Migration from `[[externals]]` TOML entries

No migration needed — `[[externals]]` was never used in a shipped version. Remove
`ExternalEntry` and `[[externals]]` parsing from `config/module.rs` entirely as part
of this PR. Delete the old externals loops in `apply.rs`, `status.rs`, and
`print_dry_run_module()`. Modules that have no other TOML keys (no `homebrew`, `ai`,
`requires_op`) can be deleted entirely.

---

## `extfile_` (future, P2)

An `extfile_<name>` prefix would fetch a single remote file (e.g. a binary or
archive asset) rather than cloning a repo. Not in scope for this PR — reserved as a
deferred TODO.

---

## Files touched

| File | Change |
|------|--------|
| `src/source.rs` | Decode `extdir_` entries in `scan()`, populate `SourceFlags::extdir` |
| `src/commands/apply.rs` | Handle `entry.flags.extdir` — git clone/pull |
| `src/commands/diff.rs` | Handle `entry.flags.extdir` — existence check |
| `src/commands/status.rs` | Handle `entry.flags.extdir` — drift marker |
| `src/commands/import.rs` | Write `extdir_` files instead of `[[externals]]` TOML entries |
| `src/config/module.rs` | Remove `ExternalEntry` struct and `[[externals]]` parsing (or deprecate) |
| `tests/integration.rs` | New tests for extdir apply, diff, status, import |

---

## NOT in scope

- `extfile_` (single-file fetch) — P2 TODO
- Archive/tarball externals — P2 TODO
- Version drift checking (HEAD vs pinned ref) in diff — P2 TODO
- Auto-update of externals on `haven apply` — configurable, P2

---

## Open questions

1. **Conflict with existing directory entries.** If `source/dot_tmux/plugins/` also
   contains plain files, those are applied first and the `extdir_` clone happens
   after. Is this ordering safe? (Yes — git clone only runs if dest doesn't exist as
   a git repo.)

2. **`extdir_` inside a template-encoded path.** E.g.
   `dot_config/nvim.tmpl/extdir_lazy` — should template expansion apply to the
   parent path? Probably not; template files should not contain `extdir_` children.

3. **Deprecation timeline for `[[externals]]`.** Warn on first `haven apply` if
   old-style entries are found, auto-migrate on `haven apply --migrate` (or similar)?
