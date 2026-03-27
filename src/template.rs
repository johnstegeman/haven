/// Template rendering via the Tera engine.
///
/// Variables available in every template:
///
///   {{ os }}                          # "macos" | "linux" | <os name>
///   {{ hostname }}                    # machine hostname
///   {{ username }}                    # current user ($USER / $USERNAME)
///   {{ profile }}                     # active haven profile name
///   {{ home_dir }}                    # absolute path to the home directory (e.g. /Users/you)
///   {{ source_dir }}                  # absolute path to the haven repo root (e.g. /Users/you/haven)
///   {{ get_env(name="VAR") }}         # read an environment variable (Tera built-in)
///   {{ get_env(name="VAR", default="fallback") }}
///
/// Custom variables from `[data]` in `haven.toml` are available under the `data` namespace:
///
///   {{ data.host }}                   # from [data] host = "my-laptop"
///   {{ data.kanata_path }}            # from [data] kanata_path = "/usr/local/bin/kanata"
///
/// Tera also provides the full Jinja2-style control flow:
///   {% if os == "macos" %}...{% endif %}
///   {% for item in list %}...{% endfor %}
///   {# this is a comment #}
///
/// Files with `template = false` (the default) are copied verbatim — `{{ }}`
/// inside them is never interpreted.
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use tera::{Context as TeraContext, Tera};

/// Template name used internally when rendering a single source string.
const TEMPLATE_NAME: &str = "_haven_template";

/// Variables injected into every template render.
pub struct TemplateContext {
    pub os: String,
    pub hostname: String,
    pub username: String,
    pub profile: String,
    pub home_dir: String,
    pub source_dir: String,
    /// Custom variables from `[data]` in `haven.toml`.
    /// Accessible in templates as `{{ data.key }}`.
    pub data: HashMap<String, String>,
}

impl TemplateContext {
    /// Build from the current machine environment, loading profile and data from
    /// the haven state file and config at `repo_root`.
    ///
    /// Used by commands that have a `repo_root` but no pre-loaded config (e.g. `diff`,
    /// `list`, `add`). Reads `~/.haven/state.json` for the active profile and
    /// `haven.toml` for `[data]` variables. Falls back to `"default"` profile and
    /// empty data on any read error.
    pub fn from_env_for_repo(repo_root: &Path) -> Self {
        let config = crate::config::haven::HavenConfig::load(repo_root).unwrap_or_default();
        let state_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join(".haven");
        let profile = crate::state::State::load(&state_dir)
            .ok()
            .and_then(|s| s.profile)
            .unwrap_or_else(|| "default".to_string());
        Self::from_env(&profile, repo_root, config.data)
    }

    /// Build from the current machine environment.
    ///
    /// `data` comes from `[data]` in `haven.toml` — pass `config.data.clone()`.
    pub fn from_env(profile: &str, repo_root: &Path, data: HashMap<String, String>) -> Self {
        Self {
            os: detect_os(),
            hostname: detect_hostname(),
            username: detect_username(),
            profile: profile.to_string(),
            home_dir: dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .to_string_lossy()
                .into_owned(),
            source_dir: repo_root.to_string_lossy().into_owned(),
            data,
        }
    }

    fn to_tera(&self) -> TeraContext {
        let mut ctx = TeraContext::new();
        ctx.insert("os", &self.os);
        ctx.insert("hostname", &self.hostname);
        ctx.insert("username", &self.username);
        ctx.insert("profile", &self.profile);
        ctx.insert("home_dir", &self.home_dir);
        ctx.insert("source_dir", &self.source_dir);
        // Custom [data] variables are nested under a `data` object.
        ctx.insert("data", &self.data);
        ctx
    }
}

/// Render a Tera template string with the given context.
///
/// Registers the `op(path="...")` function for 1Password secret injection.
/// The function only executes if `{{ op(...) }}` actually appears in the template —
/// files without op() calls are not affected even if `op` is not installed.
///
/// Returns the rendered string or a Tera error (includes line/column).
pub fn render(source: &str, ctx: &TemplateContext) -> Result<String> {
    let mut tera = Tera::default();
    // Register the op() function — lazy: only called if the template uses it.
    tera.register_function("op", crate::onepassword::make_tera_function());
    // Parse the template.
    tera.add_raw_template(TEMPLATE_NAME, source)
        .context("Template parse error")?;
    let tera_ctx = ctx.to_tera();
    // autoescape=false: dotfiles contain shell syntax, HTML escaping would corrupt them.
    tera.render(TEMPLATE_NAME, &tera_ctx)
        .context("Template rendering failed")
}

