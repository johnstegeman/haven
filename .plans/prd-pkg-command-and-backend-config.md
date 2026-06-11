# PRD: `haven pkg` command + backend configuration (Homebrew)

> **Part 1 of 3** in the unified package-management effort. Ships independently.
> Order: **this PRD → `prd-pkg-mise-backend` → `prd-pkg-outdated-upgrade-search`**.
> The product (full vision) ships only when all three land.

## Overview

Introduce `haven pkg` as the single front-end for declarative package management, backed by a
pluggable backend abstraction declared in `haven.toml`. In this PRD only the **Homebrew** backend is
wired in. `haven pkg install/uninstall` over brew behaves exactly like `haven brew install/uninstall`
does today. The legacy `haven brew` command is **removed** and fully replaced by `haven pkg`.

This establishes the config schema (`[packages] backends`), the backend-resolution logic
(default = preference-order head; `--brew`/`--mise`/`--cask` overrides), and the `commands/pkg.rs`
dispatcher that subsequent PRDs extend with the mise backend and the outdated/upgrade/search verbs.

## Goals

- One command, `haven pkg`, for package management.
- `[packages] backends = [...]` in `haven.toml` declares allowed backends in preference order.
- Backend resolution: explicit `--brew`/`--mise` flags, `--cask` implies brew, otherwise the first
  allowed backend. (The `--mise` flag is accepted/validated here but has no backend behind it until
  Part 2 — selecting it errors cleanly: "mise backend not yet available".)
- Remove `haven brew`; preserve all current brew behavior under `haven pkg`.

## Non-Goals

- Any mise install/uninstall behavior (Part 2).
- `outdated` / `upgrade` / `search` verbs (Part 3).
- Changing Brewfile format, layout (`brew/Brewfile`, `brew/Brewfile.<module>`), or sorting behavior.

## Requirements

### Functional Requirements

- REQ-F-001: Add `[packages]` section to `HavenConfig` (`src/config/haven.rs`) with
  `#[serde(default)] backends: Vec<String>`, backward-compatible (existing `haven.toml` parses fine).
- REQ-F-002: `allowed_backends()` accessor: an empty/absent list defaults to `["brew", "mise"]`
  (both enabled, brew preferred). Unknown backend names are a hard error naming the offending value.
- REQ-F-003: `default_backend()` returns the first entry of the resolved allowed list.
- REQ-F-004: New top-level `Pkg { #[command(subcommand)] action: PkgAction }` in `src/main.rs`, with
  `PkgAction::Install { name, brew, mise, cask, module }` and
  `PkgAction::Uninstall { name, brew, mise, cask, module }` (derive clap, nested-subcommand style).
- REQ-F-005: New `src/commands/pkg.rs` with `resolve_backend(flags, cfg)`:
  `--cask`⇒brew; `--brew`/`--mise` force that backend; else `default_backend()`. Forcing or
  defaulting to a backend not in `allowed_backends()` is a clear error.
- REQ-F-006: `pkg::install`/`pkg::uninstall` dispatch the brew backend to the existing
  `commands::brew::{install,uninstall}` logic — unchanged Brewfile + module-config side effects.
- REQ-F-007: Remove the `Brew`/`BrewAction` clap variants and their dispatch from `src/main.rs`.
  Retain `src/commands/brew.rs` and `src/homebrew.rs` as the brew backend implementation (now invoked
  via `pkg`).
- REQ-F-008: Selecting `--mise` (or a default of `mise`) errors with a forward-looking message until
  Part 2; it must not panic or partially mutate state.

### Non-Functional Requirements

- REQ-NF-001: Zero behavior change for brew users beyond the command rename (`haven pkg install X` ≡
  former `haven brew install X`).
- REQ-NF-002: `cargo clippy --all-targets` clean; no new dependencies (all needed crates present).

## Technical Considerations

- `HavenConfig` is a plain serde struct (`src/config/haven.rs:7`); add `packages` with `#[serde(default)]`.
- Mirror existing CLI conventions: `Brew { action }` at `src/main.rs:664`, `BrewAction` at `:193`,
  dispatch at `:1299`. The `Pkg` variant slots in the same way.
- `commands/brew.rs` `install(repo, name, cask, module)` and `uninstall(repo, name, cask)` already
  encapsulate Brewfile + module-config writes — `pkg.rs` calls these directly.
- Search the repo for every reference to `haven brew` (tests, docs, CLAUDE.md, completions) and update.

## Acceptance Criteria

- [ ] `[packages] backends = ["mise","brew"]` parses; absent section ⇒ `["brew","mise"]`; unknown name errors.
- [ ] `haven pkg install ripgrep` adds to the master Brewfile and runs brew, identical to old `haven brew`.
- [ ] `haven pkg install iterm2 --cask` and `--module <m>` behave as the old brew command did.
- [ ] `haven pkg uninstall ripgrep` removes from all Brewfiles and uninstalls.
- [ ] `haven brew ...` no longer exists (help, dispatch, completions all updated).
- [ ] `haven pkg install X --mise` (or mise-default) prints a clean "not yet available" error, no state change.
- [ ] Integration tests cover backend resolution + the brew path; old brew tests are migrated to `pkg`.
- [ ] clippy clean; docs/CLAUDE.md updated for `[packages]` and the `pkg` command.

## Out of Scope

mise backend, `outdated`/`upgrade`/`search`, status/diff version reporting, mise config relocation.

## Open Questions

None — defaults resolved: backends default `["brew","mise"]`; `haven brew` removed entirely.
