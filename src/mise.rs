/// Mise (https://mise.jdx.dev) integration: detection and tool installation.
///
/// Detection order:
///   1. `mise` in PATH
///   2. `~/.local/bin/mise` (default mise self-install location)
///
/// Install flow:
///   If mise is absent, prints a one-line hint — mise install is intentionally
///   left to the user for now (unlike Homebrew, there's no single canonical
///   installer URL that works well non-interactively on all platforms).
///
/// Tool installation:
///   `mise install` (reads the config file or nearest .mise.toml / .tool-versions)
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, Value};

use crate::packages::OutdatedPackage;

/// Find the `mise` binary. Checks PATH first, then `~/.local/bin/mise`.
pub fn mise_path() -> Option<PathBuf> {
    // PATH lookup via `which`.
    if let Ok(out) = std::process::Command::new("which").arg("mise").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
    }
    // Default self-install location.
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".local").join("bin").join("mise");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Run `mise install` using the given config file (passed via `MISE_CONFIG_FILE` env).
///
/// If `config` is None, mise reads its default config from the working directory.
pub fn install_tools(mise: &Path, config: Option<&Path>) -> Result<()> {
    let mut cmd = std::process::Command::new(mise);
    cmd.arg("install");
    if let Some(cfg) = config {
        cmd.env("MISE_CONFIG_FILE", cfg);
    }
    cmd.stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    let status = cmd.status().context("Cannot run `mise install`")?;

    if !status.success() {
        anyhow::bail!("`mise install` failed (exit {:?})", status.code());
    }
    Ok(())
}

/// Check whether all tools in the config are installed.
///
/// Runs `mise current` and checks exit code. Returns `false` on any error.
pub fn tools_installed(mise: &Path, config: Option<&Path>) -> bool {
    let mut cmd = std::process::Command::new(mise);
    cmd.arg("current");
    if let Some(cfg) = config {
        cmd.env("MISE_CONFIG_FILE", cfg);
    }
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

/// Split a tool spec like `node@22` into `("node", "22")`.
/// A bare name with no `@` maps to `("name", "latest")`.
pub fn parse_tool_spec(spec: &str) -> (String, String) {
    match spec.split_once('@') {
        Some((name, version)) => (name.to_string(), version.to_string()),
        None => (spec.to_string(), "latest".to_string()),
    }
}

/// Add `name = "version"` under `[tools]` in the mise config at `path`.
///
/// Creates the file (and parent directories) if absent. Idempotent: if the
/// key already exists with the same value, the file is not modified.
/// Returns `true` if the entry was added or updated, `false` if unchanged.
pub fn add_to_misefile(path: &Path, name: &str, version: &str) -> Result<bool> {
    let mut doc = load_or_create_doc(path)?;

    if !doc.contains_key("tools") {
        doc["tools"] = Item::Table(Table::new());
    }
    let tools = doc["tools"]
        .as_table_mut()
        .context("[tools] is not a table")?;

    if let Some(existing) = tools.get(name) {
        if existing.as_str() == Some(version) {
            return Ok(false);
        }
    }

    tools[name] = Item::Value(Value::from(version));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create {}", parent.display()))?;
    }
    std::fs::write(path, doc.to_string())
        .with_context(|| format!("Cannot write {}", path.display()))?;
    Ok(true)
}

/// Remove `name` from `[tools]` in the mise config at `path`.
///
/// Returns the count of keys removed (0 or 1). Does nothing if the file or
/// key does not exist.
pub fn remove_from_misefile(path: &Path, name: &str) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let mut doc = load_or_create_doc(path)?;
    if !doc.contains_key("tools") {
        return Ok(0);
    }
    let tools = doc["tools"]
        .as_table_mut()
        .context("[tools] is not a table")?;
    if tools.remove(name).is_some() {
        std::fs::write(path, doc.to_string())
            .with_context(|| format!("Cannot write {}", path.display()))?;
        Ok(1)
    } else {
        Ok(0)
    }
}

