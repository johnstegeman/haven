/// Load and apply `config/ignore` patterns.
///
/// The ignore file lives at `<repo_root>/config/ignore` and uses gitignore-style glob patterns
/// matched against the **decoded destination path** (without the `~/` prefix).
///
/// ## Syntax
///
/// ```text
/// # comment
///
/// # Ignore a specific file
/// .zshrc
///
/// # Glob patterns
/// .ssh/id_*
/// .config/*/history
///
/// # Match everything under a directory
/// .local/share/some-app/**
///
/// # Negate a previous match
/// !.local/share/some-app/keep-this
/// ```
///
/// ## Pattern rules
///
/// - Empty lines and lines starting with `#` are skipped.
/// - `*`  matches any sequence of non-separator characters.
/// - `**` matches any sequence of characters including path separators.
/// - `?`  matches any single non-separator character.
/// - A leading `!` negates the pattern (un-ignores previously ignored paths).
/// - Patterns **without** a `/` are matched against the basename only (last path component).
/// - Patterns **with** a `/` are matched against the full decoded path from the home root.
use std::fs;
use std::path::Path;

use crate::template::{render_lenient, TemplateContext};

// ─── Public API ───────────────────────────────────────────────────────────────

/// A compiled list of ignore patterns loaded from `config/ignore`.
#[derive(Debug, Default)]
pub struct IgnoreList {
    patterns: Vec<CompiledPattern>,
}

#[derive(Debug)]
struct CompiledPattern {
    negate: bool,
    /// When true, match against the full path; when false, match against basename only.
    anchored: bool,
    raw: String,
}

impl IgnoreList {
    /// Load patterns from `<repo_root>/config/ignore`.
    ///
    /// The file is treated as a Tera template: it is rendered against the current
    /// machine context before the patterns are parsed. This allows the ignore file
    /// to use `{% if os == "macos" %}` and similar conditionals, matching how
    /// chezmoi treats `.chezmoiignore`.
    ///
    /// Returns an empty list (ignores nothing) if the file does not exist or if
    /// template rendering fails (failure is printed as a warning).
    pub fn load(repo_root: &Path, ctx: &TemplateContext) -> Self {
        let path = repo_root.join("config").join("ignore");
        let raw = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };
        // Render the file as a Tera template. render_lenient returns "" on error,
        // which produces an empty IgnoreList (ignore nothing — safe failure mode).
        let rendered = render_lenient(&raw, ctx);
        Self::from_str(&rendered)
    }

    /// Parse patterns from a string (used in tests and for load).
    pub fn from_str(content: &str) -> Self {
        let mut patterns = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (negate, raw) = if let Some(rest) = line.strip_prefix('!') {
                (true, rest.trim().to_string())
            } else {
                (false, line.to_string())
            };
            // A pattern is "anchored" (full-path match) when it contains a '/'.
            // A leading '/' just means anchored — strip it for matching.
            let raw = raw.trim_start_matches('/').to_string();
            let anchored = raw.contains('/');
            patterns.push(CompiledPattern { negate, anchored, raw });
        }
        Self { patterns }
    }

    /// Returns `true` if `dest_tilde` (e.g. `"~/.zshrc"`) should be ignored.
    pub fn is_ignored(&self, dest_tilde: &str) -> bool {
        if self.patterns.is_empty() {
            return false;
        }
        let path = dest_tilde.strip_prefix("~/").unwrap_or(dest_tilde);
        let mut ignored = false;
        for pat in &self.patterns {
            let hit = if pat.anchored {
                glob_matches(&pat.raw, path)
            } else {
                // No slash — match basename only.
                let basename = path.rsplit('/').next().unwrap_or(path);
                glob_matches(&pat.raw, basename)
            };
            if hit {
                ignored = !pat.negate;
            }
        }
        ignored
    }
}

// ─── Glob matching ────────────────────────────────────────────────────────────

/// Match `pattern` against `path` using `*` / `**` / `?` glob semantics.
///
/// - `*`  matches zero or more non-`/` characters.
/// - `**` matches zero or more characters including `/`.
/// - `?`  matches exactly one non-`/` character.
fn glob_matches(pattern: &str, path: &str) -> bool {
    glob_rec(pattern.as_bytes(), path.as_bytes())
}

