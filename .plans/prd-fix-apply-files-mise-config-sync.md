# PRD: Fix apply --files Mise Config Sync

## Overview

Two related bugs affect how `haven apply` handles the global mise config
(`~/.config/mise/config.toml`) when that file is tracked as a source file in `source/`:

1. **`apply --files` wipes mise tools.** The files phase copies the bare source version of
   the config (no `[tools]`) to `~/.config/mise/config.toml`, but the mise merge step is
   gated on `opts.apply_ai`, which is `false` for `apply --files`. The `[tools]` section is
   left empty, breaking `mise activate`.

2. **False conflict prompt on subsequent apply.** After a full `apply`, the files phase
   stores the SHA of the bare config in `state.applied_files`. The mise merge then writes
   the merged config (with `[tools]`) to disk. On the next apply, the files phase reads the
   destination (with merged tools), computes its SHA, and finds a mismatch — even though
   the user made no manual edits — causing an unexpected conflict prompt or skip warning.

Both bugs stem from the same root cause: the mise merge step is architecturally coupled to
`apply_ai` even though writing the global mise config is a consequence of the files phase,
not of AI skill deployment.

## Goals

- `apply --files` restores the `[tools]` section after copying the bare source config.
- Subsequent `apply` or `apply --files` runs do not report a false conflict on
  `~/.config/mise/config.toml` when only the merged tools differ from the stored hash.
- Behavior of `apply`, `apply --files`, `apply --ai`, and `apply --brews` in isolation and
  combined remains correct.

## Non-Goals

- Running `mise install` during `apply --files` (installing packages is not a files
  concern; the merge alone fixes the config).
- Changing when or how `haven pkg install --mise` writes module files.
- Conflict detection for user-edited sections of the global config (that should still fire).

## Requirements

### Functional Requirements

- REQ-F-001: When `opts.apply_files` or `opts.apply_ai` is true, and at least one active
  module has a mise config path, `merge_module_tools_into_global()` is called after the
  module loop, writing the merged `[tools]` to `~/.config/mise/config.toml`.
- REQ-F-002: `mise install` (without `MISE_CONFIG_FILE`) is only run when `opts.apply_ai`
  is true, keeping `apply --files` as a pure files+config-repair operation.
- REQ-F-003: After `merge_module_tools_into_global()` writes the global config, if
  `~/.config/mise/config.toml` is tracked in `state.applied_files` (i.e., it is a managed
  source file), its stored SHA is updated to the post-merge file content. This prevents
  the next apply from reporting a false conflict.
- REQ-F-004: In `--dry-run` mode, dry-run output for the mise merge is only printed when
  `opts.apply_ai` is true (preserving existing dry-run behavior; `apply --files --dry-run`
  does not show merge output).
- REQ-F-005: The `state.modules` update and op-skip warning in the module loop remain
  gated on `opts.apply_ai` only — they are not changed.

### Non-Functional Requirements

- REQ-NF-001: `cargo build` and `cargo test` pass with zero errors.
- REQ-NF-002: No new conditional branches are added outside `src/commands/apply.rs`.
- REQ-NF-003: The implementation uses only existing helpers: `sha256_of_bytes`,
  `AppliedFileEntry`, `merge_module_tools_into_global`, `mise_global_config_path`.

## Technical Considerations

**Root change in `src/commands/apply.rs`:**

The outer condition on the module iteration block and mise merge block changes from:
```rust
if opts.apply_ai { … }
```
to:
```rust
if opts.apply_ai || opts.apply_files { … }
```

Inside the loop, sub-concerns are re-gated:
- `print_dry_run_module(…)` → gated on `opts.apply_ai`
- `state.modules.insert(…)` → gated on `opts.apply_ai`
- The dry-run merge preview → gated on `opts.apply_ai`

The merge block is restructured to:
1. Always call `merge_module_tools_into_global()` (pure file I/O, no mise binary needed).
2. Update `state.applied_files["~/.config/mise/config.toml"]` if tracked.
3. Only call `install_tools(&mise, None)` when `opts.apply_ai` is true.

**SHA tracking key:** `state.applied_files` uses `dest_tilde` strings as keys. The global
mise config's tilde key is always `"~/.config/mise/config.toml"`. This matches the
`dest_tilde` that the files phase would record for `source/dot_config/mise/config.toml`.

**Imports already in scope:** `sha256_of_bytes` and `AppliedFileEntry` are already imported
in `apply.rs`; `mise_global_config_path` is already called in the block being modified.

## Acceptance Criteria

- [ ] `apply --files` with a module that has a mise config → `~/.config/mise/config.toml`
      contains the merged `[tools]` after the run.
- [ ] Full `apply` followed by `apply` again → no conflict warning/prompt on
      `~/.config/mise/config.toml`.
- [ ] `apply --files` followed by `apply --files` again → no conflict warning/prompt.
- [ ] `apply --ai` behavior is unchanged (merge runs, install runs).
- [ ] `apply --files --dry-run` does not print mise merge output.
- [ ] `cargo build` and `cargo test` pass.

## Out of Scope

- Making `apply --files` run `mise install`.
- Per-module mise config conflict handling.
- Detecting user edits to `~/.config/mise/config.toml` non-tools sections (existing behavior unchanged).
