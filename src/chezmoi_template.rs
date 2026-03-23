/// Convert a chezmoi Go template to a dfiles Tera template.
///
/// # Mapping
///
/// chezmoi uses Go's `text/template` engine. dfiles uses Tera (Jinja2-style).
/// The two share similar delimiters (`{{ }}`), but differ in control-flow syntax
/// and available variables.
///
/// ## Variable substitutions
///
/// | chezmoi                          | dfiles                               |
/// |----------------------------------|--------------------------------------|
/// | `.chezmoi.hostname`              | `hostname`                           |
/// | `.chezmoi.username`              | `username`                           |
/// | `.chezmoi.os`                    | `os`                                 |
/// | `.chezmoi.arch`                  | *unconvertible* (warning emitted)    |
/// | `.chezmoi.homeDir`               | `home_dir`                           |
/// | `.chezmoi.sourceDir`             | `source_dir`                         |
/// | `.chezmoi.config.data.<key>`     | `get_env(name="<KEY>")` + warning    |
///
/// ## Control flow
///
/// | chezmoi                             | dfiles                          |
/// |-------------------------------------|---------------------------------|
/// | `{{- if eq .chezmoi.os "linux" }}`  | `{% if os == "linux" %}`        |
/// | `{{- if eq .chezmoi.os "darwin" }}` | `{% if os == "macos" %}`        |
/// | `{{- else }}`                       | `{% else %}`                    |
/// | `{{- else if eq .chezmoi.os "linux" }}`| `{% elif os == "linux" %}`  |
/// | `{{- end }}`                        | `{% endif %}`                   |
///
/// ## Comments
///
/// `{{/* ... */}}` and `{{- /* ... */ -}}` → `{# ... #}`
///
/// ## Trim markers
///
/// chezmoi (Go template) uses `{{- ` and ` -}}` to strip surrounding whitespace.
/// Tera handles whitespace differently (no trim markers in `{{ }}`), but uses
/// `{%- %}` and `{% -%}` for control blocks. We strip `-` trim markers during
/// conversion and accept minor whitespace differences.
///
/// ## Unconvertible constructs
///
/// Anything not in the mapping above (range loops, custom functions, pipeline
/// operators, etc.) is left as-is and a warning is added to `ConversionResult`.
/// The caller decides whether to accept a partial conversion or skip the file.

use std::borrow::Cow;

// ─── Public API ───────────────────────────────────────────────────────────────

/// Result of a template conversion attempt.
#[derive(Debug, Clone)]
pub struct ConversionResult {
    /// The converted template text (Tera syntax).
    pub text: String,
    /// Warnings for constructs that could not be automatically converted.
    /// An empty list means full conversion. Non-empty means partial — the
    /// caller should decide whether to proceed or skip.
    pub warnings: Vec<String>,
}

/// Convert a chezmoi Go template string to a dfiles Tera template string.
///
/// Always returns a `ConversionResult`. If `warnings` is non-empty, some
/// constructs were left unconverted (preserved as-is in the output so the
/// user can fix them manually). If `warnings` is empty, the conversion is
/// complete.
pub fn convert(input: &str) -> ConversionResult {
    let mut warnings: Vec<String> = Vec::new();
    let mut output = String::with_capacity(input.len());

    // Tokenise: split on `{{` and `}}` boundaries.
    // We process one token at a time. A token is either:
    //   - plain text (outside any `{{ }}`)
    //   - a tag (the content between `{{` and `}}`, inclusive)
    let tokens = tokenise(input);

    for token in tokens {
        match token {
            Token::Text(t) => output.push_str(t),
            Token::Tag(raw) => {
                let converted = convert_tag(raw, &mut warnings);
                output.push_str(&converted);
            }
        }
    }

    ConversionResult {
        text: output,
        warnings,
    }
}

// ─── Tokeniser ────────────────────────────────────────────────────────────────

