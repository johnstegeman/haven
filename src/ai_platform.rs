/// Platform plugin architecture for AI skill deployment.
///
/// Platform definitions are resolved in three layers:
///
/// 1. **Embedded registry** (`src/data/platforms.toml`, compiled in) — shipped defaults.
///
/// 2. **Local registry** (`~/.haven/platforms.toml`, not committed) — machine-local
///    additions and full-platform overrides. Use this when a shipped definition becomes
///    stale (e.g. a platform changes its skills directory) or when a new platform exists
///    that hasn't been added to haven yet. Same `[[platform]]` array format as the
///    embedded file. Entries with a matching `id` replace the embedded definition entirely;
///    new IDs are added to the registry.
///
/// 3. **Repo config** (`ai/platforms.toml` in the haven repo, committed) — declares
///    which platforms are `active` on this machine and applies field-level overrides on
///    top of the resolved registry.
///
/// ```toml
/// # ai/platforms.toml (committed)
/// active = ["claude-code", "codex"]
///
/// # Field-level override on a built-in or local-registry platform:
/// [platform.claude-code]
/// skills_dir = "~/.claude/my-skills"
///
/// # Custom platform (not in registry — skills_dir required):
/// [platform.my-tool]
/// name       = "My Tool"
/// skills_dir = "~/.mytool/skills"
/// ```
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::module::expand_tilde;

/// Embedded shipped platform registry (parsed at runtime from compile-time bytes).
static BUILTIN_PLATFORMS_TOML: &str = include_str!("data/platforms.toml");

/// A resolved platform plugin — built-in definition merged with any user overrides.
#[derive(Debug, Clone)]
pub struct PlatformPlugin {
    pub id: String,
    pub name: String,
    pub config_dir: Option<PathBuf>,
    /// Where skills are deployed for this platform.
    pub skills_dir: PathBuf,
    pub config_file: Option<PathBuf>,
    /// Binary name used by `haven ai discover` to detect the platform.
    pub binary: Option<String>,
    /// Whether this platform follows the agentskills.io standard
    /// (skills deployed to `~/.agents/skills/`).
    pub agentskills_compliant: bool,
}

/// User configuration in `ai/platforms.toml`.
#[derive(Debug, Deserialize, Default)]
pub struct PlatformsConfig {
    /// Platform IDs that are active on this machine.
    #[serde(default)]
    pub active: Vec<String>,

    /// Per-platform overrides and custom platform definitions.
    #[serde(default, rename = "platform")]
    pub overrides: HashMap<String, PlatformOverride>,
}

/// Override fields for a built-in or custom platform in `ai/platforms.toml`.
///
/// Any field present overrides the corresponding built-in default.
/// For custom platforms (not in the built-in list), `skills_dir` is required.
#[derive(Debug, Deserialize, Default)]
pub struct PlatformOverride {
    pub name: Option<String>,
    pub config_dir: Option<String>,
    pub skills_dir: Option<String>,
    pub config_file: Option<String>,
    pub binary: Option<String>,
    pub agentskills_compliant: Option<bool>,
}

// ── Deserialization types for the shipped platforms.toml ─────────────────────

#[derive(Debug, Deserialize)]
struct BuiltinPlatformDef {
    id: String,
    name: String,
    config_dir: Option<String>,
    skills_dir: String,
    config_file: Option<String>,
    binary: Option<String>,
    #[serde(default)]
    agentskills_compliant: bool,
}

#[derive(Debug, Deserialize)]
struct BuiltinPlatformsFile {
    platform: Vec<BuiltinPlatformDef>,
}

// ─────────────────────────────────────────────────────────────────────────────

