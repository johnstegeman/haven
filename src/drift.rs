/// Drift detection between source files and their live destinations.
///
/// Extracted into a shared module so that both `status` and `diff` use the
/// same detection logic without duplication.
///
/// State machine:
///
///   SourceEntry
///        │
///        ├── plain file
///        │     ├── src missing  → SourceMissing
///        │     ├── dest missing → Missing
///        │     ├── bytes equal  → Clean
///        │     └── bytes differ → Modified
///        │
///        ├── template file
///        │     ├── src missing    → SourceMissing
///        │     ├── render fails   → Err (propagated)
///        │     ├── dest missing   → Missing
///        │     ├── rendered==dest → Clean
///        │     └── rendered!=dest → Modified
///        │
///        └── symlink file
///              ├── src missing          → SourceMissing
///              ├── dest correct symlink → Clean
///              ├── dest wrong target    → Modified
///              ├── dest regular file    → Modified
///              └── dest missing         → Missing
use anyhow::Result;
use std::path::Path;

use crate::template::TemplateContext;

/// The outcome of comparing a source entry against its live destination.
#[derive(Debug, Clone, PartialEq)]
pub enum DriftKind {
    /// Source and destination are in sync.
    Clean,
    /// Destination exists but differs from source.
    Modified,
    /// Destination does not exist (never applied or deleted).
    Missing,
    /// Source file is missing from the repo.
    SourceMissing,
}

/// Return the single-character marker for a non-clean drift kind.
pub fn drift_marker(kind: DriftKind) -> &'static str {
    match kind {
        DriftKind::Modified => "M",
        DriftKind::Missing => "?",
        DriftKind::SourceMissing => "!",
        DriftKind::Clean => unreachable!(),
    }
}

/// Like [`check_drift`] but strips haven-managed sections from the destination
/// text before comparing.
///
/// Use this for all plain files so that files augmented by haven after writing
/// (e.g. `~/.claude/CLAUDE.md`) are not falsely reported as modified.
/// For binary files (invalid UTF-8) it falls back to [`check_drift`].
pub fn check_drift_haven_aware(src: &Path, dest: &Path) -> DriftKind {
    if !src.exists() {
        return DriftKind::SourceMissing;
    }
    if !dest.exists() {
        return DriftKind::Missing;
    }
    let src_text = match std::fs::read_to_string(src) {
        Ok(t) => t,
        Err(_) => return check_drift(src, dest),
    };
    let dest_text = match std::fs::read_to_string(dest) {
        Ok(t) => t,
        Err(_) => return check_drift(src, dest),
    };
    let dest_stripped = crate::claude_md::strip_haven_section(&dest_text);
    if src_text == dest_stripped {
        DriftKind::Clean
    } else {
        DriftKind::Modified
    }
}

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

/// Check drift for a plain (non-template, non-symlink) file.
pub fn check_drift(src: &Path, dest: &Path) -> DriftKind {
    if !src.exists() {
        return DriftKind::SourceMissing;
    }
    if !dest.exists() {
        return DriftKind::Missing;
    }
    match (std::fs::read(src), std::fs::read(dest)) {
        (Ok(s), Ok(d)) if s == d => DriftKind::Clean,
        _ => DriftKind::Modified,
    }
}

/// Compare a template source against the live dest by rendering first.
///
/// Strips haven-managed sections from the destination before comparing so that
/// files augmented by haven after writing (e.g. via config injection) are not
/// falsely reported as modified — consistent with `check_drift_haven_aware` and
/// the idempotency check in `apply.rs`.
pub fn check_drift_template(src: &Path, ctx: &TemplateContext, dest: &Path) -> Result<DriftKind> {
    if !src.exists() {
        return Ok(DriftKind::SourceMissing);
    }
    if !dest.exists() {
        return Ok(DriftKind::Missing);
    }
    let source_text = std::fs::read_to_string(src)?;
    let rendered = crate::template::render(&source_text, ctx)?;
    let dest_bytes = std::fs::read(dest)?;
    match std::str::from_utf8(&dest_bytes) {
        Ok(dest_text) => {
            let stripped = crate::claude_md::strip_haven_section(dest_text);
            if rendered == stripped {
                Ok(DriftKind::Clean)
            } else {
                Ok(DriftKind::Modified)
            }
        }
        Err(_) => {
            if rendered.as_bytes() == dest_bytes.as_slice() {
                Ok(DriftKind::Clean)
            } else {
                Ok(DriftKind::Modified)
            }
        }
    }
}

/// Check drift for a linked (symlink) file entry.
///
/// Clean only when dest is a symlink whose target exactly matches `source_abs`.
/// Modified when dest exists but is the wrong kind or points elsewhere.
pub fn check_drift_link(source_abs: &Path, dest: &Path) -> DriftKind {
    if !source_abs.exists() {
        return DriftKind::SourceMissing;
    }
    if dest.is_symlink() {
        if let Ok(target) = std::fs::read_link(dest) {
            if target == source_abs {
                return DriftKind::Clean;
            }
        }
        DriftKind::Modified // wrong target or dangling symlink
    } else if dest.exists() {
        DriftKind::Modified // regular file where symlink expected
    } else {
        DriftKind::Missing
    }
}

/// Check drift for a symlink whose target path is stored as a template in `src`.
///
/// Renders `src` content through Tera to obtain the expected symlink target, then
/// compares it to the actual target of `dest`.
pub fn check_drift_link_template(
    src: &Path,
    ctx: &TemplateContext,
    dest: &Path,
) -> Result<DriftKind> {
    if !src.exists() {
        return Ok(DriftKind::SourceMissing);
    }
    let source_text = std::fs::read_to_string(src)?;
    let rendered = crate::template::render(&source_text, ctx)?;
    let expected_target = std::path::PathBuf::from(rendered.trim());
    if dest.is_symlink() {
        if let Ok(target) = std::fs::read_link(dest) {
            if target == expected_target {
                return Ok(DriftKind::Clean);
            }
        }
        Ok(DriftKind::Modified)
    } else if dest.exists() {
        Ok(DriftKind::Modified)
    } else {
        Ok(DriftKind::Missing)
    }
}

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
