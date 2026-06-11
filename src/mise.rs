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
}