impl PlatformsConfig {
    /// Load `ai/platforms.toml` from `repo_root`.
    /// Returns `Ok(None)` if the file doesn't exist (no AI config).
    pub fn load(repo_root: &Path) -> Result<Option<Self>> {
        let path = repo_root.join("ai").join("platforms.toml");
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("Cannot read {}", path.display()))?;
        let config: Self = toml::from_str(&text)
            .with_context(|| format!("Invalid TOML in {}", path.display()))?;
        Ok(Some(config))
    }

    /// Resolve all active platforms, merging built-in definitions with overrides.
    ///
    /// Returns an error if any ID in `active` is unknown (no built-in and no
    /// `[platform.X]` section in `ai/platforms.toml`).
    pub fn resolve_active_platforms(&self) -> Result<Vec<PlatformPlugin>> {
        let builtins = builtin_platforms();
        let builtin_ids: Vec<&str> = builtins.iter().map(|p| p.id.as_str()).collect();
        let mut result = Vec::new();

        for id in &self.active {
            let plugin = resolve_platform(id, &builtins, &self.overrides).with_context(|| {
                format!(
                    "Unknown platform '{}' in ai/platforms.toml active list.\n\
                     Built-in platforms: {}\n\
                     To use a custom platform, add a [platform.{}] section.",
                    id,
                    builtin_ids.join(", "),
                    id
                )
            })?;
            result.push(plugin);
        }

        Ok(result)
    }
}

/// Build the resolved platform registry by merging two sources:
///
/// 1. The embedded `src/data/platforms.toml` (shipped defaults).
/// 2. `~/.local/state/haven/platforms.toml` (machine-local overrides/additions, if present).
///
/// When both sources define a platform with the same `id`, the local file wins
/// (full replacement — not field-level merge). New IDs in the local file are
/// appended to the registry.
///
/// Panics at runtime if the embedded TOML is malformed (compile-time defect).
/// Silently ignores a malformed local file (prints a warning) so that one bad
/// entry does not prevent haven from running.
/// Return the full platform registry (embedded defaults merged with the
/// machine-local `~/.local/state/haven/platforms.toml` overrides).
///
/// Used by `haven ai discover` to enumerate all known platforms.
pub fn platform_registry() -> Vec<PlatformPlugin> {
    builtin_platforms()
}

fn builtin_platforms() -> Vec<PlatformPlugin> {
    let state_dir = dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("~"))
                .join(".local/state")
        })
        .join("haven");
    builtin_platforms_with_state_dir(&state_dir)
}

/// Testable core of `builtin_platforms()` — accepts an explicit state directory.
fn builtin_platforms_with_state_dir(state_dir: &Path) -> Vec<PlatformPlugin> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));

    // Layer 1: embedded shipped registry.
    let embedded: BuiltinPlatformsFile = toml::from_str(BUILTIN_PLATFORMS_TOML)
        .expect("src/data/platforms.toml is malformed — this is a haven bug");

    let mut defs: Vec<BuiltinPlatformDef> = embedded.platform;

    // Layer 2: machine-local registry (~/.local/state/haven/platforms.toml).
    let local_path = state_dir.join("platforms.toml");
    if local_path.exists() {
        match std::fs::read_to_string(&local_path)
            .map_err(|e| e.to_string())
            .and_then(|t| toml::from_str::<BuiltinPlatformsFile>(&t).map_err(|e| e.to_string()))
        {
            Ok(local) => {
                for local_def in local.platform {
                    // Replace embedded entry if same id, otherwise append.
                    if let Some(pos) = defs.iter().position(|d| d.id == local_def.id) {
                        defs[pos] = local_def;
                    } else {
                        defs.push(local_def);
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "Warning: {} is malformed and will be ignored: {}",
                    local_path.display(),
                    e
                );
            }
        }
    }

    defs.into_iter()
        .map(|def| PlatformPlugin {
            id: def.id,
            name: def.name,
            config_dir: def.config_dir.map(|d| expand_home(&home, &d)),
            skills_dir: expand_home(&home, &def.skills_dir),
            config_file: def.config_file.map(|f| expand_home(&home, &f)),
            binary: def.binary,
            agentskills_compliant: def.agentskills_compliant,
        })
        .collect()
}