/// Return all `(name, version)` pairs declared in `[tools]` in the mise config at `path`.
///
/// Returns an empty vec if the file does not exist or has no `[tools]` section.
#[allow(dead_code)]
pub fn parse_mise_tools(path: &Path) -> Result<Vec<(String, String)>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let doc = load_or_create_doc(path)?;
    let Some(tools) = doc.get("tools").and_then(|t| t.as_table()) else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for (k, v) in tools.iter() {
        let version = v.as_str().unwrap_or("").to_string();
        result.push((k.to_string(), version));
    }
    Ok(result)
}

/// Run `mise uninstall <name>` best-effort.
///
/// A non-zero exit code is treated as "tool not installed" and is not an error.
pub fn mise_uninstall(mise: &Path, name: &str) -> Result<()> {
    let _ = std::process::Command::new(mise)
        .args(["uninstall", name])
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();
    Ok(())
}

/// Run `mise outdated --json` with `MISE_CONFIG_FILE=<config>` and return all
/// outdated tools.
///
/// `mise outdated` exits 0 whether or not packages are outdated.
pub fn mise_outdated(mise: &str, config: &Path) -> Result<Vec<OutdatedPackage>> {
    let out = std::process::Command::new(mise)
        .args(["outdated", "--json"])
        .env("MISE_CONFIG_FILE", config)
        .output()
        .with_context(|| format!("Cannot run `{} outdated --json` — is mise installed?", mise))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("`{} outdated --json` failed: {}", mise, stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.trim().is_empty() {
        return Ok(Vec::new());
    }

    parse_mise_outdated_json(&stdout)
}

fn parse_mise_outdated_json(json: &str) -> Result<Vec<OutdatedPackage>> {
    let v: serde_json::Value =
        serde_json::from_str(json).context("Failed to parse `mise outdated --json` output")?;

    let Some(obj) = v.as_object() else {
        return Ok(Vec::new());
    };

    let mut result = Vec::new();
    for (name, entry) in obj {
        let current = entry["current"].as_str().unwrap_or("?").to_string();
        let latest = entry["latest"].as_str().unwrap_or("?").to_string();
        result.push(OutdatedPackage {
            name: name.clone(),
            current_version: current,
            latest_version: latest,
        });
    }
    Ok(result)
}

/// Query the mise registry for tools whose name contains `term`.
///
/// Uses `mise registry` (lists all tools) and filters locally.  `mise registry
/// <name>` only works for exact names, so substring search requires the full
/// listing.
pub fn mise_search(mise: &str, term: &str) -> Result<Vec<String>> {
    let out = std::process::Command::new(mise)
        .arg("registry")
        .output()
        .with_context(|| format!("Cannot run `{} registry` — is mise installed?", mise))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("`{} registry` failed: {}", mise, stderr.trim());
    }

    let lower_term = term.to_lowercase();
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let tool_name = line.split_whitespace().next()?;
            if tool_name.to_lowercase().contains(&lower_term) {
                Some(tool_name.to_string())
            } else {
                None
            }
        })
        .collect())
}

/// Upgrade tool(s) in the given mise config file and rewrite the pinned version.
///
/// Uses `mise upgrade --bump` with `MISE_CONFIG_FILE` set so that the upgraded
/// version is written back into the config file, not just installed.
///
/// When `name` is `None`, all tools declared in the config are upgraded.
/// The config is read back after the upgrade to surface any parse errors early.
pub fn mise_upgrade(mise: &str, config: &Path, name: Option<&str>) -> Result<()> {
    let tools_before = parse_mise_tools(config)?;

    let tool_names: Vec<String> = match name {
        Some(n) => vec![n.to_string()],
        None => tools_before.iter().map(|(k, _)| k.clone()).collect(),
    };

    if tool_names.is_empty() {
        return Ok(());
    }

    let mut cmd = std::process::Command::new(mise);
    cmd.arg("upgrade").arg("--bump").arg("--yes");
    cmd.args(&tool_names);
    cmd.env("MISE_CONFIG_FILE", config);
    cmd.stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    let status = cmd
        .status()
        .with_context(|| format!("Cannot run `{} upgrade` — is mise installed?", mise))?;

    if !status.success() {
        anyhow::bail!(
            "`{} upgrade --bump` failed (exit {:?})",
            mise,
            status.code()
        );
    }

    // Read back the config to confirm it is still valid TOML and surface any
    // post-upgrade parse errors early.
    let _tools_after = parse_mise_tools(config)?;

    Ok(())
}

