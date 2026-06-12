# PRD: Generate Mise Global Config on Apply

## Overview

Haven modules can declare a mise config file (`[mise] config = "mise/mise.<module>.toml"`)
that lists the tools for that module. Currently, `haven apply` runs `mise install` per
module using `MISE_CONFIG_FILE=<module-config>`, which installs tools but does not make
them active on PATH. Mise only adds tools to PATH when they appear in a config file in its
standard lookup chain (primarily `~/.config/mise/config.toml`).

This feature adds a step to `haven apply` that merges the `[tools]` sections from all
active module mise configs into `~/.config/mise/config.toml`, replacing the tools list
while preserving the `[settings]` section. A single `mise install` is then run against
the merged global config, ensuring all declared tools are both installed and active on PATH.

## Goals

- Make module-declared mise tools appear on PATH after `haven apply`.
- Generate `~/.config/mise/config.toml [tools]` deterministically from module files —
  single source of truth.
- Preserve any `[settings]` (and other non-`[tools]` sections) in the user's global config.
- Keep the per-module mise files (`mise/mise.<module>.toml`) as the canonical declaration;
  the global config is a generated artifact.

## Non-Goals

- Changing how `haven pkg install --mise` writes module files.
- Managing mise plugins, aliases, tasks, or config sections beyond `[tools]`.
- Per-project `.mise.toml` files in project directories.
- Removing the `mise/` module files — they remain the source of truth.

## Requirements

### Functional Requirements

- REQ-F-001: After processing all modules, Haven collects the resolved path of every
  active module's mise config file (those where `module.mise.config` is set).
- REQ-F-002: Haven reads the `[tools]` table from each collected config file using the
  existing `parse_mise_tools()` function.
- REQ-F-003: Haven merges all collected `[tools]` entries into one unified table. If the
  same tool key appears in multiple modules, the last active module's value wins (module
  order follows the profile's module list order).
- REQ-F-004: Haven writes the merged `[tools]` table into `~/.config/mise/config.toml`,
  replacing the existing `[tools]` section entirely. All other sections (e.g. `[settings]`,
  `[env]`) are preserved unchanged using `toml_edit`.
- REQ-F-005: If `~/.config/mise/config.toml` does not exist, Haven creates it with only
  the merged `[tools]` section.
- REQ-F-006: After writing the merged config, Haven runs a single `mise install` (without
  `MISE_CONFIG_FILE`) so mise uses the global config and installs all declared tools.
- REQ-F-007: If mise is not installed, Haven skips the merge step with a warning, matching
  existing behavior.
- REQ-F-008: In `--dry-run` mode, Haven prints the merged `[tools]` content that would be
  written without modifying any file.

### Non-Functional Requirements

- REQ-NF-001: `cargo build` and `cargo test` pass with zero errors.
- REQ-NF-002: The merge step uses `toml_edit` (already a dependency) so the global config
  retains any hand-edited formatting in non-`[tools]` sections.
- REQ-NF-003: The per-module `mise install` calls are replaced by the single post-merge
  install — no redundant installs.

## Technical Considerations

**Existing utilities to reuse:**
- `src/mise.rs::parse_mise_tools(path)` — reads `[tools]` from a mise config; returns
  `Result<Vec<(String, String)>>` (or similar).
- `src/mise.rs::load_or_create_doc(path)` — loads a TOML file as `toml_edit::DocumentMut`
  or creates an empty one.
- `src/mise.rs::install_tools(mise, config)` — runs `mise install` with optional
  `MISE_CONFIG_FILE`.
- `src/config/module.rs::MiseConfig.config` — `Option<String>` relative path to the
  module mise config.

**New function in `src/mise.rs`:**
```rust
pub fn merge_module_tools_into_global(
    module_configs: &[PathBuf],
    global_config: &Path,
) -> Result<()>
```
Reads `[tools]` from each module config, merges them, and writes to `global_config`
replacing only the `[tools]` table.

**Apply pipeline change in `src/commands/apply.rs`:**
- During the module loop, collect `config_path` for each module that has a mise config.
- After the module loop, call `merge_module_tools_into_global()` with the collected paths
  and `~/.config/mise/config.toml` as the target.
- Run `install_tools(mise, None)` once (no `MISE_CONFIG_FILE`) so mise uses the global
  config.
- Remove the per-module `install_tools()` call (it's superseded by the post-merge install).

**Global config path:** `~/.config/mise/config.toml` — expand via `dirs::home_dir()` or
the existing `expand_tilde()` helper already in the codebase.

## Acceptance Criteria

- [ ] `merge_module_tools_into_global()` exists in `src/mise.rs` and is tested.
- [ ] After `haven apply`, `~/.config/mise/config.toml` contains the merged `[tools]`
  from all active module mise configs.
- [ ] `[settings]` and other sections in the global config are preserved.
- [ ] A single `mise install` runs at the end (not one per module).
- [ ] `--dry-run` prints what would be written without touching the global config.
- [ ] `cargo test` and `cargo test --test integration` pass.

## Out of Scope

- Migrating existing tools out of a hand-edited `~/.config/mise/config.toml`.
- Conflict resolution when the same tool appears in multiple modules with different versions
  (last module in profile order wins — no warning needed).
- Haven-managed removal of tools from the global config when a module is removed from the
  profile (future work).