/// Expand a `~`-prefixed path using the provided home directory.
///
/// Uses the already-resolved `home` PathBuf rather than calling
/// `dirs::home_dir()` repeatedly. Falls back to `expand_tilde` for
/// any path that fails simple prefix expansion.
fn expand_home(home: &Path, path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else if path == "~" {
        home.to_path_buf()
    } else {
        PathBuf::from(path)
    }
}

/// Resolve one platform ID to a `PlatformPlugin`, applying any overrides.
fn resolve_platform(
    id: &str,
    builtins: &[PlatformPlugin],
    overrides: &HashMap<String, PlatformOverride>,
) -> Option<PlatformPlugin> {
    let builtin = builtins.iter().find(|p| p.id == id).cloned();
    let override_ = overrides.get(id);

    match (builtin, override_) {
        (None, None) => None,
        (Some(base), None) => Some(base),
        (Some(mut base), Some(o)) => {
            if let Some(n) = &o.name {
                base.name = n.clone();
            }
            if let Some(d) = &o.config_dir {
                base.config_dir = expand_tilde(d).ok();
            }
            if let Some(s) = &o.skills_dir {
                if let Ok(p) = expand_tilde(s) {
                    base.skills_dir = p;
                }
            }
            if let Some(f) = &o.config_file {
                base.config_file = expand_tilde(f).ok();
            }
            if let Some(b) = &o.binary {
                base.binary = Some(b.clone());
            }
            if let Some(c) = o.agentskills_compliant {
                base.agentskills_compliant = c;
            }
            Some(base)
        }
        (None, Some(o)) => {
            // Custom platform: skills_dir is required.
            let skills_dir = expand_tilde(o.skills_dir.as_deref()?).ok()?;
            Some(PlatformPlugin {
                id: id.to_string(),
                name: o.name.clone().unwrap_or_else(|| id.to_string()),
                config_dir: o.config_dir.as_deref().and_then(|d| expand_tilde(d).ok()),
                skills_dir,
                config_file: o.config_file.as_deref().and_then(|f| expand_tilde(f).ok()),
                binary: o.binary.clone(),
                agentskills_compliant: o.agentskills_compliant.unwrap_or(false),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_platforms_toml(dir: &TempDir, content: &str) {
        let ai_dir = dir.path().join("ai");
        std::fs::create_dir_all(&ai_dir).unwrap();
        std::fs::write(ai_dir.join("platforms.toml"), content).unwrap();
    }

    #[test]
    fn embedded_toml_parses_cleanly() {
        // Ensure the shipped registry is always valid.
        let platforms = builtin_platforms();
        assert!(!platforms.is_empty());
    }

    #[test]
    fn loads_active_list() {
        let dir = TempDir::new().unwrap();
        write_platforms_toml(
            &dir,
            r#"active = ["claude-code", "codex"]"#,
        );

        let cfg = PlatformsConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(cfg.active, ["claude-code", "codex"]);
    }

    #[test]
    fn returns_none_when_file_absent() {
        let dir = TempDir::new().unwrap();
        assert!(PlatformsConfig::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn resolves_builtin_platform() {
        let dir = TempDir::new().unwrap();
        write_platforms_toml(&dir, r#"active = ["claude-code"]"#);

        let cfg = PlatformsConfig::load(dir.path()).unwrap().unwrap();
        let platforms = cfg.resolve_active_platforms().unwrap();

        assert_eq!(platforms.len(), 1);
        assert_eq!(platforms[0].id, "claude-code");
        assert!(platforms[0].skills_dir.to_string_lossy().contains(".claude/skills"));
        assert!(!platforms[0].agentskills_compliant);
    }

    #[test]
    fn agentskills_compliant_platforms() {
        let dir = TempDir::new().unwrap();
        write_platforms_toml(&dir, r#"active = ["github-copilot", "cross-client"]"#);

        let cfg = PlatformsConfig::load(dir.path()).unwrap().unwrap();
        let platforms = cfg.resolve_active_platforms().unwrap();

        assert!(platforms.iter().all(|p| p.agentskills_compliant));
    }

    #[test]
    fn unknown_platform_id_is_hard_error() {
        let dir = TempDir::new().unwrap();
        write_platforms_toml(&dir, r#"active = ["claudecode"]"#); // typo

        let cfg = PlatformsConfig::load(dir.path()).unwrap().unwrap();
        let err = cfg.resolve_active_platforms().unwrap_err();
        let msg = format!("{:#}", err);
        assert!(msg.contains("Unknown platform 'claudecode'"), "error was: {}", msg);
        assert!(msg.contains("Built-in platforms:"), "error was: {}", msg);
    }

    #[test]
    fn override_replaces_field() {
        let dir = TempDir::new().unwrap();
        write_platforms_toml(
            &dir,
            r#"
active = ["claude-code"]

[platform.claude-code]
skills_dir = "/tmp/test-skills"
"#,
        );

        let cfg = PlatformsConfig::load(dir.path()).unwrap().unwrap();
        let platforms = cfg.resolve_active_platforms().unwrap();

        assert_eq!(platforms[0].skills_dir, PathBuf::from("/tmp/test-skills"));
        // Non-overridden fields keep their built-in values.
        assert_eq!(platforms[0].name, "Claude Code");
        assert!(!platforms[0].agentskills_compliant);
    }

    #[test]
    fn override_agentskills_compliant() {
        let dir = TempDir::new().unwrap();
        write_platforms_toml(
            &dir,
            r#"
active = ["claude-code"]

[platform.claude-code]
agentskills_compliant = true
"#,
        );

        let cfg = PlatformsConfig::load(dir.path()).unwrap().unwrap();
        let platforms = cfg.resolve_active_platforms().unwrap();
        assert!(platforms[0].agentskills_compliant);
    }

    #[test]
    fn custom_platform_without_skills_dir_is_error() {
        let dir = TempDir::new().unwrap();
        write_platforms_toml(
            &dir,
            r#"
active = ["my-tool"]

[platform.my-tool]
name = "My Tool"
# missing skills_dir
"#,
        );

        let cfg = PlatformsConfig::load(dir.path()).unwrap().unwrap();
        // Should fail: custom platform with no skills_dir → resolve returns None → error.
        assert!(cfg.resolve_active_platforms().is_err());
    }

    #[test]
    fn custom_platform_with_skills_dir_resolves() {
        let dir = TempDir::new().unwrap();
        write_platforms_toml(
            &dir,
            r#"
active = ["my-tool"]

[platform.my-tool]
name       = "My Tool"
skills_dir = "/tmp/my-tool/skills"
"#,
        );

        let cfg = PlatformsConfig::load(dir.path()).unwrap().unwrap();
        let platforms = cfg.resolve_active_platforms().unwrap();

        assert_eq!(platforms[0].id, "my-tool");
        assert_eq!(platforms[0].name, "My Tool");
        assert_eq!(platforms[0].skills_dir, PathBuf::from("/tmp/my-tool/skills"));
        assert!(!platforms[0].agentskills_compliant);
    }

    #[test]
    fn custom_platform_agentskills_compliant_opt_in() {
        let dir = TempDir::new().unwrap();
        write_platforms_toml(
            &dir,
            r#"
active = ["my-tool"]

[platform.my-tool]
name                  = "My Tool"
skills_dir            = "/tmp/my-tool/skills"
agentskills_compliant = true
"#,
        );

        let cfg = PlatformsConfig::load(dir.path()).unwrap().unwrap();
        let platforms = cfg.resolve_active_platforms().unwrap();
        assert!(platforms[0].agentskills_compliant);
    }

    #[test]
    fn builtin_platform_ids_are_present() {
        let builtins = builtin_platforms();
        let ids: Vec<&str> = builtins.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"claude-code"));
        assert!(ids.contains(&"codex"));
        assert!(ids.contains(&"cursor"));
        assert!(ids.contains(&"github-copilot"));
        assert!(ids.contains(&"cross-client"));
    }

    #[test]
    fn copilot_and_cross_client_share_agents_skills_dir() {
        let builtins = builtin_platforms();
        let copilot = builtins.iter().find(|p| p.id == "github-copilot").unwrap();
        let cross = builtins.iter().find(|p| p.id == "cross-client").unwrap();
        assert_eq!(copilot.skills_dir, cross.skills_dir);
        assert!(copilot.agentskills_compliant);
        assert!(cross.agentskills_compliant);
    }

    // ── Local registry (~/.local/state/haven/platforms.toml) ────────────────

    fn write_local_registry(state_dir: &TempDir, content: &str) {
        std::fs::create_dir_all(state_dir.path()).unwrap();
        std::fs::write(state_dir.path().join("platforms.toml"), content).unwrap();
    }

    #[test]
    fn local_registry_absent_returns_only_embedded() {
        let state_dir = TempDir::new().unwrap();
        let platforms = builtin_platforms_with_state_dir(state_dir.path());
        let ids: Vec<&str> = platforms.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"claude-code"));
        assert!(ids.contains(&"codex"));
    }

    #[test]
    fn local_registry_adds_new_platform() {
        let state_dir = TempDir::new().unwrap();
        write_local_registry(
            &state_dir,
            r#"
[[platform]]
id = "new-agent"
name = "New Agent"
skills_dir = "~/.new-agent/skills"
agentskills_compliant = false
"#,
        );

        let platforms = builtin_platforms_with_state_dir(state_dir.path());
        let ids: Vec<&str> = platforms.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"new-agent"), "expected new-agent in {:?}", ids);
        // Embedded platforms still present.
        assert!(ids.contains(&"claude-code"));

        let new_agent = platforms.iter().find(|p| p.id == "new-agent").unwrap();
        assert_eq!(new_agent.name, "New Agent");
        assert!(new_agent.skills_dir.ends_with(".new-agent/skills"));
    }

    #[test]
    fn local_registry_overrides_embedded_platform() {
        let state_dir = TempDir::new().unwrap();
        write_local_registry(
            &state_dir,
            r#"
[[platform]]
id = "claude-code"
name = "Claude Code (local override)"
skills_dir = "~/.agents/skills"
agentskills_compliant = true
"#,
        );

        let platforms = builtin_platforms_with_state_dir(state_dir.path());
        let claude = platforms.iter().find(|p| p.id == "claude-code").unwrap();
        assert_eq!(claude.name, "Claude Code (local override)");
        assert!(claude.skills_dir.ends_with(".agents/skills"));
        assert!(claude.agentskills_compliant);
        // Only one entry for claude-code (not duplicated).
        let count = platforms.iter().filter(|p| p.id == "claude-code").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn local_registry_malformed_is_ignored_with_warning() {
        let state_dir = TempDir::new().unwrap();
        write_local_registry(&state_dir, "this is not valid toml [[[");

        // Should not panic — falls back to embedded platforms only.
        let platforms = builtin_platforms_with_state_dir(state_dir.path());
        let ids: Vec<&str> = platforms.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"claude-code"), "embedded platforms should still load");
    }

    #[test]
    fn local_registry_platform_usable_in_active_list() {
        let state_dir = TempDir::new().unwrap();
        write_local_registry(
            &state_dir,
            r#"
[[platform]]
id = "future-agent"
name = "Future Agent"
skills_dir = "/opt/future-agent/skills"
binary = "future-agent"
agentskills_compliant = false
"#,
        );

        // Simulate what resolve_active_platforms does: use local registry as builtins.
        let builtins = builtin_platforms_with_state_dir(state_dir.path());
        let overrides = HashMap::new();
        let plugin = resolve_platform("future-agent", &builtins, &overrides);
        assert!(plugin.is_some());
        let plugin = plugin.unwrap();
        assert_eq!(plugin.binary.as_deref(), Some("future-agent"));
    }
}
