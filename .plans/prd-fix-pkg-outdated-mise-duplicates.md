# PRD: Fix duplicate mise entries in `haven pkg outdated`

## Overview

`haven pkg outdated` prints each mise-managed package multiple times. The brew
backend lists each package once; only mise duplicates. This makes the command's
output unusable for scanning what actually needs upgrading (GitHub issue #13).

Observed output:

```
==> mise outdated
  awscli  2.35.3 → 2.35.4
  uv  0.11.20 → 0.11.21
  awscli  2.35.3 → 2.35.4      <- repeated
  uv  0.11.20 → 0.11.21
  awscli  2.35.3 → 2.35.4
  kubectl  1.36.1 → 1.36.2     <- only one config surfaces kubectl
  ...
```

## Goals

- `haven pkg outdated` lists each outdated mise tool exactly once.
- The result is the **union** of outdated tools across all haven-managed
  `mise/mise*.toml` configs (so a tool only some configs declare, e.g.
  `kubectl`, still appears — once).
- Brew output and the existing skip-note / empty-state behavior are unchanged.

## Non-Goals

- Changing `haven pkg upgrade` or `haven pkg search` behavior. `upgrade`
  intentionally rewrites the version pin per config file; `search` does not loop
  over configs. Neither exhibits the duplication bug.
- Changing how `mise outdated` itself resolves config (mise merging the global
  `~/.config/mise/config.toml` into every invocation is expected behavior we
  work around, not something we can or should change).

## Requirements

### Functional Requirements

- REQ-F-001: When multiple `mise/mise*.toml` config files exist, each outdated
  mise tool appears exactly once in `haven pkg outdated` output.
- REQ-F-002: The de-duplicated list is the union across all config files,
  preserving first-seen order, so tools declared only in a subset of configs are
  still reported.
- REQ-F-003: De-duplication is keyed on tool name. (Versions come from global
  installed/registry state, so the same name always carries the same
  current/latest versions.)
- REQ-F-004: Existing error and empty-state handling is preserved — a missing
  mise binary prints the skip note and continues; no outdated tools prints
  `mise: nothing outdated`; the `==> mise outdated` header prints only when at
  least one tool is outdated.

### Non-Functional Requirements

- REQ-NF-001: The de-duplication logic is a pure, separately unit-testable
  function (no subprocess I/O), consistent with the existing
  `parse_mise_outdated_json` testing pattern in `src/mise.rs`.

## Technical Considerations

Root cause: `outdated()` in `src/commands/pkg.rs` (the `"mise"` arm, ~lines
119–152) loops over every config from `all_misefiles()`
(`src/commands/mise.rs:99`) and calls `mise_outdated()` (`src/mise.rs:181`) once
per config, printing results immediately inside the loop. Because
`mise outdated` merges the global config into every invocation, shared tools are
returned — and printed — once per config file.

Fix shape:

- Add `mise_outdated_all(mise: &str, configs: &[PathBuf]) -> Result<Vec<OutdatedPackage>>`
  in `src/mise.rs` that runs `mise_outdated` per config, concatenates the
  results, and returns `dedupe_outdated(all)`.
- Add a pure helper `dedupe_outdated(Vec<OutdatedPackage>) -> Vec<OutdatedPackage>`
  that keeps the first occurrence of each tool name (e.g. via a
  `HashSet<String>` seen-set filter).
- Simplify the `"mise"` arm of `outdated()` to a single
  `mise_outdated_all` call, then print the returned list once with the same
  header/empty/error handling as today.

`OutdatedPackage` (`src/packages.rs`) is `Debug, Clone` with public `name`,
`current_version`, `latest_version` fields — sufficient for dedup; no derive
changes needed.

## Acceptance Criteria

- [ ] `dedupe_outdated` collapses repeated tool names to one entry, first-seen
      order, in a `#[cfg(test)]` unit test in `src/mise.rs`.
- [ ] `outdated()`'s mise arm calls `mise_outdated_all` once and prints each tool
      a single time.
- [ ] `cargo test` passes, including existing `parse_mise_outdated_json_*` tests
      and `pkg_outdated_missing_binaries_exits_success_with_skip_notes`
      (`tests/integration.rs:5129`).
- [ ] `cargo clippy --all-targets` is clean.

## Out of Scope

- Any change to `upgrade()` or `search()` in `src/commands/pkg.rs`.
- Reporting per-config provenance of outdated tools.

## Open Questions

None.
