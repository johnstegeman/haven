# PRD: `haven pkg outdated` / `upgrade` / `search`

> **Part 3 of 3** in the unified package-management effort. Ships independently.
> **Depends on `prd-pkg-command-and-backend-config` and `prd-pkg-mise-backend`**.
> Order: `prd-pkg-command-and-backend-config` → `prd-pkg-mise-backend` → **this PRD**.
> Completing this PRD ships the full vision.

## Overview

Add the cross-backend management verbs so users stop running `brew`/`mise` directly:
`haven pkg outdated`, `haven pkg upgrade`, and `haven pkg search`. These fan out across all allowed
backends. Critically, `haven pkg upgrade` for mise **rewrites the version pin back into the mise
config file**, so an upgrade on one machine propagates declaratively to others (brew Brewfiles don't
pin, so the brew path changes no config). This closes the gap left by `status`/`diff`, which report
package presence but never versions.

## Goals

- `haven pkg outdated`: unified, backend-grouped report of upgradable packages.
- `haven pkg upgrade [name]`: upgrade one or all; mise upgrades rewrite pins in `mise/mise*.toml`.
- `haven pkg search <term>`: query all allowed backends, show grouped results with install hints.

## Non-Goals

- Folding version-outdated info into `haven status` / `haven diff` (kept in the new `outdated` verb).
- Pinning versions in Brewfiles.
- A TUI/interactive picker for `search` (plain grouped output with copy-pasteable install commands).

## Requirements

### Functional Requirements

- REQ-F-001: `PkgAction::Outdated`, `PkgAction::Upgrade { name: Option<String> }`,
  `PkgAction::Search { term }` added to the clap enum + dispatch (`src/main.rs`).
- REQ-F-002: `mise::mise_outdated(mise, config)` → `mise outdated` (with `MISE_CONFIG_FILE`); parse to a
  list of (tool, current, latest). `homebrew::brew_outdated(brew)` → `brew outdated`.
- REQ-F-003: `pkg::outdated` fans out to every allowed backend and prints a grouped report; clearly
  states when a backend's binary is missing rather than failing the whole command.
- REQ-F-004: `mise::mise_upgrade(mise, config, name)` bumps tool(s) **and rewrites the pinned version
  in the targeted `mise/mise*.toml`**. Prefer mise's native pin-bump
  (verify exact flag — `mise upgrade --bump [--path <file>]` vs `MISE_CONFIG_FILE`); fallback:
  resolve the new version, rewrite `[tools]` via `toml_edit`, then `install_tools`.
- REQ-F-005: `homebrew::brew_upgrade(brew, name)` → `brew upgrade [name]` (no config mutation).
- REQ-F-006: `pkg::upgrade(name)` dispatches per backend: brew ⇒ `brew_upgrade`; mise ⇒ `mise_upgrade`
  with pin rewrite. With no `name`, upgrades all packages across all allowed backends.
- REQ-F-007: `mise::mise_search(mise, term)` queries the mise registry (verify subcommand:
  `mise registry` filtered, or equivalent); `homebrew::brew_search(brew, term)` → `brew search`.
- REQ-F-008: `pkg::search(term)` fans out, prints results grouped by backend, each with an install
  hint (e.g. `haven pkg install <name> --mise`).
- REQ-F-009: Naming-collision guard — top-level `haven upgrade` (binary self-update) is untouched;
  package upgrade is `haven pkg upgrade`. Both help texts disambiguate.

### Non-Functional Requirements

- REQ-NF-001: A missing backend binary degrades gracefully (skip with a note), never aborts the run.
- REQ-NF-002: Output parsing tolerant of mise/brew formatting variation; failures are reported, not panics.
- REQ-NF-003: clippy clean; no new dependencies.

## Technical Considerations

- The mise pin-rewrite is the core value of wrapping `upgrade` instead of running mise directly —
  without it the declarative contract silently breaks across machines. Verify the exact mise flag
  during implementation and assert (in a test or post-run read) that the config file's pin changed.
- `BrewfileDiff` (`src/homebrew.rs:271`) reports presence drift only — `outdated` is a distinct,
  version-level concern and is implemented separately, not by extending `brewfile_diff`.
- mise's `[tools]` editing for the rewrite reuses `toml_edit` helpers from Part 2.
- Fan-out reuses `allowed_backends()` (Part 1) and the per-module config discovery (Part 2).

## Acceptance Criteria

- [ ] `haven pkg outdated` lists upgradable brew + mise packages, grouped by backend; missing binary noted, not fatal.
- [ ] `haven pkg upgrade node` upgrades node **and** the pinned version in `mise/mise*.toml` is rewritten to the new version.
- [ ] `haven pkg upgrade` (no arg) upgrades across all allowed backends; brew path leaves Brewfiles unchanged.
- [ ] `haven pkg search ripgrep` shows grouped brew + mise matches with copy-pasteable install hints.
- [ ] `haven upgrade` (binary self-update) still works and is clearly distinct from `haven pkg upgrade`.
- [ ] Tests: mise pin-rewrite verified at the config-file level; output parsing covered; binary-missing path covered.
- [ ] clippy clean; docs/CLAUDE.md updated for the three new verbs.

## Out of Scope

Brewfile version pinning, interactive search UI, surfacing outdated state inside `status`/`diff`.

## Open Questions

- Confirm the exact mise subcommand/flags for (a) writing the bumped pin to a specific config file and
  (b) listing the available-tool registry for `search`. Resolve during implementation against the
  installed mise version.
