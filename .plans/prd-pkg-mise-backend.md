# PRD: mise as a managed `haven pkg` backend

> **Part 2 of 3** in the unified package-management effort. Ships independently.
> **Depends on `prd-pkg-command-and-backend-config`** (the `pkg` command + backend abstraction).
> Order: `prd-pkg-command-and-backend-config` → **this PRD** → `prd-pkg-outdated-upgrade-search`.

## Overview

Make mise a first-class, declarative backend behind `haven pkg`, with full CLI + per-module parity
with Homebrew. Adds a haven-internal `mise/` directory (`mise/mise.<module>.toml`, parallel to
`brew/Brewfile.<module>`, **not** deployed to the home dir), the mise config-editing primitives, a
`commands/mise.rs` analogue of `commands/brew.rs`, and wires mise into `haven pkg install/uninstall`
so the `--mise` flag and a mise default backend become functional.

After Part 1, `--mise` returns a "not yet available" error; this PRD removes that stub and implements
the real behavior.

## Goals

- `haven pkg install <tool> --mise [--module <m>]` writes `mise/mise.<module>.toml`, registers it in
  the module config `[mise] config`, and installs the tool.
- `haven pkg uninstall <tool>` (mise-resolved) removes the tool from every `mise/mise*.toml` and
  uninstalls the binary.
- Per-module mise configs live in a haven-internal `mise/` dir, symmetric with `brew/`.
- Config edits are format-preserving and testable without the mise binary.

## Non-Goals

- `outdated` / `upgrade` / `search` (Part 3).
- Auto-migrating existing `source/mise.toml` configs (documented manual step only).
- Deploying mise configs to `~/.config/mise` (explicitly internal-only per design decision).

## Requirements

### Functional Requirements

- REQ-F-001: mise config primitives in `src/mise.rs` using `toml_edit` (already a dep):
  `add_to_misefile(path, name, version)`, `remove_from_misefile(path, name)`, `parse_mise_tools(path)`
  — operate on the `[tools]` table; idempotent; create-if-missing; preserve comments/formatting.
- REQ-F-002: Tool-spec parsing: `name@version` ⇒ (name, version); bare `name` ⇒ version `"latest"`.
- REQ-F-003: New `src/commands/mise.rs` mirroring `commands/brew.rs`:
  - `install(repo, name, module_filter)`: resolve target (`mise/mise.<module>.toml`, or master
    `mise/mise.toml` when no module), `add_to_misefile`, register `[mise] config` via
    `ModuleConfig::load/save`, then run `mise::install_tools`.
  - `uninstall(repo, name)`: `remove_from_misefile` across all `mise/mise*.toml`, then `mise uninstall`.
  - `resolve_module_misefile` / `all_misefiles` mirroring the brew helpers.
- REQ-F-004: `mise::mise_uninstall(mise, name)` runs `mise uninstall <name>` (best-effort; absent tool
  is not an error).
- REQ-F-005: Wire the mise backend into `commands/pkg.rs` install/uninstall dispatch; remove the
  Part-1 "not yet available" stub. `--cask` with `--mise` is rejected (mise has no casks).
- REQ-F-006: `apply.rs` mise step is unchanged in mechanism (it already resolves `[mise].config` and
  passes `MISE_CONFIG_FILE`); only the convention for newly-created files is `mise/`. Existing
  `config = "source/mise.toml"` references keep working.
- REQ-F-007: `haven pkg uninstall` with both backends allowed removes the named tool from both brew
  and mise configs (idempotent) and runs each backend's uninstall best-effort.

### Non-Functional Requirements

- REQ-NF-001: All config-mutation logic unit-testable without the mise binary installed.
- REQ-NF-002: When mise is not installed, install/uninstall fail (or no-op) with a clear hint, never panic.
- REQ-NF-003: clippy clean; no new dependencies.

## Technical Considerations

- `commands/brew.rs` is the structural template: `resolve_module_brewfile` (`:249`),
  `all_brewfiles_with_sort` (`:111`), install/uninstall flow — replicate for mise files.
- `MiseConfig { config: Option<String> }` (`src/config/module.rs:62`) already models a per-module
  path; the CLI writes `mise/mise.<module>.toml` into it.
- mise config TOML shape: a `[tools]` table (`node = "20"`, `"npm:@anthropic-ai/claude-code" = "latest"`).
  Use `toml_edit::DocumentMut` for format-preserving inserts/removes.
- Editing the file with `toml_edit` (not shelling to `mise use`) keeps mutation logic unit-testable
  and matches how `homebrew.rs` hand-edits Brewfiles; the binary is only invoked for actual install.

## Acceptance Criteria

- [ ] `haven pkg install node --mise` creates/updates `mise/mise.toml` with a pinned `[tools]` entry and runs `mise install`.
- [ ] `--module shell` writes `mise/mise.shell.toml` and registers `[mise] config = "mise/mise.shell.toml"` in `modules/shell.toml`.
- [ ] `haven pkg uninstall node` removes the entry from every `mise/mise*.toml` and runs `mise uninstall`.
- [ ] `node@22` pins `22`; bare `node` pins `latest`; re-adding is idempotent; comments/formatting preserved.
- [ ] `--cask --mise` is rejected with a clear message.
- [ ] `haven apply` installs mise tools from the new `mise/` location; legacy `source/mise.toml` still applies.
- [ ] Unit tests for add/remove/parse mise tools (no binary); integration tests for the CLI paths (guarded like brew tests).
- [ ] clippy clean; docs/CLAUDE.md updated for the `mise/` dir + mise backend.

## Out of Scope

`outdated`/`upgrade`/`search`, version-pin rewriting on upgrade, status/diff version reporting,
auto-migration of `source/mise.toml`.

## Open Questions

- Exact reliable way to detect mise's available-tool registry is deferred to Part 3 (`search`); not needed here.