/// Render a Tera template string, returning an empty string on error.
///
/// Used for `config/ignore` and other opt-in template files where a render
/// failure should produce an empty result (ignoring nothing) rather than
/// crashing the command. Errors are printed to stderr as warnings.
pub fn render_lenient(source: &str, ctx: &TemplateContext) -> String {
    match render(source, ctx) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("warning: config/ignore template error (ignoring all patterns): {}", e);
            String::new()
        }
    }
}

fn detect_os() -> String {
    if cfg!(target_os = "macos") {
        "macos".to_string()
    } else if cfg!(target_os = "linux") {
        "linux".to_string()
    } else {
        std::env::consts::OS.to_string()
    }
}

fn detect_hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn detect_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(profile: &str) -> TemplateContext {
        TemplateContext {
            os: "macos".to_string(),
            hostname: "testhost".to_string(),
            username: "testuser".to_string(),
            profile: profile.to_string(),
            home_dir: "/home/testuser".to_string(),
            source_dir: "/home/testuser/haven".to_string(),
            data: HashMap::new(),
        }
    }

    #[test]
    fn renders_os_variable() {
        let out = render("platform={{ os }}", &ctx("default")).unwrap();
        assert_eq!(out, "platform=macos");
    }

    #[test]
    fn renders_hostname_variable() {
        let out = render("host={{ hostname }}", &ctx("default")).unwrap();
        assert_eq!(out, "host=testhost");
    }

    #[test]
    fn renders_username_variable() {
        let out = render("user={{ username }}", &ctx("default")).unwrap();
        assert_eq!(out, "user=testuser");
    }

    #[test]
    fn renders_profile_variable() {
        let out = render("profile={{ profile }}", &ctx("work")).unwrap();
        assert_eq!(out, "profile=work");
    }

    #[test]
    fn renders_home_dir_variable() {
        let out = render("home={{ home_dir }}", &ctx("default")).unwrap();
        assert_eq!(out, "home=/home/testuser");
    }

    #[test]
    fn renders_source_dir_variable() {
        let out = render("dir={{ source_dir }}", &ctx("default")).unwrap();
        assert_eq!(out, "dir=/home/testuser/haven");
    }

    #[test]
    fn renders_conditional_block() {
        let tmpl = "{% if os == \"macos\" %}brew install foo{% else %}apt install foo{% endif %}";
        let out = render(tmpl, &ctx("default")).unwrap();
        assert_eq!(out, "brew install foo");
    }

    #[test]
    fn renders_env_variable_via_get_env() {
        std::env::set_var("HAVEN_TEST_VAR", "hello");
        let out = render(r#"{{ get_env(name="HAVEN_TEST_VAR") }}"#, &ctx("default")).unwrap();
        assert_eq!(out, "hello");
        std::env::remove_var("HAVEN_TEST_VAR");
    }

    #[test]
    fn renders_env_variable_with_default() {
        std::env::remove_var("HAVEN_MISSING_VAR");
        let out = render(
            r#"{{ get_env(name="HAVEN_MISSING_VAR", default="fallback") }}"#,
            &ctx("default"),
        )
        .unwrap();
        assert_eq!(out, "fallback");
    }

    #[test]
    fn renders_custom_data_variable() {
        let mut data = HashMap::new();
        data.insert("myhost".to_string(), "desktop".to_string());
        let mut ctx = ctx("default");
        ctx.data = data;
        let out = render("machine={{ data.myhost }}", &ctx).unwrap();
        assert_eq!(out, "machine=desktop");
    }

    #[test]
    fn passes_through_non_template_braces() {
        // Regular shell brace expansion must not be treated as template syntax.
        let out = render("arr=(a b c)", &ctx("default")).unwrap();
        assert_eq!(out, "arr=(a b c)");
    }

    #[test]
    fn errors_on_malformed_syntax() {
        let result = render("{{ unclosed", &ctx("default"));
        assert!(result.is_err(), "expected error for malformed template");
    }

    #[test]
    fn render_lenient_returns_empty_on_error() {
        // Malformed template should produce empty string (not panic/error).
        let result = render_lenient("{{ unclosed", &ctx("default"));
        assert_eq!(result, "");
    }

    #[test]
    fn render_lenient_returns_rendered_on_success() {
        let result = render_lenient("os={{ os }}", &ctx("default"));
        assert_eq!(result, "os=macos");
    }
}
