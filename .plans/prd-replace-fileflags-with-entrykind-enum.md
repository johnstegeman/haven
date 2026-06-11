# PRD: Replace FileFlags with EntryKind Enum

## Overview

`FileFlags` is a flat boolean struct with 8 fields attached to every `SourceEntry`. Four of
those fields (`extdir`, `extfile`, `symlink`, and the implicit plain-file case) are mutually
exclusive _kinds_ that determine the routing logic in apply, status, diff, and list. Having
them as unguarded booleans allows illegal combinations the type system cannot prevent and
forces callers to write cascading `if/else if` chains where the compiler cannot verify
exhaustiveness. This refactor replaces the kind-determining flags with a sealed `EntryKind`
enum, inlines the composable modifier flags directly on `SourceEntry`, and simplifies
`SourceDir` to just its two meaningful fields.

## Goals

- Replace the four mutually-exclusive kind flags with a sealed `EntryKind` enum so callers
  use exhaustive `match` instead of cascading `if/else`.
- Make illegal flag combinations (e.g. `extdir + symlink`) unrepresentable at the type level.
- Keep the composable modifier flags (`private`, `executable`, `template`, `create_only`) as
  direct boolean fields on `SourceEntry`.
- Simplify `SourceDir` to `{ dest_tilde, private, exact }` — the only two fields it uses.
- Remove `FileFlags` from the public API once all callers are migrated.

## Non-Goals

- Changing any observable behavior of `haven apply`, `haven status`, `haven diff`, or
  `haven list`.
- Adding new magic-name prefixes or new entry kinds.
- Changing the encoding/decoding logic in `decode_component` beyond mapping to the new types.

## Requirements

### Functional Requirements

- REQ-F-001: `source.rs` exports a new `EntryKind` enum with variants `PlainFile`, `Symlink`,
  `ExternalDir`, `ExternalFile`.
- REQ-F-002: `SourceEntry` replaces `flags: FileFlags` with `kind: EntryKind` and flat fields
  `private: bool`, `executable: bool`, `template: bool`, `create_only: bool`.
- REQ-F-003: `SourceDir` replaces `flags: FileFlags` with `private: bool` and `exact: bool`.
- REQ-F-004: `FileFlags` becomes private to `source.rs` (used only inside `decode_component` /
  `decode_path` as an internal intermediate).
- REQ-F-005: All callers of `SourceEntry.flags.*` are migrated to use `entry.kind` +
  `entry.<modifier>` before `FileFlags` is removed.
- REQ-F-006: The `exact` tag in `haven list` output is removed (it was always false on files;
  `exact` is a `SourceDir` concept, not an entry concept).

### Non-Functional Requirements

- REQ-NF-001: `cargo build` and `cargo test` pass with zero errors after every committed task.
- REQ-NF-002: `cargo test --test integration` passes unchanged — no integration test output
  changes.

## Technical Considerations

**Migration strategy — always-compilable steps:**
The transition uses a temporary dual representation: during steps 2 and 3 (caller migration),
`SourceEntry` holds BOTH the old `flags: FileFlags` (kept public for backward compat) AND the
new `kind: EntryKind` + modifier fields. This ensures each intermediate commit compiles. The
`flags` field is dropped only in task-004, after all callers are migrated.

**Callers (outside source.rs):**
- `src/commands/apply.rs` — 25 accesses across `collect_exact_dirs`, `apply_entry`,
  `print_dry_run_entry`, and state-cleanup.
- `src/commands/status.rs` — 8 accesses (drift routing + C-marker guard).
- `src/commands/diff.rs` — 5 accesses (drift routing).
- `src/commands/list.rs` — 8 accesses (tag generation). The `entry.flags.exact` access here
  was always false and is dropped.

**No other callers:** `add.rs`, `chezmoi.rs`, `drift.rs`, and `tests/integration.rs` do not
read `.flags.*`.

**`decode_component` internal change:** The function currently returns `(String, FileFlags)`.
After task-001 it will still return that internally; `decode_path` maps `FileFlags` to the
public `EntryKind` + modifier fields when constructing `SourceEntry`. `FileFlags` never
escapes `source.rs`.

## Acceptance Criteria

- [ ] `EntryKind` enum exists in `src/source.rs` with four variants.
- [ ] `SourceEntry` has `kind: EntryKind` and four flat boolean modifier fields; no `flags` field.
- [ ] `SourceDir` has `private: bool` and `exact: bool` instead of `flags: FileFlags`.
- [ ] `FileFlags` is private to `source.rs` (not `pub`).
- [ ] Zero `entry.flags.*` or `dir.flags.*` accesses remain outside `source.rs`.
- [ ] `cargo test` passes (all unit tests in `source.rs` updated).
- [ ] `cargo test --test integration` passes with no output changes.
- [ ] `haven list` no longer shows an `exact` tag on file entries.

## Out of Scope

- Adding new `EntryKind` variants or magic-name prefixes.
- Changing drift-check logic in `drift.rs`.
- Changing `encode_filename` (the encoder is not affected by this refactor).
- Modifying integration tests (they test behavior, not types).

## Open Questions

None — the design is fully resolved from codebase analysis.
