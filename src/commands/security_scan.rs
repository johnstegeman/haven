/// Scan tracked source files for secrets, sensitive filenames, and sensitive paths.
///
/// Exits 0 when no issues are found, 1 when findings are reported.
/// Add paths to `[security] allow` in `dfiles.toml` to suppress false positives.
use anyhow::Result;
use regex::Regex;
use std::path::Path;

use crate::config::dfiles::DfilesConfig;
use crate::fs::is_sensitive_with_rule;
use crate::ignore::IgnoreList;
use crate::template::TemplateContext;
use crate::source;

pub struct ScanOptions<'a> {
    pub repo_root: &'a Path,
    /// Enable high-entropy string detection (opt-in: may produce false positives).
    pub entropy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    High,
    Medium,
    #[allow(dead_code)] // reserved for future low-severity content rules
    Low,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::High => "HIGH",
            Severity::Medium => "MEDIUM",
            Severity::Low => "LOW",
        }
    }
}

#[derive(Debug)]
pub enum FindingKind {
    Filename,
    Path,
    Content { line: usize, snippet: String },
    Entropy { line: usize, snippet: String },
}

#[derive(Debug)]
pub struct Finding {
    pub dest_tilde: String,
    pub rule: &'static str,
    pub severity: Severity,
    pub kind: FindingKind,
    pub revoke_url: Option<&'static str>,
}

// ─── Content rules ─────────────────────────────────────────────────────────────

struct ContentRule {
    name: &'static str,
    pattern: Regex,
    severity: Severity,
    revoke_url: Option<&'static str>,
}

fn build_content_rules() -> Vec<ContentRule> {
    // (name, pattern, severity, revoke_url)
    let specs: &[(&'static str, &str, Severity, Option<&'static str>)] = &[
        (
            "PEM private key",
            r"-----BEGIN [A-Z ]*PRIVATE KEY",
            Severity::High,
            None,
        ),
        (
            "GitHub token (ghp_/ghs_/github_pat_)",
            r"(?:ghp_|ghs_|github_pat_)[A-Za-z0-9_]{10,}",
            Severity::High,
            Some("https://github.com/settings/tokens"),
        ),
        (
            "AWS access key ID",
            r"AKIA[0-9A-Z]{16}",
            Severity::High,
            Some("https://console.aws.amazon.com/iam/home#/security_credentials"),
        ),
        (
            "AWS secret access key",
            r"(?i)aws_secret_access_key\s*=\s*[A-Za-z0-9/+]{40}",
            Severity::High,
            Some("https://console.aws.amazon.com/iam/home#/security_credentials"),
        ),
        (
            "OpenAI API key",
            r"sk-[A-Za-z0-9]{20,}",
            Severity::High,
            Some("https://platform.openai.com/api-keys"),
        ),
        (
            "Anthropic API key",
            r"sk-ant-[A-Za-z0-9\-]{20,}",
            Severity::High,
            Some("https://console.anthropic.com/settings/keys"),
        ),
        (
            "Generic secret assignment",
            r#"(?i)(?:password|secret|api_key|auth_token)\s*[:=]\s*["']?[A-Za-z0-9!@#$%^&*()\-_+=]{8,}"#,
            Severity::Medium,
            None,
        ),
    ];

    specs
        .iter()
        .filter_map(|(name, pat, sev, url)| {
            Regex::new(pat).ok().map(|re| ContentRule {
                name,
                pattern: re,
                severity: *sev,
                revoke_url: *url,
            })
        })
        .collect()
}

// ─── Path rules ────────────────────────────────────────────────────────────────