fn glob_rec(pat: &[u8], s: &[u8]) -> bool {
    match (pat.split_first(), s.split_first()) {
        // Both exhausted — full match.
        (None, None) => true,

        // Pattern exhausted, string has remaining chars — no match.
        (None, Some(_)) => false,

        // `**` — matches zero or more chars including `/`.
        (Some((&b'*', rest_p)), _) if rest_p.first() == Some(&b'*') => {
            let after_double = &rest_p[1..];
            // Consume optional trailing `/` after `**`.
            let rest_pat = if after_double.first() == Some(&b'/') {
                &after_double[1..]
            } else {
                after_double
            };
            // Try matching zero characters (skip `**`).
            if glob_rec(rest_pat, s) {
                return true;
            }
            // Consume one character from s and retry (keeping `**` in pattern).
            match s.split_first() {
                Some((_, s_rest)) => glob_rec(pat, s_rest),
                None => false,
            }
        }

        // `*` — matches zero or more non-`/` characters.
        (Some((&b'*', rest_p)), _) => {
            // Try zero match.
            if glob_rec(rest_p, s) {
                return true;
            }
            // Consume one non-separator char and retry.
            match s.split_first() {
                Some((&c, s_rest)) if c != b'/' => glob_rec(pat, s_rest),
                _ => false,
            }
        }

        // `?` — matches exactly one non-`/` character.
        (Some((&b'?', rest_p)), Some((&c, s_rest))) if c != b'/' => glob_rec(rest_p, s_rest),
        (Some((&b'?', _)), _) => false,

        // Literal character must match exactly.
        (Some((&p, rest_p)), Some((&c, s_rest))) => {
            if p == c {
                glob_rec(rest_p, s_rest)
            } else {
                false
            }
        }

        // String exhausted but pattern has remaining (non-star) chars.
        (Some(_), None) => false,
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ignored(patterns: &str, dest: &str) -> bool {
        IgnoreList::from_str(patterns).is_ignored(dest)
    }

    // ── Glob matcher unit tests ────────────────────────────────────────────────

    #[test]
    fn glob_exact_match() {
        assert!(glob_matches(".zshrc", ".zshrc"));
        assert!(!glob_matches(".zshrc", ".bashrc"));
    }

    #[test]
    fn glob_star_matches_non_sep() {
        assert!(glob_matches("*.zsh", "foo.zsh"));
        assert!(glob_matches("*.zsh", ".zsh")); // star matches zero chars
        assert!(!glob_matches("*.zsh", "foo/bar.zsh")); // star does not cross /
    }

    #[test]
    fn glob_double_star_crosses_sep() {
        assert!(glob_matches("**/.zshrc", ".zshrc"));
        assert!(glob_matches("**/.zshrc", "a/b/.zshrc"));
        assert!(glob_matches(".config/**", ".config/git/config"));
        assert!(glob_matches(".config/**", ".config/nvim/init.lua"));
    }

    #[test]
    fn glob_question_mark() {
        assert!(glob_matches("id_?sa", "id_rsa"));
        assert!(glob_matches("id_?sa", "id_dsa"));
        assert!(!glob_matches("id_?sa", "id_/sa")); // ? does not match /
    }

    // ── IgnoreList tests ───────────────────────────────────────────────────────

    #[test]
    fn empty_list_ignores_nothing() {
        assert!(!ignored("", "~/.zshrc"));
    }

    #[test]
    fn comment_and_blank_lines_skipped() {
        assert!(!ignored("# comment\n\n# another", "~/.zshrc"));
    }

    #[test]
    fn exact_basename_match() {
        assert!(ignored(".zshrc", "~/.zshrc"));
        assert!(!ignored(".zshrc", "~/.bashrc"));
    }

    #[test]
    fn basename_pattern_with_star() {
        assert!(ignored("id_*", "~/.ssh/id_rsa"));
        assert!(ignored("id_*", "~/.ssh/id_ed25519"));
        assert!(!ignored("id_*", "~/.ssh/config"));
    }

    #[test]
    fn anchored_path_pattern() {
        assert!(ignored(".ssh/id_rsa", "~/.ssh/id_rsa"));
        assert!(!ignored(".ssh/id_rsa", "~/.ssh/config"));
    }

    #[test]
    fn double_star_directory_pattern() {
        assert!(ignored(".config/app/**", "~/.config/app/settings.json"));
        assert!(ignored(".config/app/**", "~/.config/app/nested/file"));
        assert!(!ignored(".config/app/**", "~/.config/other/settings.json"));
    }

    #[test]
    fn negation_unignores() {
        let patterns = ".ssh/**\n!.ssh/config";
        assert!(ignored(patterns, "~/.ssh/id_rsa"));
        assert!(!ignored(patterns, "~/.ssh/config"));
    }

    #[test]
    fn last_pattern_wins() {
        // Second pattern re-ignores what the first negate released.
        let patterns = ".ssh/**\n!.ssh/id_rsa\n.ssh/id_rsa";
        assert!(ignored(patterns, "~/.ssh/id_rsa"));
    }

    #[test]
    fn leading_slash_stripped_and_treated_as_anchored() {
        assert!(ignored("/.ssh/id_rsa", "~/.ssh/id_rsa"));
        assert!(!ignored("/.ssh/id_rsa", "~/.ssh/config"));
    }

    // ── IgnoreList::load with template context ─────────────────────────────────

    use tempfile::TempDir;
    use crate::template::TemplateContext;
    use std::collections::HashMap;

    fn test_ctx(os: &str) -> TemplateContext {
        TemplateContext {
            os: os.to_string(),
            hostname: "testhost".to_string(),
            username: "testuser".to_string(),
            profile: "default".to_string(),
            home_dir: "/home/testuser".to_string(),
            source_dir: "/home/testuser/haven".to_string(),
            data: HashMap::new(),
        }
    }

    #[test]
    fn load_returns_empty_when_no_file() {
        let dir = TempDir::new().unwrap();
        let ctx = test_ctx("macos");
        let list = IgnoreList::load(dir.path(), &ctx);
        assert!(!list.is_ignored("~/.zshrc"));
    }

    #[test]
    fn load_renders_os_conditional() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        let content = "{% if os == \"macos\" %}\n.DS_Store\n{% endif %}\n";
        std::fs::write(config_dir.join("ignore"), content).unwrap();

        // On macos: .DS_Store should be ignored.
        let list = IgnoreList::load(dir.path(), &test_ctx("macos"));
        assert!(list.is_ignored("~/.DS_Store"));

        // On linux: .DS_Store should not be ignored.
        let list = IgnoreList::load(dir.path(), &test_ctx("linux"));
        assert!(!list.is_ignored("~/.DS_Store"));
    }

    #[test]
    fn load_returns_empty_on_render_error() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        // Malformed template — should degrade gracefully (no patterns, no crash).
        std::fs::write(config_dir.join("ignore"), "{% unclosed %}").unwrap();
        let ctx = test_ctx("macos");
        let list = IgnoreList::load(dir.path(), &ctx);
        assert!(!list.is_ignored("~/.zshrc"), "should ignore nothing on render error");
    }
}
