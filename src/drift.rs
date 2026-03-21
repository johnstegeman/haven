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
pub fn check_drift_template(
    src: &Path,
    ctx: &TemplateContext,
    dest: &Path,
) -> Result<DriftKind> {
    if !src.exists() {
        return Ok(DriftKind::SourceMissing);
    }
    if !dest.exists() {
        return Ok(DriftKind::Missing);
    }
    let source_text = std::fs::read_to_string(src)?;
    let rendered = crate::template::render(&source_text, ctx)?;
    let dest_bytes = std::fs::read(dest)?;
    if rendered.as_bytes() == dest_bytes.as_slice() {
        Ok(DriftKind::Clean)
    } else {
        Ok(DriftKind::Modified)
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
