/// Utilities for generating and formatting unified diffs.
use similar::{ChangeTag, TextDiff};

/// Compute a unified diff between `a` (expected/source) and `b` (actual/dest).
///
/// Returns `None` when the strings are identical.
/// `label_a` and `label_b` appear on the `---` / `+++` header lines.
pub fn unified_diff(
    a: &str,
    b: &str,
    label_a: &str,
    label_b: &str,
    context_lines: usize,
) -> Option<String> {
    if a == b {
        return None;
    }
    let diff = TextDiff::from_lines(a, b);
    let mut out = String::new();
    out.push_str(&format!("--- {}\n", label_a));
    out.push_str(&format!("+++ {}\n", label_b));
    for group in diff.grouped_ops(context_lines) {
        // @@ hunk header
        let first = &group[0];
        let last = &group[group.len() - 1];
        let old_start = first.old_range().start + 1;
        let old_len: usize = group.iter().map(|op| op.old_range().len()).sum();
        let new_start = first.new_range().start + 1;
        let new_len: usize = group.iter().map(|op| op.new_range().len()).sum();
        let _ = last; // used via group iteration above
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_start, old_len, new_start, new_len
        ));
        for op in &group {
            for change in diff.iter_changes(op) {
                let prefix = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                out.push_str(prefix);
                out.push_str(change.value());
                if change.missing_newline() {
                    out.push('\n');
                }
            }
        }
    }
    if out.lines().count() <= 2 {
        // Only the --- / +++ headers, no hunks — shouldn't happen if a != b, but guard.
        return None;
    }
    Some(out)
}

/// Apply ANSI color codes to a unified diff string.
///
/// - `+` lines      → green
/// - `-` lines      → red
/// - `@@` lines     → cyan
/// - `---` / `+++`  → bold
///
/// No-op on empty input. Safe to call on non-diff text (unrecognised lines pass through).
pub fn colorize_diff(diff: &str) -> String {
    const GREEN: &str = "\x1b[32m";
    const RED: &str = "\x1b[31m";
    const CYAN: &str = "\x1b[36m";
    const BOLD: &str = "\x1b[1m";
    const RESET: &str = "\x1b[0m";

    if diff.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(diff.len() + diff.lines().count() * 8);
    for line in diff.lines() {
        if line.starts_with("---") || line.starts_with("+++") {
            out.push_str(BOLD);
            out.push_str(line);
            out.push_str(RESET);
        } else if line.starts_with('+') {
            out.push_str(GREEN);
            out.push_str(line);
            out.push_str(RESET);
        } else if line.starts_with('-') {
            out.push_str(RED);
            out.push_str(line);
            out.push_str(RESET);
        } else if line.starts_with("@@") {
            out.push_str(CYAN);
            out.push_str(line);
            out.push_str(RESET);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Compute a `--stat` summary line for a file showing the count of added/removed lines.
///
/// Example: `"~/.gitconfig | 5 ++---"`
pub fn stat_line(path: &str, diff: &str) -> String {
    let mut adds = 0usize;
    let mut dels = 0usize;
    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            adds += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            dels += 1;
        }
    }
    let total = adds + dels;
    let bar = "+".repeat(adds) + &"-".repeat(dels);
    format!("{} | {} {}", path, total, bar)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_strings_returns_none() {
        assert!(unified_diff("hello\n", "hello\n", "a", "b", 3).is_none());
    }

    #[test]
    fn single_line_change_produces_diff() {
        let diff = unified_diff("hello\n", "world\n", "a", "b", 3).unwrap();
        assert!(diff.contains("-hello"));
        assert!(diff.contains("+world"));
    }

    #[test]
    fn added_lines_only() {
        let diff = unified_diff("", "new line\n", "a", "b", 3).unwrap();
        assert!(diff.contains("+new line"));
    }

    #[test]
    fn removed_lines_only() {
        let diff = unified_diff("old line\n", "", "a", "b", 3).unwrap();
        assert!(diff.contains("-old line"));
    }

    #[test]
    fn context_lines_respected() {
        let a = "ctx1\nctx2\nold\nctx3\nctx4\n";
        let b = "ctx1\nctx2\nnew\nctx3\nctx4\n";
        let diff = unified_diff(a, b, "a", "b", 1).unwrap();
        // With 1 context line only ctx2 and ctx3 appear (not ctx1/ctx4).
        assert!(diff.contains("ctx2"));
        assert!(diff.contains("ctx3"));
        assert!(!diff.contains("ctx1\n ctx2")); // ctx1 not in the hunk
    }

    #[test]
    fn empty_vs_nonempty() {
        let diff = unified_diff("", "a\nb\n", "a", "b", 3).unwrap();
        assert!(diff.contains("+a"));
        assert!(diff.contains("+b"));
    }

    #[test]
    fn multiline_file_one_change_shows_hunk() {
        let a = "line1\nline2\nline3\nline4\nline5\n";
        let b = "line1\nLINE2\nline3\nline4\nline5\n";
        let diff = unified_diff(a, b, "a", "b", 3).unwrap();
        assert!(diff.contains("@@"));
        assert!(diff.contains("-line2"));
        assert!(diff.contains("+LINE2"));
    }

    #[test]
    fn colorize_diff_adds_green_to_plus_lines() {
        let diff = "+added line\n";
        let colored = colorize_diff(diff);
        assert!(colored.contains("\x1b[32m+added line"));
    }

    #[test]
    fn colorize_diff_adds_red_to_minus_lines() {
        let diff = "-removed line\n";
        let colored = colorize_diff(diff);
        assert!(colored.contains("\x1b[31m-removed line"));
    }

    #[test]
    fn colorize_diff_noop_on_empty() {
        assert_eq!(colorize_diff(""), "");
    }

    #[test]
    fn colorize_diff_bold_for_headers() {
        let diff = "--- a\n+++ b\n";
        let colored = colorize_diff(diff);
        assert!(colored.contains("\x1b[1m---"));
        assert!(colored.contains("\x1b[1m+++"));
    }

    #[test]
    fn stat_line_shows_plus_minus_counts() {
        let diff = "--- a\n+++ b\n+added\n+another\n-removed\n";
        let line = stat_line("~/.gitconfig", diff);
        assert_eq!(line, "~/.gitconfig | 3 ++-");
    }

    #[test]
    fn stat_line_zero_changes() {
        let line = stat_line("~/.zshrc", "");
        assert_eq!(line, "~/.zshrc | 0 ");
    }
}