/// (glob pattern matched against dest_tilde, rule name, severity)
const PATH_RULES: &[(&str, &str, Severity)] = &[
    ("~/.config/gh/hosts.yml",     "GitHub CLI credentials",       Severity::High),
    ("~/.config/gcloud/**",        "Google Cloud credentials",      Severity::High),
    ("~/.aws/credentials",         "AWS credentials file",          Severity::High),
    ("~/.aws/config",              "AWS config (may contain keys)", Severity::Medium),
    ("~/.docker/config.json",      "Docker credentials",            Severity::High),
    ("~/.kube/**",                 "Kubernetes credentials",        Severity::High),
    ("~/.ssh/**",                  "SSH key or config",             Severity::High),
    ("~/.gnupg/**",                "GPG keyring",                   Severity::High),
    ("~/.config/op/**",            "1Password credentials",         Severity::High),
    ("~/.netrc",                   ".netrc credentials",            Severity::High),
    ("~/.config/hub",              "Hub (GitHub) credentials",      Severity::High),
    ("~/.config/gh/**",            "GitHub CLI config",             Severity::Medium),
];

/// Returns the first matching path rule for `dest_tilde`, or `None`.
fn is_sensitive_path(dest_tilde: &str) -> Option<(&'static str, Severity)> {
    let path = dest_tilde.strip_prefix("~/").unwrap_or(dest_tilde);
    for &(glob, rule, sev) in PATH_RULES {
        let g = glob.strip_prefix("~/").unwrap_or(glob);
        if glob_matches(g, path) {
            return Some((rule, sev));
        }
    }
    None
}

/// Minimal glob matcher supporting `*` (non-separator) and `**` (any including `/`).
fn glob_matches(pattern: &str, path: &str) -> bool {
    glob_matches_inner(pattern, path)
}

fn glob_matches_inner(pat: &str, s: &str) -> bool {
    match pat.find('*') {
        None => pat == s,
        Some(star_pos) => {
            let prefix = &pat[..star_pos];
            if !s.starts_with(prefix) {
                return false;
            }
            let s = &s[star_pos..];
            let rest = &pat[star_pos..];
            if rest.starts_with("**") {
                let after = &rest[2..];
                // ** matches zero or more path segments (including separators)
                if after.is_empty() {
                    return true;
                }
                let after = after.trim_start_matches('/');
                // Try matching after at every position in s
                for i in 0..=s.len() {
                    if s[i..].starts_with('/') || i == 0 {
                        if glob_matches_inner(after, if i == 0 { s } else { &s[i + 1..] }) {
                            return true;
                        }
                    }
                }
                false
            } else {
                // single * — match non-separator chars
                let after = &rest[1..];
                for i in 0..=s.len() {
                    if s[..i].contains('/') {
                        break; // * cannot cross /
                    }
                    if glob_matches_inner(after, &s[i..]) {
                        return true;
                    }
                }
                false
            }
        }
    }
}

// ─── Main scan ────────────────────────────────────────────────────────────────

/// Build an `IgnoreList` from `[security] allow` patterns.
///
/// Allow patterns are written with `~/` prefix (e.g. `~/.config/gh/hosts.yml`),
/// but `IgnoreList` matches against paths with `~/` already stripped, so we
/// normalise by removing the prefix before compiling.
pub fn make_allow_list(patterns: &[String]) -> IgnoreList {
    if patterns.is_empty() {
        return IgnoreList::default();
    }
    let normalised: Vec<String> = patterns
        .iter()
        .map(|p| p.strip_prefix("~/").unwrap_or(p).to_string())
        .collect();
    IgnoreList::from_str(&normalised.join("\n"))
}

