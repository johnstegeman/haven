# PRD: Regenerate Mise Global Config After pkg install/uninstall

## Overview

`haven apply` now merges all active module mise configs into `~/.config/mise/config.toml`
so tools are active on PATH. But `haven pkg install <name> --mise` and
`haven pkg uninstall <name> --mise` only modify the module-level `mise/mise.<module>.toml`
file — they do not call `merge_module_tools_into_global()`. The global config is stale
until the next `haven apply`, leaving the user with a tool that is tracked but not on PATH.

## Goals

- After `haven pkg install <name> --mise`, the global `~/.config/mise/config.toml` is
  regenerated from all module mise files and `mise install` runs so the tool is immediately
  active on PATH.
- After `haven pkg uninstall <name> --mise`, the global config is similarly regenerated so
  the tool is removed from PATH without requiring a full `haven apply`.

## Non-Goals

- Changes to the brew backend or `haven pkg install --brew`.
- Changes to `haven apply` (already correct).
- Changing the CLI argument surface.

## Requirements

### Functional Requirements

- REQ-F-001: `haven pkg install <name> --mise` regenerates `~/.config/mise/config.toml`
  from all `mise/mise*.toml` module files after writing the tool entry, then runs
  `mise install` without `MISE_CONFIG_FILE` so mise uses the global config.
- REQ-F-002: `haven pkg uninstall <name> --mise` regenerates `~/.config/mise/config.toml`
  from all remaining `mise/mise*.toml` module files after removing the tool entry.
- REQ-F-003: If mise is not installed, the merge still runs (it is pure file I/O); only
  the `mise install` call is skipped — matching the approach in `apply.rs`.
- REQ-F-004: Behaviour is unchanged when no `mise/mise*.toml` files exist.

### Non-Functional Requirements

- REQ-NF-001: `cargo build` and `cargo test` pass with zero errors.
- REQ-NF-002: No new abstractions — reuse existing functions only.

## Technical Considerations

**File to change:** `src/commands/mise.rs` only.

**Existing utilities to reuse:**
- `all_misefiles(repo_root)` (in `src/commands/mise.rs`) — returns all `mise/mise*.toml`
  paths. Already called in `uninstall()`; add to `install()`.
- `crate::mise::merge_module_tools_into_global(&[PathBuf], &Path)` (in `src/mise.rs`) —
  merges `[tools]` sections from module files into the global config.
- `crate::mise::install_tools(&Path, None)` (in `src/mise.rs`) — runs `mise install`
  against the global config (pass `None` for `MISE_CONFIG_FILE`).
- `crate::mise::mise_path()` (in `src/mise.rs`) — resolves the mise binary.
- `expand_tilde("~/.config/mise/config.toml")` — expands the global config path.
  `expand_tilde` is imported from `config::module` in the file.

**`install()` change** (currently calls `install_tools(bin, Some(config_path))`):

```
1. add_to_misefile(config_path, name, version)   ← unchanged
2. all_misefiles(repo_root)                       ← new: collect all module files
3. merge_module_tools_into_global(&all, &global)  ← new: regenerate global config
4. if let Some(bin) = mise_path() {
       install_tools(&bin, None)                  ← changed: was Some(config_path)
   }
```

**`uninstall()` change** (currently calls `mise_uninstall` but not `merge`):

```
1. all_misefiles(repo_root)                       ← unchanged (already collected)
2. remove_from_misefile(path, name) for each      ← unchanged
3. merge_module_tools_into_global(&all, &global)  ← new: regenerate global config
4. if let Some(bin) = mise_path() {
       mise_uninstall(&bin, name)                 ← unchanged
   }
```

## Acceptance Criteria

- [ ] `install()` calls `merge_module_tools_into_global` after `add_to_misefile`.
- [ ] `install()` calls `install_tools(bin, None)` instead of `install_tools(bin, Some(path))`.
- [ ] `uninstall()` calls `merge_module_tools_into_global` after all `remove_from_misefile` calls.
- [ ] `cargo build` and `cargo test` pass.
- [ ] Integration test verifies global config is updated after `pkg install --mise`.

## Out of Scope

- Handling the case where a module was removed from the profile but its mise file still
  exists (tool cleanup across profiles).
- `haven pkg upgrade --mise` (separate command, not changed here).