#[derive(Debug)]
enum Token<'a> {
    Text(&'a str),
    Tag(&'a str), // includes the surrounding {{ }}
}

fn tokenise(input: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut remaining = input;

    while !remaining.is_empty() {
        match remaining.find("{{") {
            None => {
                tokens.push(Token::Text(remaining));
                break;
            }
            Some(start) => {
                if start > 0 {
                    tokens.push(Token::Text(&remaining[..start]));
                }
                let after_open = &remaining[start..];
                match after_open.find("}}") {
                    None => {
                        // Unclosed tag — emit as text.
                        tokens.push(Token::Text(after_open));
                        break;
                    }
                    Some(end) => {
                        let tag_end = end + 2; // include the "}}"
                        tokens.push(Token::Tag(&after_open[..tag_end]));
                        remaining = &after_open[tag_end..];
                    }
                }
            }
        }
    }
    tokens
}

// ─── Tag converter ────────────────────────────────────────────────────────────

/// Convert a single `{{ ... }}` tag (the whole string including delimiters).
fn convert_tag<'a>(raw: &'a str, warnings: &mut Vec<String>) -> Cow<'a, str> {
    // Strip outer delimiters and trim markers, extracting the inner content.
    // Patterns: `{{`, `{{-`, `}}`  `- }}`.
    let inner = raw
        .trim_start_matches("{{")
        .trim_end_matches("}}")
        .trim_start_matches('-')
        .trim_end_matches('-')
        .trim();

    // ── Comments: `/* ... */` ─────────────────────────────────────────────────
    if inner.starts_with("/*") && inner.ends_with("*/") {
        let body = inner
            .trim_start_matches("/*")
            .trim_end_matches("*/")
            .trim();
        return Cow::Owned(format!("{{# {} #}}", body));
    }

    // ── `if` / `else if` / `else` / `end` ────────────────────────────────────
    if let Some(rest) = inner.strip_prefix("if ") {
        return convert_if(rest.trim(), warnings);
    }
    if let Some(rest) = inner.strip_prefix("else if ") {
        return convert_elif(rest.trim(), warnings);
    }
    if inner == "else" {
        return Cow::Borrowed("{% else %}");
    }
    if inner == "end" {
        return Cow::Borrowed("{% endif %}");
    }

    // ── Variable output: `.chezmoi.*` ─────────────────────────────────────────
    if inner.starts_with('.') {
        return convert_variable(inner, warnings);
    }

    // ── Anything else — preserve and warn ────────────────────────────────────
    warnings.push(format!("Unconvertible construct: {{{{{}}}}}", raw
        .trim_start_matches("{{")
        .trim_end_matches("}}")
        .trim()));
    Cow::Owned(raw.to_string())
}

// ─── Condition converter (`if` / `else if`) ──────────────────────────────────

fn convert_if(condition: &str, warnings: &mut Vec<String>) -> Cow<'static, str> {
    if let Some(tera_cond) = convert_condition(condition) {
        Cow::Owned(format!("{{% if {} %}}", tera_cond))
    } else {
        warnings.push(format!("Unconvertible if-condition: {}", condition));
        Cow::Owned(format!("{{{{- if {} }}}}", condition))
    }
}

fn convert_elif(condition: &str, warnings: &mut Vec<String>) -> Cow<'static, str> {
    if let Some(tera_cond) = convert_condition(condition) {
        Cow::Owned(format!("{{% elif {} %}}", tera_cond))
    } else {
        warnings.push(format!("Unconvertible else-if condition: {}", condition));
        Cow::Owned(format!("{{{{- else if {} }}}}", condition))
    }
}

/// Convert a Go template boolean condition to a Tera condition string.
///
/// Returns `None` for unsupported conditions.
fn convert_condition(cond: &str) -> Option<String> {
    // `(index . "key")` → `data.key`  (truthy check for a custom data variable)
    if let Some(inner) = cond.strip_prefix("(index . \"").and_then(|s| s.strip_suffix("\")")) {
        return Some(format!("data.{}", inner));
    }

    // `eq .chezmoi.os "linux"` → `os == "linux"`
    // `eq .chezmoi.os "darwin"` → `os == "macos"`
    if let Some(rest) = cond.strip_prefix("eq ") {
        let parts: Vec<&str> = rest.splitn(2, ' ').collect();
        if parts.len() == 2 {
            let lhs = parts[0].trim();
            let rhs = parts[1].trim().trim_matches('"');
            if let Some(tera_lhs) = go_var_to_tera(lhs) {
                let tera_rhs = normalize_os_value(lhs, rhs);
                return Some(format!("{} == \"{}\"", tera_lhs, tera_rhs));
            }
        }
    }

    // `ne .chezmoi.os "linux"` → `os != "linux"`
    if let Some(rest) = cond.strip_prefix("ne ") {
        let parts: Vec<&str> = rest.splitn(2, ' ').collect();
        if parts.len() == 2 {
            let lhs = parts[0].trim();
            let rhs = parts[1].trim().trim_matches('"');
            if let Some(tera_lhs) = go_var_to_tera(lhs) {
                let tera_rhs = normalize_os_value(lhs, rhs);
                return Some(format!("{} != \"{}\"", tera_lhs, tera_rhs));
            }
        }
    }

    None
}

