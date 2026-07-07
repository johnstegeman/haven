# Fix `haven status` mise-config false drift — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `haven status` recognize the global mise config's merged content as clean, the same way `haven diff` already does, by extracting the merge-aware comparison logic into two shared functions both commands call.

**Architecture:** Add `mise::expected_config_text` (computes the text `apply` would write to the mise-merged destination) and `drift::check_drift_mise_aware` (resolves `DriftKind` from that expected text, falling back to the existing raw comparison when it doesn't apply). Rewire `diff.rs`'s inline logic to call these, then wire `status.rs` to call them too.

**Tech Stack:** Rust, existing `anyhow`/`toml_edit` deps, `assert_cmd`/`tempfile`/`predicates` for tests (already in use).

**Spec:** `docs/superpowers/specs/2026-07-07-status-mise-drift-fix-design.md`

---

### Task 1: Add `mise::expected_config_text`

**Files:**
- Modify: `src/mise.rs` (insert new function after `active_mise_config_paths`, which ends at line 423, before `load_or_create_doc` at line 425)
- Modify: `src/mise.rs` tests module (starts line 435-436)

- [ ] **Step 1: Write the failing unit tests**

Add to the `mod tests` block in `src/mise.rs` (after the existing tests, before the closing `}` of the module):

```rust
    #[test]
    fn expected_config_text_none_when_no_mise_config_paths() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("config.toml");
        std::fs::write(&src, "[settings]\n").unwrap();
        let dest = dir.path().join("dest.toml");

        let result = expected_config_text(&[], Some(dest.as_path()), &src, &dest);

        assert_eq!(result, None);
    }

    #[test]
    fn expected_config_text_none_when_dest_is_not_global_mise_path() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("config.toml");
        std::fs::write(&src, "[settings]\n").unwrap();
        let module_cfg = dir.path().join("mise.module.toml");
        std::fs::write(&module_cfg, "[tools]\nnode = \"22\"\n").unwrap();
        let dest = dir.path().join("dest.toml");
        let other_global = dir.path().join("other.toml");

        let result =
            expected_config_text(&[module_cfg], Some(other_global.as_path()), &src, &dest);

        assert_eq!(result, None);
    }

    #[test]
    fn expected_config_text_merges_when_dest_matches_global_path() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("config.toml");
        std::fs::write(&src, "[settings]\nfoo = 1\n").unwrap();
        let module_cfg = dir.path().join("mise.module.toml");
        std::fs::write(&module_cfg, "[tools]\nnode = \"22\"\n").unwrap();
        let dest = dir.path().join("dest.toml");

        let result = expected_config_text(&[module_cfg], Some(dest.as_path()), &src, &dest)
            .expect("expected merged text");

        assert!(result.contains("node = \"22\""));
        assert!(result.contains("foo = 1"));
    }

    #[test]
    fn expected_config_text_none_when_src_unreadable() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("does-not-exist.toml");
        let module_cfg = dir.path().join("mise.module.toml");
        std::fs::write(&module_cfg, "[tools]\nnode = \"22\"\n").unwrap();
        let dest = dir.path().join("dest.toml");

        let result = expected_config_text(&[module_cfg], Some(dest.as_path()), &src, &dest);

        assert_eq!(result, None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib mise::tests::expected_config_text -- --nocapture`
Expected: FAIL with `cannot find function 'expected_config_text' in this scope`

- [ ] **Step 3: Implement `expected_config_text`**

Insert into `src/mise.rs` immediately after `active_mise_config_paths` (after the closing `}` on line 423, before the doc comment for `load_or_create_doc`):

```rust
/// Compute the text `apply` would actually write to `dest`, when `dest` is
/// the global mise config and at least one active module declares `[tools]`.
///
/// Returns `None` when `dest` isn't the mise-merged destination (or no
/// active module declares tools) — callers should fall back to comparing
/// `src` and `dest` directly in that case. Shared by `haven diff` and
/// `haven status` so both compare against the same merged content `apply`
/// produces, instead of raw source, avoiding a false "modified" report.
pub fn expected_config_text(
    mise_config_paths: &[PathBuf],
    mise_global_path: Option<&Path>,
    src: &Path,
    dest: &Path,
) -> Option<String> {
    if mise_config_paths.is_empty() {
        return None;
    }
    if mise_global_path != Some(dest) {
        return None;
    }
    let base = std::fs::read_to_string(src).ok()?;
    merge_tools_into_text(mise_config_paths, &base).ok()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib mise::tests::expected_config_text`
Expected: PASS (4 tests)

- [ ] **Step 5: Commit**

```bash
git add src/mise.rs
git commit -m "$(cat <<'EOF'
refactor: extract mise::expected_config_text for merge-aware comparison

Pulls the merge-aware "what would apply write here" computation out
of diff.rs so status.rs can reuse it in a later commit, instead of
duplicating the logic and risking the same drift between the two
commands that caused the status false-positive bug.
EOF
)"
```

---

### Task 2: Add `drift::check_drift_mise_aware`

**Files:**
- Modify: `src/drift.rs` (insert new function after `check_drift_haven_aware`, which ends at line 84, before `check_drift` at line 86)
- Modify: `src/drift.rs` (add a new `#[cfg(test)] mod tests` block at the end of the file — none exists yet)

- [ ] **Step 1: Write the failing unit tests**

Append to the end of `src/drift.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn check_drift_mise_aware_source_missing_when_expected_given_but_src_absent() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("does-not-exist.toml");
        let dest = dir.path().join("dest.toml");
        std::fs::write(&dest, "[tools]\nnode = \"22\"\n").unwrap();

        let kind = check_drift_mise_aware(&src, &dest, Some("[tools]\nnode = \"22\"\n"));

        assert_eq!(kind, DriftKind::SourceMissing);
    }

    #[test]
    fn check_drift_mise_aware_missing_when_expected_given_but_dest_absent() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("config.toml");
        std::fs::write(&src, "[settings]\n").unwrap();
        let dest = dir.path().join("dest.toml");

        let kind = check_drift_mise_aware(&src, &dest, Some("[tools]\nnode = \"22\"\n"));

        assert_eq!(kind, DriftKind::Missing);
    }

    #[test]
    fn check_drift_mise_aware_clean_when_dest_matches_expected() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("config.toml");
        std::fs::write(&src, "[settings]\n").unwrap();
        let dest = dir.path().join("dest.toml");
        std::fs::write(&dest, "[tools]\nnode = \"22\"\n").unwrap();

        let kind = check_drift_mise_aware(&src, &dest, Some("[tools]\nnode = \"22\"\n"));

        assert_eq!(kind, DriftKind::Clean);
    }

    #[test]
    fn check_drift_mise_aware_modified_when_dest_differs_from_expected() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("config.toml");
        std::fs::write(&src, "[settings]\n").unwrap();
        let dest = dir.path().join("dest.toml");
        std::fs::write(&dest, "[tools]\nnode = \"20\"\n").unwrap();

        let kind = check_drift_mise_aware(&src, &dest, Some("[tools]\nnode = \"22\"\n"));

        assert_eq!(kind, DriftKind::Modified);
    }

    #[test]
    fn check_drift_mise_aware_falls_back_to_haven_aware_when_no_expected() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("file.txt");
        std::fs::write(&src, "same\n").unwrap();
        let dest = dir.path().join("dest.txt");
        std::fs::write(&dest, "same\n").unwrap();

        let kind = check_drift_mise_aware(&src, &dest, None);

        assert_eq!(kind, DriftKind::Clean);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib drift::tests -- --nocapture`
Expected: FAIL with `cannot find function 'check_drift_mise_aware' in this scope`

- [ ] **Step 3: Implement `check_drift_mise_aware`**

Insert into `src/drift.rs` immediately after `check_drift_haven_aware` (after its closing `}` on line 84, before the doc comment for `check_drift` on line 86):

```rust
/// Like [`check_drift_haven_aware`], but for entries where the destination is
/// produced by merging `src` with extra content (currently: the global mise
/// config merged with active modules' `[tools]`).
///
/// When `expected` is `Some`, compares `src`/`dest` existence and exact text
/// equality against it directly — `dest` is expected to differ from raw
/// `src`, so no haven-managed-section stripping applies here. When `expected`
/// is `None`, falls back to [`check_drift_haven_aware`] for the normal case.
pub fn check_drift_mise_aware(src: &Path, dest: &Path, expected: Option<&str>) -> DriftKind {
    let Some(expected) = expected else {
        return check_drift_haven_aware(src, dest);
    };
    if !src.exists() {
        return DriftKind::SourceMissing;
    }
    if !dest.exists() {
        return DriftKind::Missing;
    }
    match std::fs::read_to_string(dest) {
        Ok(dest_text) if dest_text == expected => DriftKind::Clean,
        _ => DriftKind::Modified,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib drift::tests`
Expected: PASS (5 tests)

- [ ] **Step 5: Commit**

```bash
git add src/drift.rs
git commit -m "$(cat <<'EOF'
refactor: extract drift::check_drift_mise_aware for merge-aware drift

Pulls the DriftKind decision that diff.rs makes for the mise-merged
destination into a shared function, matching expected_config_text
from the prior commit, so status.rs can reuse both in a later commit.
EOF
)"
```

---

### Task 3: Rewire `diff.rs` to use the shared functions

**Files:**
- Modify: `src/commands/diff.rs:254-284` (replace inline `is_mise_global`/`expected_src_text`/`kind` computation)
- Modify: `src/commands/diff.rs:36-39` (drop now-unused `check_drift_haven_aware` import)

- [ ] **Step 1: Replace the inline computation**

In `src/commands/diff.rs`, replace this block (currently lines 254–284):

```rust
                        // If this entry is the global mise config and at least one
                        // active module declares mise tools, the destination `apply`
                        // produces is source merged with `[tools]` — compute that
                        // expected text instead of comparing raw source to dest.
                        let is_mise_global = !mise_config_paths.is_empty()
                            && mise_global_path.as_deref() == Some(dest.as_path());
                        let expected_src_text = if is_mise_global {
                            std::fs::read_to_string(&entry.src)
                                .ok()
                                .and_then(|base| {
                                    crate::mise::merge_tools_into_text(&mise_config_paths, &base)
                                        .ok()
                                })
                        } else {
                            None
                        };

                        let kind = if let Some(expected) = &expected_src_text {
                            if !entry.src.exists() {
                                DriftKind::SourceMissing
                            } else if !dest.exists() {
                                DriftKind::Missing
                            } else {
                                match std::fs::read_to_string(&dest) {
                                    Ok(dest_text) if &dest_text == expected => DriftKind::Clean,
                                    _ => DriftKind::Modified,
                                }
                            }
                        } else {
                            check_drift_haven_aware(&entry.src, &dest)
                        };
```

with:

```rust
                        // If this entry is the global mise config and at least one
                        // active module declares mise tools, the destination `apply`
                        // produces is source merged with `[tools]` — compute that
                        // expected text instead of comparing raw source to dest.
                        let expected_src_text = crate::mise::expected_config_text(
                            &mise_config_paths,
                            mise_global_path.as_deref(),
                            &entry.src,
                            &dest,
                        );

                        let kind = check_drift_mise_aware(
                            &entry.src,
                            &dest,
                            expected_src_text.as_deref(),
                        );
```

- [ ] **Step 2: Update imports**

In `src/commands/diff.rs`, replace the `crate::drift` import block:

```rust
use crate::drift::{
    check_drift_haven_aware, check_drift_link, check_drift_link_template, check_drift_template,
    DriftKind,
};
```

with:

```rust
use crate::drift::{
    check_drift_link, check_drift_link_template, check_drift_mise_aware, check_drift_template,
    DriftKind,
};
```

- [ ] **Step 3: Build and confirm no warnings**

Run: `cargo build 2>&1 | grep -E "warning|error"`
Expected: no output (no unused-import warnings, no errors)

- [ ] **Step 4: Run existing diff tests to confirm no regression**

Run: `cargo test --test integration diff_`
Expected: PASS, including `diff_no_false_drift_after_mise_merge`

- [ ] **Step 5: Commit**

```bash
git add src/commands/diff.rs
git commit -m "$(cat <<'EOF'
refactor: rewire haven diff to use shared mise-aware drift helpers

No behavior change — diff.rs now calls mise::expected_config_text and
drift::check_drift_mise_aware instead of the equivalent inline logic,
so status.rs can reuse the same code path in the next commit.
EOF
)"
```

---

### Task 4: Wire `status.rs` to use the shared functions

**Files:**
- Modify: `src/commands/status.rs:1-14` (imports)
- Modify: `src/commands/status.rs:53-96` (precompute mise paths, use shared functions in the `PlainFile` branch)

- [ ] **Step 1: Update imports**

In `src/commands/status.rs`, replace:

```rust
use crate::drift::{
    check_drift_haven_aware, check_drift_link, check_drift_link_template, check_drift_template,
    drift_marker, DriftKind,
};
```

with:

```rust
use crate::drift::{
    check_drift_link, check_drift_link_template, check_drift_mise_aware, check_drift_template,
    drift_marker, DriftKind,
};
```

- [ ] **Step 2: Precompute mise paths before the file loop**

In `src/commands/status.rs`, the file-drift section currently starts (lines 54–59):

```rust
    if show_files {
        let source_dir = opts.repo_root.join("source");
        let ignore = IgnoreList::load(opts.repo_root, &template_ctx);
        let entries = source::scan(&source_dir, &ignore)?;
        source::warn_duplicate_destinations(&entries);

        let mut file_drift: Vec<(String, String)> = Vec::new();
```

Replace it with:

```rust
    if show_files {
        let source_dir = opts.repo_root.join("source");
        let ignore = IgnoreList::load(opts.repo_root, &template_ctx);
        let entries = source::scan(&source_dir, &ignore)?;
        source::warn_duplicate_destinations(&entries);

        // The global mise config's destination is written by `apply` as the
        // bare source content merged with `[tools]` from active modules'
        // mise configs — it is never a byte-for-byte copy of source. Precompute
        // the expected merge inputs here so the file loop below can compare
        // against what `apply` would actually produce, instead of raw source,
        // avoiding a false "modified" report on every status check.
        let mise_global_path = crate::mise::mise_global_config_path().ok();
        let mise_config_paths: Vec<PathBuf> =
            crate::mise::active_mise_config_paths(opts.repo_root, &sorted);

        let mut file_drift: Vec<(String, String)> = Vec::new();
```

- [ ] **Step 3: Use the shared functions in the `PlainFile` branch**

In `src/commands/status.rs`, replace:

```rust
                EntryKind::PlainFile => {
                    if entry.template {
                        check_drift_template(&entry.src, &template_ctx, &dest)?
                    } else {
                        check_drift_haven_aware(&entry.src, &dest)
                    }
                }
```

with:

```rust
                EntryKind::PlainFile => {
                    if entry.template {
                        check_drift_template(&entry.src, &template_ctx, &dest)?
                    } else {
                        let expected = crate::mise::expected_config_text(
                            &mise_config_paths,
                            mise_global_path.as_deref(),
                            &entry.src,
                            &dest,
                        );
                        check_drift_mise_aware(&entry.src, &dest, expected.as_deref())
                    }
                }
```

- [ ] **Step 4: Build and confirm no warnings**

Run: `cargo build 2>&1 | grep -E "warning|error"`
Expected: no output

Note: `PathBuf` is already imported in `src/commands/status.rs` (`use std::path::{Path, PathBuf};` at line 2) and `sorted` is already computed at line 45 (`let sorted = sort_modules(&modules);`) before the `show_files` block — no additional imports needed.

- [ ] **Step 5: Commit**

```bash
git add src/commands/status.rs
git commit -m "$(cat <<'EOF'
fix: status false drift on mise config that apply merged with [tools]

haven status compared the global mise config's destination against
raw source, but apply always merges in [tools] from active modules —
so status reported it modified even right after a clean apply. Same
bug diff had before a134ded3; status now uses the same merge-aware
comparison via the shared mise::expected_config_text /
drift::check_drift_mise_aware helpers.
EOF
)"
```

---

### Task 5: Integration test for the fix

**Files:**
- Modify: `tests/integration.rs` (add new test near `diff_no_false_drift_after_mise_merge`, which ends at line 5515)

- [ ] **Step 1: Write the failing integration test**

Add to `tests/integration.rs` immediately after `diff_no_false_drift_after_mise_merge` (after its closing `}` at line 5515):

```rust

#[test]
fn status_no_false_drift_after_mise_merge() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    cmd(&repo).arg("init").assert().success();

    fs::create_dir_all(repo.path().join("mise")).unwrap();
    fs::write(
        repo.path().join("mise").join("mise.packages.toml"),
        "[tools]\nnode = \"22\"\n",
    )
    .unwrap();

    fs::write(
        repo.path().join("modules").join("packages.toml"),
        "[mise]\nconfig = \"mise/mise.packages.toml\"\n",
    )
    .unwrap();

    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = [\"packages\"]\n",
    )
    .unwrap();

    let source_mise_dir = repo.path().join("source").join("dot_config").join("mise");
    fs::create_dir_all(&source_mise_dir).unwrap();
    fs::write(
        source_mise_dir.join("config.toml"),
        "[settings]\n# base config\n",
    )
    .unwrap();

    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default", "--files"])
        .assert()
        .success();

    // The global mise config now contains merged [tools] that the bare
    // source never had. `haven status` should recognize this as the
    // expected, apply-produced state — not report it as drift.
    cmd_home(&repo, &home)
        .args(["status", "--profile", "default", "--files"])
        .assert()
        .success()
        .stdout(predicate::str::contains("up to date"))
        .stdout(predicate::str::contains("mise/config.toml").not());
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test integration status_no_false_drift_after_mise_merge`
Expected: FAIL — stdout contains `M .config/mise/config.toml` instead of "up to date"

- [ ] **Step 3: Verify the test passes (implementation already done in Task 4)**

Run: `cargo test --test integration status_no_false_drift_after_mise_merge`
Expected: PASS

- [ ] **Step 4: Run the full test suite**

Run: `cargo test`
Expected: all tests PASS (no regressions in `status_*` or `diff_*` tests)

- [ ] **Step 5: Commit**

```bash
git add tests/integration.rs
git commit -m "$(cat <<'EOF'
test: regression test for status mise-config false drift fix

Mirrors diff_no_false_drift_after_mise_merge for the status command.
EOF
)"
```

---

## Self-Review Notes

- **Spec coverage:** All 4 spec items (mise.rs helper, drift.rs helper, diff.rs rewire, status.rs wiring) map to Tasks 1–4; spec's "Testing" section maps to Task 5.
- **Type consistency:** `expected_config_text(&[PathBuf], Option<&Path>, &Path, &Path) -> Option<String>` and `check_drift_mise_aware(&Path, &Path, Option<&str>) -> DriftKind` signatures are used identically in Tasks 1–4.
- **No behavior change to `diff`:** Task 3 is a pure refactor — `diff_no_false_drift_after_mise_merge` (existing test) must still pass unchanged.
