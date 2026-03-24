---
name: telemetry_Q000001
description: Auto-sort brewfiles design proposal
type: project
---

**Feature:** Auto-sort brewfiles alphabetically by formula name

**Why:**
Q000001 raised the question of auto-sorting Brewfiles. Sorted Brewfiles make it easier to spot missing dependencies (gaps in the alphabetical sequence) and new formulas land in the right spot naturally.

**How to apply:**
Add an optional `[brewfiles]` section to `haven.toml` with `sort = "on"` (opt-in by default).

**Design options to evaluate:**
1. Simple: Re-sort formulas when writing Brewfile after `brew bundle dump`
2. Smart: Sort before/after installing, preserve comments/anchors
3. Hybrid: User can disable per-module in `haven.toml`

**Open questions:**
- How to handle comments and anchors during re-sort?
- Should we preserve the original file's formatting style?
- What if a formula name changes (should we migrate it)?
