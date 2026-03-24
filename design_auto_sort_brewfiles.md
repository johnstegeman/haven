# Auto-Sort Brewfiles: Design Document

## Summary

Add an optional configuration to automatically sort Brewfile formulas alphabetically when the user enables auto-sorting.

---

## Motivation

**Problem:** Unsorted Brewfiles can make it hard to:
- Spot missing dependencies (no obvious gaps)
- See where a new formula should be added
- Quickly scan for duplicate formulas

**Reference:** Telemetry entry Q000001 raised this question.

---

## Design Options

### Option A: Opt-in Auto-Sort (Recommended)

**Configuration:**
```toml
[brewfiles]
sort = "on"  # or "off" (default)
```

**Behavior:**
- When enabled, `haven apply` re-sorts formulas after `brew bundle dump`
- Formulas are sorted by their name (short name without version prefix)
- Comments and anchors are preserved in place

**Pros:**
- Simple, predictable behavior
- Easy to opt-out per-module
- Minimal complexity

**Cons:**
- Requires explicit opt-in
- May need to migrate existing Brewfiles

### Option B: Smart Re-Sort

**Behavior:**
- Same as Option A, but preserves formatting (tabs vs spaces, line length)
- Anchors (`^anchor`) maintain position but are noted in comments
- Comments referencing specific formulas move with them

**Pros:**
- Respects user's style preferences
- Anchors remain valid after re-sort

**Cons:**
- More complex implementation
- Edge cases around comment preservation

### Option C: Hybrid (Per-Module)

**Configuration:**
```toml
[[module]]
name = "packages"
sort = "on"

[[module]]
name = "my-custom-brew"
sort = "off"
```

**Pros:**
- Fine-grained control
- Can mix sorted and unsorted Brewfiles

**Cons:**
- More configuration options
- Users might not need this level of control

---

## Recommended Implementation

**Start with Option A** — simple opt-in sorting.

**Phase 1:** Basic sort after `brew bundle dump`
**Phase 2:** Add comment preservation (if feasible)
**Phase 3:** Per-module control (if requested)

---

## Implementation Notes

### Where to integrate

The sorting should happen in `brew::dump()` in `src/lib.rs`, after the `brew bundle dump` command:

```rust
pub fn dump(path: &Path, output: &str) -> Result<()> {
    // 1. Write to Brewfile.tmp
    // 2. Run "brew bundle dump --file=Brewfile"
    // 3. If sort is enabled:
    //    - Read Brewfile.tmp
    //    - Extract formulas and comments
    //    - Sort formulas by name
    //    - Reconstruct with comments in place
    // 4. Replace original Brewfile
}
```

### Handling comments and anchors

Brewfile examples:
```ruby
# This is a comment
^anchor: some-anchor

homebrew/cask-fonts # @homebrew/cask-fonts
```

On sort:
- Keep `^anchor:` lines exactly where they are
- Move `homebrew/cask-fonts` to its sorted position
- Keep comments with their formulas
- Orphaned comments could become: `# (moved from line X)`

---

## UX Considerations

1. **Migration path:** When enabling auto-sort, suggest running `haven apply --remove-unreferenced-brews` to clean up first.

2. **Dry-run mode:** `haven apply --dry-run --sort-brews` to preview the sorting effect.

3. **Undo:** Keep a backup of the unsorted version temporarily; offer `haven undo --brews` to revert.

4. **Documentation:** Add note to `COMMANDS.md` about the sort option when enabled.

---

## Acceptance Criteria

- [ ] Config option works correctly
- [ ] Formulas are sorted by short name (without `homebrew/tap/` prefix)
- [ ] Comments are preserved (either attached to formulas or marked as moved)
- [ ] Anchors remain valid after sort
- [ ] Existing unsorted Brewfiles still work (opt-in)
- [ ] Unit tests cover sorting logic

---

## Open Questions

1. Should we preserve the exact line width and formatting of the original Brewfile?
2. How to handle formulas that get added/removed between `brew bundle update` and `brew bundle dump`?
3. Should we provide a CLI flag `--sort-brews` for manual sorting?