/// Merge `[tools]` from all `module_configs` into a single global mise config.
///
/// Entries from later configs overwrite earlier ones for duplicate keys.
/// All other sections in `global_config` (e.g. `[settings]`) are preserved.
/// If `module_configs` is empty, the `[tools]` table is cleared.
pub fn merge_module_tools_into_global(
    module_configs: &[PathBuf],
    global_config: &Path,
) -> Result<()> {
    // Build merged tools map, preserving insertion order; later entries win.
    let mut merged: Vec<(String, Item)> = Vec::new();

    for config_path in module_configs {
        if !config_path.exists() {
            continue;
        }
        let doc = load_or_create_doc(config_path)?;
        let Some(tools_table) = doc.get("tools").and_then(|t| t.as_table()) else {
            continue;
        };
        for (key, item) in tools_table.iter() {
            // Remove any existing entry for this key, then push the new value.
            merged.retain(|(k, _)| k != key);
            merged.push((key.to_string(), item.clone()));
        }
    }

    let mut doc = load_or_create_doc(global_config)?;

    // Replace the [tools] table entirely.
    doc.remove("tools");
    let mut new_tools = Table::new();
    for (key, item) in merged {
        new_tools.insert(&key, item);
    }
    doc.insert("tools", Item::Table(new_tools));

    if let Some(parent) = global_config.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create {}", parent.display()))?;
    }
    std::fs::write(global_config, doc.to_string())
        .with_context(|| format!("Cannot write {}", global_config.display()))?;
    Ok(())
}