pub fn run(opts: &ScanOptions<'_>) -> Result<()> {
    let source_dir = opts.repo_root.join("source");
    let ctx = TemplateContext::from_env_for_repo(opts.repo_root);
    let ignore = IgnoreList::load(opts.repo_root, &ctx);
    let config = DfilesConfig::load(opts.repo_root).unwrap_or_default();
    let allow_list = make_allow_list(&config.security.allow);

    let entries = source::scan(&source_dir, &ignore)?;

    if entries.is_empty() {
        println!("No tracked files found.");
        return Ok(());
    }

    let content_rules = build_content_rules();
    let mut findings: Vec<Finding> = Vec::new();

    for entry in &entries {
        if allow_list.is_ignored(&entry.dest_tilde) {
            continue;
        }

        // 1. Filename-based check on the DECODED dest name, not the encoded source name.
        let dest_name = Path::new(&entry.dest_tilde)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if let Some(rule) = is_sensitive_with_rule(dest_name) {
            findings.push(Finding {
                dest_tilde: entry.dest_tilde.clone(),
                rule,
                severity: Severity::High,
                kind: FindingKind::Filename,
                revoke_url: None,
            });
        }

        // 2. Path-based check.
        if let Some((rule, severity)) = is_sensitive_path(&entry.dest_tilde) {
            // Only report path finding if not already flagged by filename for the same file.
            let already_flagged = findings
                .iter()
                .any(|f| f.dest_tilde == entry.dest_tilde && matches!(f.kind, FindingKind::Filename));
            if !already_flagged {
                findings.push(Finding {
                    dest_tilde: entry.dest_tilde.clone(),
                    rule,
                    severity,
                    kind: FindingKind::Path,
                    revoke_url: None,
                });
            }
        }

        // 3. Content-based check.
        scan_file_content(&entry.src, &entry.dest_tilde, &content_rules, opts.entropy, &mut findings);
    }

    if findings.is_empty() {
        println!(
            "Security scan: no issues found in {} tracked file(s).",
            entries.len()
        );
        return Ok(());
    }

    print_findings(&findings);
    std::process::exit(1);
}

// ─── File content scanning ────────────────────────────────────────────────────

fn scan_file_content(
    src: &Path,
    dest_tilde: &str,
    rules: &[ContentRule],
    entropy: bool,
    findings: &mut Vec<Finding>,
) {
    // Skip files larger than 1 MB.
    if let Ok(meta) = std::fs::metadata(src) {
        if meta.len() > 1_048_576 {
            return;
        }
    }

    let bytes = match std::fs::read(src) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("warning: cannot read {}", src.display());
            return;
        }
    };

    // Binary detection: null byte in first 8192 bytes.
    let probe = &bytes[..bytes.len().min(8192)];
    if probe.contains(&0u8) {
        return;
    }

    let text = match std::str::from_utf8(&bytes) {
        Ok(t) => t,
        Err(_) => return,
    };

    for (line_idx, line) in text.lines().enumerate() {
        let line_no = line_idx + 1;

        for rule in rules {
            if let Some(m) = rule.pattern.find(line) {
                let snippet: String = m.as_str().chars().take(8).collect();
                findings.push(Finding {
                    dest_tilde: dest_tilde.to_string(),
                    rule: rule.name,
                    severity: rule.severity,
                    kind: FindingKind::Content { line: line_no, snippet },
                    revoke_url: rule.revoke_url,
                });
                break; // one content finding per line is enough
            }
        }

        if entropy {
            check_entropy(line, line_no, dest_tilde, findings);
        }
    }
}

fn check_entropy(line: &str, line_no: usize, dest_tilde: &str, findings: &mut Vec<Finding>) {
    for token in line.split(|c: char| {
        c.is_whitespace() || c == '=' || c == ':' || c == '"' || c == '\'' || c == ','
    }) {
        if token.len() < 16 {
            continue;
        }
        if shannon_entropy(token) > 4.5 {
            let snippet: String = token.chars().take(8).collect();
            findings.push(Finding {
                dest_tilde: dest_tilde.to_string(),
                rule: "High-entropy string",
                severity: Severity::Medium,
                kind: FindingKind::Entropy { line: line_no, snippet },
                revoke_url: None,
            });
            return; // one entropy finding per line
        }
    }
}

fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    for b in s.bytes() {
        counts[b as usize] += 1;
    }
    let len = s.len() as f64;
    counts.iter().filter(|&&c| c > 0).fold(0.0, |acc, &c| {
        let p = c as f64 / len;
        acc - p * p.log2()
    })
}

// ─── Output ────────────────────────────────────────────────────────────────────

