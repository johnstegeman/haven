# Fix: `haven status` false drift on merged mise config

## Problem

`haven apply` writes `~/.config/mise/config.toml` as the checked-in source
merged with `[tools]` from each active module's mise config — it is never a
byte-for-byte copy of source. `haven diff` learned this in `a134ded3` (2026-07-06):
it now computes the expected merged text and compares the destination against
that, instead of raw source.

`haven status` never received the equivalent fix. `src/commands/status.rs`
still runs `check_drift_haven_aware(&entry.src, &dest)` directly on every
`EntryKind::PlainFile`, including the mise global config. Since the
destination legitimately differs from raw source (by design), `status`
always reports `.config/mise/config.toml` as modified, even immediately
after a clean apply.

## Root cause

The merge-aware comparison logic added for `diff` was implemented inline in
`src/commands/diff.rs` only, rather than as a shared function both `diff` and
`status` could call. `status` was left on the old raw-comparison path.

## Fix

Extract the merge-aware comparison into two small shared functions so `diff`
and `status` go through identical logic, rather than fixing `status` with a
second inline copy that could drift from `diff`'s again.

### 1. `src/mise.rs`: `expected_config_text`

```rust
pub fn expected_config_text(
    mise_config_paths: &[PathBuf],
    mise_global_path: Option<&Path>,
    src: &Path,
    dest: &Path,
) -> Option<String>
```

Returns `None` unless `dest` is the global mise config *and* at least one
active module declares `[tools]`. Otherwise reads `src` and merges it with
`mise_config_paths` via the existing `merge_tools_into_text`, returning the
text `apply` would actually write to `dest`.

This is the `is_mise_global` / `expected_src_text` computation currently
inlined in `diff.rs`, lifted out unchanged in behavior.

### 2. `src/drift.rs`: `check_drift_mise_aware`

```rust
pub fn check_drift_mise_aware(src: &Path, dest: &Path, expected: Option<&str>) -> DriftKind
```

If `expected` is `Some`, resolve drift from src/dest existence and text
equality directly (mirrors the `kind` branch currently inlined in `diff.rs`).
If `expected` is `None`, fall back to the existing `check_drift_haven_aware`.

### 3. `src/commands/diff.rs`

Replace the inline `is_mise_global` / `expected_src_text` / `kind`
computation with calls to `mise::expected_config_text` and
`drift::check_drift_mise_aware`. The precomputed `mise_config_paths` /
`mise_global_path` (built once before the file loop) stay as-is. `diff`
still needs the expected text afterward to render the actual diff output, so
that value stays local to `diff.rs`.

### 4. `src/commands/status.rs`

Precompute `mise_global_path` and `mise_config_paths` once before the file
loop, the same way `diff.rs` does, using the `sorted` modules list `status`
already builds. In the `EntryKind::PlainFile` non-template branch, call
`mise::expected_config_text(...)` then `drift::check_drift_mise_aware(...)`
instead of calling `check_drift_haven_aware` directly.

## Testing

Add an integration test in `tests/integration.rs` mirroring the existing
`diff_no_false_drift_after_mise_merge` test: set up a module with mise
`[tools]`, run `apply --files`, then run `status --files` and assert it
reports clean (no `M` marker for `.config/mise/config.toml`) instead of
drift.

## Non-goals / no behavior change

- No change to `diff`'s output or behavior — this is a refactor-and-reuse of
  its existing logic, not a change to it.
- No change to drift handling for any non-mise-global file.
- No change to the `C` marker (user-edited-since-apply) logic in `status.rs`,
  which is independent of this fix.