/// chezmoi uses "darwin" for macOS; dfiles uses "macos". Normalise for OS comparisons.
///
/// Only applies when `lhs` is `.chezmoi.os` — other comparisons pass through unchanged.
fn normalize_os_value<'a>(lhs: &str, rhs: &'a str) -> &'a str {
    if lhs == ".chezmoi.os" && rhs == "darwin" {
        // "darwin" is chezmoi's name for macOS; dfiles uses "macos".
        // Both are &'static str so we can return the literal.
        "macos"
    } else {
        rhs
    }
}

// ─── Variable output converter ────────────────────────────────────────────────

fn convert_variable<'a>(inner: &str, warnings: &mut Vec<String>) -> Cow<'a, str> {
    if let Some(tera) = go_var_to_tera(inner) {
        Cow::Owned(format!("{{{{ {} }}}}", tera))
    } else {
        warnings.push(format!("Unconvertible variable: {}", inner));
        Cow::Owned(format!("{{{{ {} }}}}", inner))
    }
}

/// Map a chezmoi Go template variable name to its Tera equivalent.
///
/// Returns `None` for variables with no direct mapping.
fn go_var_to_tera(var: &str) -> Option<String> {
    match var {
        ".chezmoi.hostname"  => Some("hostname".into()),
        ".chezmoi.username"  => Some("username".into()),
        ".chezmoi.os"        => Some("os".into()),
        ".chezmoi.homeDir"   => Some("home_dir".into()),
        ".chezmoi.homedir"   => Some("home_dir".into()), // lowercase variant
        ".chezmoi.sourceDir" => Some("source_dir".into()),
        ".chezmoi.sourcedir" => Some("source_dir".into()), // lowercase variant
        _ => {
            // `.someVar` (custom data variable, not a chezmoi built-in) → `data.someVar`
            if var.starts_with('.') && !var.starts_with(".chezmoi") {
                return Some(format!("data.{}", &var[1..]));
            }
            None
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn convert_clean(input: &str) -> String {
        let r = convert(input);
        assert!(
            r.warnings.is_empty(),
            "expected no warnings but got: {:?}",
            r.warnings
        );
        r.text
    }

    // ── Variable substitutions ────────────────────────────────────────────────

    #[test]
    fn converts_hostname() {
        assert_eq!(
            convert_clean("Host: {{ .chezmoi.hostname }}"),
            "Host: {{ hostname }}"
        );
    }

    #[test]
    fn converts_username() {
        assert_eq!(
            convert_clean("User: {{ .chezmoi.username }}"),
            "User: {{ username }}"
        );
    }

    #[test]
    fn converts_os() {
        assert_eq!(
            convert_clean("OS: {{ .chezmoi.os }}"),
            "OS: {{ os }}"
        );
    }

    #[test]
    fn converts_homedir() {
        assert_eq!(
            convert_clean("Home: {{ .chezmoi.homeDir }}"),
            "Home: {{ home_dir }}"
        );
    }

    #[test]
    fn converts_source_dir() {
        assert_eq!(
            convert_clean("Source: {{ .chezmoi.sourceDir }}"),
            "Source: {{ source_dir }}"
        );
    }

    #[test]
    fn converts_trim_marker_variable() {
        // `{{- .chezmoi.hostname -}}` → `{{ hostname }}`
        assert_eq!(
            convert_clean("{{- .chezmoi.hostname -}}"),
            "{{ hostname }}"
        );
    }

    // ── Control flow ──────────────────────────────────────────────────────────

    #[test]
    fn converts_if_os_linux() {
        assert_eq!(
            convert_clean("{{- if eq .chezmoi.os \"linux\" -}}apt{{- end }}"),
            "{% if os == \"linux\" %}apt{% endif %}"
        );
    }

    #[test]
    fn converts_if_os_darwin_to_macos() {
        assert_eq!(
            convert_clean("{{ if eq .chezmoi.os \"darwin\" }}brew{{ end }}"),
            "{% if os == \"macos\" %}brew{% endif %}"
        );
    }

    #[test]
    fn converts_else() {
        assert_eq!(
            convert_clean("{{ if eq .chezmoi.os \"linux\" }}apt{{ else }}brew{{ end }}"),
            "{% if os == \"linux\" %}apt{% else %}brew{% endif %}"
        );
    }

    #[test]
    fn converts_else_if() {
        let input = "{{ if eq .chezmoi.os \"linux\" }}apt{{ else if eq .chezmoi.os \"darwin\" }}brew{{ end }}";
        let expected = "{% if os == \"linux\" %}apt{% elif os == \"macos\" %}brew{% endif %}";
        assert_eq!(convert_clean(input), expected);
    }

    #[test]
    fn converts_ne_condition() {
        assert_eq!(
            convert_clean("{{ if ne .chezmoi.os \"linux\" }}not-linux{{ end }}"),
            "{% if os != \"linux\" %}not-linux{% endif %}"
        );
    }

    // ── Comments ──────────────────────────────────────────────────────────────

    #[test]
    fn converts_comment() {
        assert_eq!(
            convert_clean("text{{/* a comment */}}more"),
            "text{# a comment #}more"
        );
    }

    #[test]
    fn converts_trimmed_comment() {
        assert_eq!(
            convert_clean("{{- /* trimmed comment */ -}}"),
            "{# trimmed comment #}"
        );
    }

    // ── Unconvertible constructs ──────────────────────────────────────────────

    #[test]
    fn warns_on_range_loop() {
        let r = convert("{{ range .list }}item{{ end }}");
        assert!(!r.warnings.is_empty(), "expected warning for range loop");
    }

    #[test]
    fn warns_on_unknown_variable() {
        let r = convert("{{ .chezmoi.arch }}");
        assert!(!r.warnings.is_empty(), "expected warning for .chezmoi.arch");
    }

    #[test]
    fn warns_on_custom_data_variable() {
        let r = convert("{{ .chezmoi.config.data.email }}");
        assert!(!r.warnings.is_empty(), "expected warning for custom data variable");
    }

    // ── Mixed ─────────────────────────────────────────────────────────────────

    #[test]
    fn converts_realistic_gitconfig() {
        let input = r#"[user]
    name = {{ .chezmoi.username }}
    email = user@example.com
{{ if eq .chezmoi.os "darwin" }}
[credential]
    helper = osxkeychain
{{ else }}
[credential]
    helper = /usr/lib/git-core/git-credential-gnome-keyring
{{ end }}"#;

        let result = convert(input);
        assert!(result.warnings.is_empty(), "warnings: {:?}", result.warnings);
        assert!(result.text.contains("{{ username }}"));
        assert!(result.text.contains("{% if os == \"macos\" %}"));
        assert!(result.text.contains("{% else %}"));
        assert!(result.text.contains("{% endif %}"));
    }

    #[test]
    fn plain_text_passes_through_unchanged() {
        let input = "no templates here, just text with {braces}";
        let result = convert(input);
        assert!(result.warnings.is_empty());
        assert_eq!(result.text, input);
    }

    // ── Custom data variables ─────────────────────────────────────────────────

    #[test]
    fn converts_index_dot_key_condition() {
        // `(index . "host")` → truthy check `data.host`
        assert_eq!(
            convert_clean("{{ if (index . \"host\") }}yes{{ end }}"),
            "{% if data.host %}yes{% endif %}"
        );
    }

    #[test]
    fn converts_custom_data_variable() {
        // `.host` (not a chezmoi built-in) → `data.host`
        assert_eq!(
            convert_clean("export host={{ .host }}"),
            "export host={{ data.host }}"
        );
    }

    #[test]
    fn converts_index_dot_realistic_template() {
        // Full pattern as seen in host.zsh.tmpl imported from chezmoi
        let input = "{{- if (index . \"host\") }}\nexport host={{ .host }}\n{% else %}\nexport host=`hostname`\n{% endif %}";
        let result = convert(input);
        assert!(result.warnings.is_empty(), "warnings: {:?}", result.warnings);
        assert!(result.text.contains("{% if data.host %}"));
        assert!(result.text.contains("{{ data.host }}"));
        assert!(result.text.contains("{% else %}"));
        assert!(result.text.contains("{% endif %}"));
    }
}