fn print_findings(findings: &[Finding]) {
    println!("\nSecurity scan: {} finding(s)\n", findings.len());
    println!("{:<8}  {:<45}  RULE / DETAIL", "SEVERITY", "FILE");
    println!("{}", "─".repeat(100));

    for f in findings {
        let detail = match &f.kind {
            FindingKind::Filename => "filename matches sensitive pattern".to_string(),
            FindingKind::Path => "path matches sensitive location".to_string(),
            FindingKind::Content { line, snippet } => {
                format!("line {line}: match starting with {snippet:?}")
            }
            FindingKind::Entropy { line, snippet } => {
                format!("line {line}: high-entropy token starting with {snippet:?}")
            }
        };
        println!(
            "{:<8}  {:<45}  {} — {}",
            f.severity.label(),
            f.dest_tilde,
            f.rule,
            detail
        );
        if let Some(url) = f.revoke_url {
            println!("          Revoke at: {url}");
        }
    }

    println!();
    println!("Run `dfiles security-scan` to re-check after fixing.");
    println!("Add paths to `[security] allow` in dfiles.toml to suppress false positives.");
}

// ─── Public API for dfiles add ───────────────────────────────────────────────

/// Scan a single file's content for secrets (used by `dfiles add`).
///
/// Does not check filename or path — those are the caller's responsibility.
/// Does not check entropy (opt-in only via `security-scan --entropy`).
/// Returns an empty vec if no content patterns match.
pub fn scan_single_file_content(src: &Path, dest_tilde: &str) -> Vec<Finding> {
    let rules = build_content_rules();
    let mut findings = Vec::new();
    scan_file_content(src, dest_tilde, &rules, false, &mut findings);
    findings
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[allow(dead_code)]
    fn make_repo_with_file(dest_name: &str, content: &str) -> (TempDir, PathBuf) {
        let repo = TempDir::new().unwrap();
        let source_dir = repo.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();

        // Use encoded source name: dot_<name> for hidden files.
        let src_name = if let Some(plain) = dest_name.strip_prefix('.') {
            format!("dot_{plain}")
        } else {
            dest_name.to_string()
        };
        let src_file = source_dir.join(&src_name);
        fs::write(&src_file, content).unwrap();

        (repo, src_file)
    }

    #[test]
    fn empty_repo_reports_no_issues() {
        let repo = TempDir::new().unwrap();
        fs::create_dir_all(repo.path().join("source")).unwrap();

        let _opts = ScanOptions { repo_root: repo.path(), entropy: false };
        // run() calls process::exit(1) on findings, so we test the scanning logic directly.
        let ignore = IgnoreList::default();
        let entries = source::scan(&repo.path().join("source"), &ignore).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn filename_match_detected() {
        // .env decoded name → is_sensitive_with_rule should catch it
        assert!(is_sensitive_with_rule(".env").is_some());
        assert!(is_sensitive_with_rule("id_rsa").is_some());
        assert!(is_sensitive_with_rule(".zshrc").is_none());
        assert!(is_sensitive_with_rule("credentials").is_some());
    }

    #[test]
    fn is_sensitive_with_rule_returns_rule_name() {
        let rule = is_sensitive_with_rule(".env").unwrap();
        assert_eq!(rule, ".env");

        let rule = is_sensitive_with_rule("id_rsa").unwrap();
        assert_eq!(rule, "_rsa");
    }

    #[test]
    fn is_sensitive_no_match() {
        assert!(is_sensitive_with_rule(".zshrc").is_none());
        assert!(is_sensitive_with_rule("gitconfig").is_none());
        assert!(is_sensitive_with_rule("aliases").is_none());
    }

    #[test]
    fn path_match_detected() {
        let result = is_sensitive_path("~/.config/gh/hosts.yml");
        assert!(result.is_some());
        let (rule, sev) = result.unwrap();
        assert_eq!(rule, "GitHub CLI credentials");
        assert_eq!(sev, Severity::High);
    }

    #[test]
    fn path_match_glob() {
        // ~/.kube/** should match any file under ~/.kube
        assert!(is_sensitive_path("~/.kube/config").is_some());
        assert!(is_sensitive_path("~/.kube/cache/discovery/foo").is_some());
        // ~/.ssh/** should match
        assert!(is_sensitive_path("~/.ssh/id_ed25519").is_some());
        // non-sensitive path should not match
        assert!(is_sensitive_path("~/.zshrc").is_none());
    }

    #[test]
    fn content_match_detected() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test_file");
        fs::write(&file, "line1\nGITHUB_TOKEN=ghp_abcdefghijklm\nline3\n").unwrap();

        let mut findings = Vec::new();
        let rules = build_content_rules();
        scan_file_content(&file, "~/.config/test", &rules, false, &mut findings);

        assert!(!findings.is_empty(), "should find GitHub token");
        if let FindingKind::Content { line, snippet } = &findings[0].kind {
            assert_eq!(*line, 2);
            assert_eq!(snippet.len(), 8);
            assert!(snippet.starts_with("ghp_"));
        } else {
            panic!("expected Content finding");
        }
    }

    #[test]
    fn binary_file_skipped() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("binary");
        // Write bytes with a null byte — should be treated as binary.
        let mut content = b"GITHUB_TOKEN=ghp_abc".to_vec();
        content.push(0u8);
        fs::write(&file, &content).unwrap();

        let mut findings = Vec::new();
        let rules = build_content_rules();
        scan_file_content(&file, "~/.config/test", &rules, false, &mut findings);
        assert!(findings.is_empty(), "binary file should be skipped");
    }

    #[test]
    fn allow_listed_file_skipped() {
        let repo = TempDir::new().unwrap();
        let source_dir = repo.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();

        // Write a sensitive file.
        let src_file = source_dir.join("dot_env");
        fs::write(&src_file, "API_KEY=secret123\n").unwrap();

        // Write dfiles.toml with allow list.
        let config_content = "[security]\nallow = [\"~/.env\"]\n";
        fs::write(repo.path().join("dfiles.toml"), config_content).unwrap();

        let config = DfilesConfig::load(repo.path()).unwrap();
        let allow_list = make_allow_list(&config.security.allow);

        // ~/.env should be allowed.
        assert!(allow_list.is_ignored("~/.env"));
    }

    #[test]
    fn entropy_off_by_default() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("high_entropy");
        // A high-entropy string that doesn't match known patterns.
        fs::write(&file, "token=Xk8mN2pQ7rL4vW9jY3bZ6dF1\n").unwrap();

        let mut findings = Vec::new();
        let rules = build_content_rules();
        // entropy=false
        scan_file_content(&file, "~/.config/test", &rules, false, &mut findings);
        let entropy_findings: Vec<_> = findings
            .iter()
            .filter(|f| matches!(f.kind, FindingKind::Entropy { .. }))
            .collect();
        assert!(entropy_findings.is_empty(), "entropy should not fire by default");
    }

    #[test]
    fn entropy_opt_in() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("high_entropy");
        // A high-entropy string that doesn't match known patterns (no known prefixes).
        fs::write(&file, "CUSTOM_KEY=Xk8mN2pQ7rL4vW9jY3bZ6dF1hAsGcEiKmOpQrStUv\n").unwrap();

        let mut findings = Vec::new();
        let rules = build_content_rules();
        // entropy=true
        scan_file_content(&file, "~/.config/test", &rules, true, &mut findings);
        let entropy_findings: Vec<_> = findings
            .iter()
            .filter(|f| matches!(f.kind, FindingKind::Entropy { .. }))
            .collect();
        assert!(!entropy_findings.is_empty(), "entropy should fire when enabled");
    }

    #[test]
    fn shannon_entropy_calibration() {
        // Low entropy: all same char → 0.0
        assert_eq!(shannon_entropy("aaaaaaaaaaaaaaaa"), 0.0);

        // High entropy: random-looking string.
        let h = shannon_entropy("Xk8mN2pQ7rL4vW9j");
        assert!(h > 3.5, "random string should have high entropy, got {h}");

        // English word: low entropy.
        let h = shannon_entropy("helloworld123456");
        assert!(h < 4.5, "common word should have lower entropy, got {h}");
    }
}