fn load_or_create_doc(path: &Path) -> Result<DocumentMut> {
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("Cannot read {}", path.display()))?;
    text.parse::<DocumentMut>()
        .with_context(|| format!("Invalid TOML in {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_tool_spec_with_version() {
        assert_eq!(parse_tool_spec("node@22"), ("node".into(), "22".into()));
    }

    #[test]
    fn parse_tool_spec_bare_name() {
        assert_eq!(parse_tool_spec("node"), ("node".into(), "latest".into()));
    }

    #[test]
    fn parse_tool_spec_full_semver() {
        assert_eq!(
            parse_tool_spec("python@3.11.2"),
            ("python".into(), "3.11.2".into())
        );
    }

    #[test]
    fn add_to_misefile_creates_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mise.toml");
        let added = add_to_misefile(&path, "node", "latest").unwrap();
        assert!(added);
        assert!(path.exists());
        let tools = parse_mise_tools(&path).unwrap();
        assert_eq!(tools, vec![("node".to_string(), "latest".to_string())]);
    }

    #[test]
    fn add_to_misefile_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested/dir/mise.toml");
        let added = add_to_misefile(&path, "node", "22").unwrap();
        assert!(added);
        assert!(path.exists());
    }

    #[test]
    fn add_to_misefile_idempotent_same_value() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mise.toml");
        add_to_misefile(&path, "node", "22").unwrap();
        let added = add_to_misefile(&path, "node", "22").unwrap();
        assert!(!added);
        let tools = parse_mise_tools(&path).unwrap();
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn add_to_misefile_updates_existing_different_version() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mise.toml");
        add_to_misefile(&path, "node", "20").unwrap();
        let added = add_to_misefile(&path, "node", "22").unwrap();
        assert!(added);
        let tools = parse_mise_tools(&path).unwrap();
        assert_eq!(tools, vec![("node".to_string(), "22".to_string())]);
    }

    #[test]
    fn add_to_misefile_preserves_existing_tools() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mise.toml");
        add_to_misefile(&path, "python", "3.11").unwrap();
        add_to_misefile(&path, "node", "22").unwrap();
        let tools = parse_mise_tools(&path).unwrap();
        assert_eq!(tools.len(), 2);
        assert!(tools.iter().any(|(k, v)| k == "python" && v == "3.11"));
        assert!(tools.iter().any(|(k, v)| k == "node" && v == "22"));
    }

    #[test]
    fn add_to_misefile_preserves_comments() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mise.toml");
        std::fs::write(&path, "# my comment\n[tools]\npython = \"3.11\"\n").unwrap();
        add_to_misefile(&path, "node", "22").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# my comment"));
        assert!(content.contains("python"));
        assert!(content.contains("node"));
    }

    #[test]
    fn remove_from_misefile_removes_key() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mise.toml");
        add_to_misefile(&path, "node", "22").unwrap();
        let removed = remove_from_misefile(&path, "node").unwrap();
        assert_eq!(removed, 1);
        let tools = parse_mise_tools(&path).unwrap();
        assert!(tools.is_empty());
    }

    #[test]
    fn remove_from_misefile_missing_key_returns_zero() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mise.toml");
        add_to_misefile(&path, "node", "22").unwrap();
        let removed = remove_from_misefile(&path, "python").unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn remove_from_misefile_missing_file_returns_zero() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.toml");
        let removed = remove_from_misefile(&path, "node").unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn parse_mise_tools_empty_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.toml");
        let tools = parse_mise_tools(&path).unwrap();
        assert!(tools.is_empty());
    }

    #[test]
    fn parse_mise_tools_reads_all_tools() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mise.toml");
        std::fs::write(&path, "[tools]\nnode = \"22\"\npython = \"3.11\"\n").unwrap();
        let tools = parse_mise_tools(&path).unwrap();
        assert_eq!(tools.len(), 2);
        assert!(tools.iter().any(|(k, v)| k == "node" && v == "22"));
        assert!(tools.iter().any(|(k, v)| k == "python" && v == "3.11"));
    }

    #[test]
    fn parse_mise_tools_empty_for_no_tools_section() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mise.toml");
        std::fs::write(&path, "# no tools\n").unwrap();
        let tools = parse_mise_tools(&path).unwrap();
        assert!(tools.is_empty());
    }

    // parse_mise_outdated_json tests

    #[test]
    fn parse_mise_outdated_json_multiple_tools() {
        let json = r#"{
            "node": {"current": "20.0.0", "latest": "22.0.0", "source": "mise.toml"},
            "python": {"current": "3.11.0", "latest": "3.12.0", "source": "mise.toml"}
        }"#;
        let pkgs = parse_mise_outdated_json(json).unwrap();
        assert_eq!(pkgs.len(), 2);

        let node = pkgs.iter().find(|p| p.name == "node").unwrap();
        assert_eq!(node.current_version, "20.0.0");
        assert_eq!(node.latest_version, "22.0.0");

        let python = pkgs.iter().find(|p| p.name == "python").unwrap();
        assert_eq!(python.current_version, "3.11.0");
        assert_eq!(python.latest_version, "3.12.0");
    }

    #[test]
    fn parse_mise_outdated_json_empty_object() {
        let json = r#"{}"#;
        let pkgs = parse_mise_outdated_json(json).unwrap();
        assert!(pkgs.is_empty());
    }

    #[test]
    fn parse_mise_outdated_json_missing_fields_defaults_to_question_mark() {
        let json = r#"{"node": {}}"#;
        let pkgs = parse_mise_outdated_json(json).unwrap();
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "node");
        assert_eq!(pkgs[0].current_version, "?");
        assert_eq!(pkgs[0].latest_version, "?");
    }

    #[test]
    fn merge_two_module_configs_into_global() {
        let dir = TempDir::new().unwrap();
        let mod1 = dir.path().join("mod1.toml");
        let mod2 = dir.path().join("mod2.toml");
        let global = dir.path().join("global.toml");

        std::fs::write(&mod1, "[tools]\nnode = \"22\"\n").unwrap();
        std::fs::write(&mod2, "[tools]\npython = \"3.12\"\n").unwrap();

        merge_module_tools_into_global(&[mod1, mod2], &global).unwrap();

        let tools = parse_mise_tools(&global).unwrap();
        assert_eq!(tools.len(), 2);
        assert!(tools.iter().any(|(k, v)| k == "node" && v == "22"));
        assert!(tools.iter().any(|(k, v)| k == "python" && v == "3.12"));
    }

    #[test]
    fn merge_preserves_settings_section() {
        let dir = TempDir::new().unwrap();
        let mod1 = dir.path().join("mod1.toml");
        let global = dir.path().join("global.toml");

        std::fs::write(&mod1, "[tools]\nnode = \"22\"\n").unwrap();
        std::fs::write(
            &global,
            "[settings]\nexperimental = true\n\n[tools]\nold = \"1\"\n",
        )
        .unwrap();

        merge_module_tools_into_global(&[mod1], &global).unwrap();

        let content = std::fs::read_to_string(&global).unwrap();
        assert!(content.contains("experimental = true"));
        assert!(content.contains("node"));
        assert!(!content.contains("old"));
    }

    #[test]
    fn merge_empty_configs_clears_tools() {
        let dir = TempDir::new().unwrap();
        let global = dir.path().join("global.toml");

        std::fs::write(&global, "[tools]\nnode = \"22\"\n").unwrap();

        merge_module_tools_into_global(&[], &global).unwrap();

        let tools = parse_mise_tools(&global).unwrap();
        assert!(tools.is_empty());
    }

    #[test]
    fn merge_duplicate_key_last_wins() {
        let dir = TempDir::new().unwrap();
        let mod1 = dir.path().join("mod1.toml");
        let mod2 = dir.path().join("mod2.toml");
        let global = dir.path().join("global.toml");

        std::fs::write(&mod1, "[tools]\nnode = \"20\"\n").unwrap();
        std::fs::write(&mod2, "[tools]\nnode = \"22\"\n").unwrap();

        merge_module_tools_into_global(&[mod1, mod2], &global).unwrap();

        let tools = parse_mise_tools(&global).unwrap();
        assert_eq!(tools.len(), 1);
        assert!(tools.iter().any(|(k, v)| k == "node" && v == "22"));
    }

    /// Verifies that `mise_upgrade` rewrites the pinned version in a mise.toml.
    ///
    /// Requires `mise` to be installed and available in PATH, and needs network
    /// access to resolve tool versions.  Skipped in CI and offline environments.
    #[test]
    #[ignore = "requires mise binary and network access to resolve latest version"]
    fn mise_upgrade_rewrites_pinned_version() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("mise.toml");
        std::fs::write(&config_path, "[tools]\nnode = \"20.0.0\"\n").unwrap();

        let Some(mise_bin) = super::mise_path() else {
            return; // mise not installed — skip
        };
        let mise_str = mise_bin.to_string_lossy();

        mise_upgrade(&mise_str, &config_path, Some("node")).unwrap();

        let tools = parse_mise_tools(&config_path).unwrap();
        let node_version = tools
            .iter()
            .find(|(k, _)| k == "node")
            .map(|(_, v)| v.as_str())
            .unwrap_or("20.0.0");

        assert_ne!(
            node_version, "20.0.0",
            "expected the pinned version to be bumped from 20.0.0, got: {node_version}"
        );
    }
}
