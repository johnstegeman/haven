/// 1Password CLI integration.
///
/// Secrets are injected into templates at render time using the Tera function:
///
///   {{ op(path="op://vault/item/field") }}
///   {{ op(path="Personal/GitHub/token") }}   # op:// prefix added automatically
///
/// Prerequisites:
///   1. The `op` CLI must be installed: https://developer.1password.com/docs/cli/get-started/
///   2. The user must be signed in: `op signin`
///
/// haven never stores secrets to disk — they are rendered into the destination
/// file in memory and written directly. Source template files contain only the
/// `op://` URI references, not the secret values.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tera::{Error as TeraError, Value};

/// Well-known `op` install locations checked when it is not in PATH.
const OP_LOCATIONS: &[&str] = &[
    "/opt/homebrew/bin/op",      // macOS Homebrew (Apple Silicon)
    "/usr/local/bin/op",         // macOS Homebrew (Intel) or manual install
    "/usr/bin/op",               // Linux system install
    "/home/linuxbrew/.linuxbrew/bin/op", // Linux Homebrew
];

/// Find the `op` binary. Checks PATH first, then known install locations.
pub fn op_path() -> Option<PathBuf> {
    if let Ok(out) = std::process::Command::new("which").arg("op").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(PathBuf::from(s));
            }
        }
    }
    OP_LOCATIONS
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

/// Check whether the user is currently signed into 1Password.
///
/// Runs `op whoami` — exits 0 when authenticated, non-zero otherwise.
pub fn is_authenticated(op: &Path) -> bool {
    std::process::Command::new(op)
        .arg("whoami")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Read a single secret from 1Password.
///
/// `uri` may be a full `op://vault/item/field` reference or the short form
/// `vault/item/field` (the `op://` prefix is added automatically).
pub fn read_secret(op: &Path, uri: &str) -> Result<String, String> {
    let full_uri = if uri.starts_with("op://") {
        uri.to_string()
    } else {
        format!("op://{}", uri)
    };

    let out = std::process::Command::new(op)
        .args(["read", &full_uri])
        .output()
        .map_err(|e| format!("Cannot run op: {}", e))?;

    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("`op read {}` failed (exit {:?})", full_uri, out.status.code())
        } else {
            format!("`op read {}` failed: {}", full_uri, stderr)
        })
    }
}

/// Build the Tera `op()` function.
///
/// Usage in templates:
///   {{ op(path="op://Personal/GitHub/token") }}
///   {{ op(path="Work/Slack/api_key") }}
///
/// The function is always registered in every Tera render — it only executes
/// if `{{ op(...) }}` actually appears in the template being rendered.
pub fn make_tera_function() -> impl tera::Function {
    move |args: &HashMap<String, Value>| -> tera::Result<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TeraError::msg(
                    "op() requires a 'path' argument.\n\
                     Example: {{ op(path=\"op://vault/item/field\") }}",
                )
            })?;

        let op_bin = op_path().ok_or_else(|| {
            TeraError::msg(
                "op() requires the 1Password CLI, which is not installed.\n\
                 Install it from https://developer.1password.com/docs/cli/get-started/\n\
                 Then sign in with: op signin",
            )
        })?;

        if !is_authenticated(&op_bin) {
            return Err(TeraError::msg(
                "op() requires an active 1Password session.\n\
                 Sign in with: op signin",
            ));
        }

        read_secret(&op_bin, path).map(Value::String).map_err(TeraError::msg)
    }
}
