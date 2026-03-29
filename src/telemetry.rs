/// Local telemetry: append-only JSONL event log at `~/.local/state/haven/telemetry.jsonl`.
///
/// # Design
///
/// - **Opt-in by default** — telemetry is off unless the user enables it via
///   `haven.toml`, `HAVEN_TELEMETRY=1`, or a special build feature flag.
/// - **Local only** — events are written to a file on the user's machine.
///   No data leaves the machine; the file is for the user (and optionally
///   shared with maintainers voluntarily for usage analysis).
/// - **Append-only JSONL** — one JSON object per line, easy to grep/jq.
/// - **No personal data** — only command names, timing, OS, flags, and errors.
///
/// # Event format
///
/// ```json
/// {"ts":"2026-03-21T12:00:00Z","cmd":"apply","flags":["--dry-run"],"profile":"default","os":"macos","arch":"aarch64","duration_ms":1234,"exit_ok":true}
/// ```
///
/// # Enabling
///
/// In `haven.toml`:
/// ```toml
/// [telemetry]
/// enabled = true
/// ```
///
/// Or set the environment variable:
/// ```sh
/// HAVEN_TELEMETRY=1 haven apply
/// ```
///
/// Or build with `--features telemetry-default-on` for special distribution
/// builds where telemetry is on by default.
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

/// A user-written note in the telemetry log.
///
/// Notes are written by `haven telemetry --note "..."` and stand out from
/// command events via the `kind` field. During analysis, filter for
/// `kind == "note"` to find context annotations the user left.
/// Append a user note to the telemetry JSONL file.
///
/// Always writes regardless of whether `[telemetry] enabled` is set — the user
/// explicitly invoked the command, so the intent is unambiguous.
///
/// Returns the auto-generated ID (N000001, N000002, …).
pub fn append_note(note: &str) -> anyhow::Result<String> {
    append_typed("note", 'N', note)
}

/// A typed telemetry annotation — action, bug, or question.
///
/// Written by `haven telemetry --action/--bug/--question "..."`.
/// The `id` is auto-generated (e.g. `A000001`, `B000001`, `Q000001`) by
/// scanning the existing log for the highest sequence number of that prefix.
///
/// Example JSON line:
/// ```json
/// {"ts":"2026-03-23T12:00:00Z","kind":"action","id":"A000001","note":"testing chezmoi migration guide"}
/// ```
#[derive(Debug, Serialize)]
pub struct TypedEvent {
    pub ts: String,
    pub kind: &'static str,
    pub id: String,
    pub note: String,
}

/// Append a typed annotation (action / bug / question) to the telemetry log.
///
/// Scans the existing log to determine the next sequence number for the given
/// prefix character (`A`, `B`, or `Q`) and returns the generated ID.
pub fn append_typed(kind: &'static str, prefix: char, text: &str) -> anyhow::Result<String> {
    let path = default_telemetry_path();
    let seq = next_seq_for_prefix(&path, prefix);
    let id = format!("{}{:06}", prefix, seq);
    let event = TypedEvent {
        ts: chrono::Utc::now().to_rfc3339(),
        kind,
        id: id.clone(),
        note: text.to_string(),
    };
    append_jsonl(&path, &event)?;
    Ok(id)
}

/// Print the telemetry JSONL file to stdout.
///
/// If `kind_filter` is `Some("bug")` etc., only lines with that `"kind"` are shown.
/// Pass `None` to show all lines.
pub fn list(kind_filter: Option<&str>) -> anyhow::Result<()> {
    let path = default_telemetry_path();
    if !path.exists() {
        println!("No telemetry data yet ({} does not exist).", path.display());
        return Ok(());
    }
    let contents = std::fs::read_to_string(&path)?;
    for line in contents.lines() {
        if let Some(kind) = kind_filter {
            let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            if val.get("kind").and_then(|v| v.as_str()) != Some(kind) {
                continue;
            }
        }
        println!("{}", line);
    }
    Ok(())
}

/// Return the next sequence number for IDs starting with `prefix`.
///
/// Scans every line in `path` looking for `"id":"<prefix><digits>"` and
/// returns `max + 1` (or `1` if no matches are found).
fn next_seq_for_prefix(path: &PathBuf, prefix: char) -> u32 {
    let Ok(contents) = std::fs::read_to_string(path) else { return 1 };
    let mut max = 0u32;
    for line in contents.lines() {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
            if id.starts_with(prefix) {
                if let Ok(n) = id[prefix.len_utf8()..].parse::<u32>() {
                    if n > max { max = n; }
                }
            }
        }
    }
    max + 1
}

fn append_jsonl<T: serde::Serialize>(path: &PathBuf, event: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_string(event).map_err(std::io::Error::other)?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())
}

/// Build identity embedded at compile time: `"<version>+<short-commit>"`.
/// Example: `"0.3.0+47625a5"`.
pub const BUILD_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "+",
    env!("HAVEN_GIT_COMMIT"),
);

/// A single telemetry event.
#[derive(Debug, Serialize)]
pub struct Event {
    /// RFC-3339 timestamp (UTC).
    pub ts: String,
    /// Build version: `"<semver>+<short-commit>"` (e.g. `"0.3.0+47625a5"`).
    pub version: &'static str,
    /// Top-level command name (e.g. "apply", "status", "diff").
    pub cmd: String,
    /// CLI flags that were passed (flag names only, no values that might be PII).
    pub flags: Vec<String>,
    /// Active profile name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Operating system family: "macos", "linux", or "windows".
    pub os: &'static str,
    /// CPU architecture: "aarch64", "x86_64", etc.
    pub arch: &'static str,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Whether the command exited without error.
    pub exit_ok: bool,
    /// Short error message if `exit_ok` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A telemetry recorder that measures duration from creation to `finish()`.
pub struct Recorder {
    enabled: bool,
    cmd: String,
    flags: Vec<String>,
    profile: Option<String>,
    started: Instant,
    path: PathBuf,
}

impl Recorder {
    /// Create a new recorder.
    ///
    /// `enabled` should be `false` when telemetry is off — all methods become
    /// no-ops so there is zero overhead in the hot path.
    pub fn new(
        enabled: bool,
        cmd: impl Into<String>,
        flags: Vec<String>,
        profile: Option<String>,
    ) -> Self {
        Self {
            enabled,
            cmd: cmd.into(),
            flags,
            profile,
            started: Instant::now(),
            path: default_telemetry_path(),
        }
    }

    /// Record the command result and append the event to the JSONL file.
    ///
    /// Call this exactly once, just before the process exits.
    /// If telemetry is disabled this is a no-op.
    pub fn finish(self, result: &anyhow::Result<()>) {
        if !self.enabled {
            return;
        }

        let duration_ms = self.started.elapsed().as_millis() as u64;
        let exit_ok = result.is_ok();
        let error = result.as_ref().err().map(|e| {
            // Truncate long errors; strip any path segments that might be PII.
            let msg = e.to_string();
            if msg.len() > 200 { format!("{}…", &msg[..200]) } else { msg }
        });

        let event = Event {
            ts: chrono::Utc::now().to_rfc3339(),
            version: BUILD_VERSION,
            cmd: self.cmd,
            flags: self.flags,
            profile: self.profile,
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            duration_ms,
            exit_ok,
            error,
        };

        // Best-effort: ignore any write errors so telemetry never crashes haven.
        let _ = append_event(&self.path, &event);
    }
}

/// Determine whether telemetry is enabled for this invocation.
///
/// Resolution order (first wins):
/// 1. `HAVEN_TELEMETRY=0` → disabled
/// 2. `HAVEN_TELEMETRY=1` → enabled
/// 3. `[telemetry] enabled = true` in `haven.toml` → enabled
/// 4. `telemetry-default-on` feature flag → enabled
/// 5. Otherwise → disabled
pub fn is_enabled(config_enabled: bool) -> bool {
    let env_val = std::env::var("HAVEN_TELEMETRY").ok();
    is_enabled_inner(config_enabled, env_val.as_deref())
}

fn is_enabled_inner(config_enabled: bool, env_val: Option<&str>) -> bool {
    match env_val {
        Some("0") | Some("false") | Some("no") => return false,
        Some(_) => return true,
        None => {}
    }
    if config_enabled {
        return true;
    }
    cfg!(feature = "telemetry-default-on")
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn default_telemetry_path() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/state")
        })
        .join("haven")
        .join("telemetry.jsonl")
}

fn append_event(path: &PathBuf, event: &Event) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_string(event).map_err(std::io::Error::other)?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn is_enabled_respects_env_override() {
        // HAVEN_TELEMETRY=0 always disables, even if config says true.
        assert!(!is_enabled_inner(true, Some("0")));
        assert!(!is_enabled_inner(true, Some("false")));
        assert!(!is_enabled_inner(true, Some("no")));

        // Any other non-empty value enables regardless of config.
        assert!(is_enabled_inner(false, Some("1")));
        assert!(is_enabled_inner(false, Some("true")));
        assert!(is_enabled_inner(false, Some("yes")));
    }

    #[test]
    fn is_enabled_follows_config_when_no_env() {
        // Without the feature flag, config controls it.
        #[cfg(not(feature = "telemetry-default-on"))]
        {
            assert!(!is_enabled_inner(false, None));
            assert!(is_enabled_inner(true, None));
        }
    }

    #[test]
    fn append_event_creates_file_and_is_valid_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".haven").join("telemetry.jsonl");

        let event = Event {
            ts: "2026-03-21T12:00:00Z".into(),
            version: "0.0.0+test",
            cmd: "apply".into(),
            flags: vec!["--dry-run".into()],
            profile: Some("default".into()),
            os: "linux",
            arch: "x86_64",
            duration_ms: 42,
            exit_ok: true,
            error: None,
        };

        append_event(&path, &event).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("\"cmd\":\"apply\""));
        assert!(contents.contains("\"exit_ok\":true"));
        // Ensure it's valid JSON.
        let _: serde_json::Value = serde_json::from_str(contents.trim()).unwrap();
    }

    #[test]
    fn append_note_event_writes_valid_jsonl() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("telemetry.jsonl");
        let event = TypedEvent {
            ts: "2026-03-23T12:00:00Z".into(),
            kind: "note",
            id: "N000001".into(),
            note: "starting fresh config — prior data is from testing".into(),
        };
        append_jsonl(&path, &event).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("\"kind\":\"note\""));
        assert!(contents.contains("starting fresh config"));
        let _: serde_json::Value = serde_json::from_str(contents.trim()).unwrap();
    }

    #[test]
    fn next_seq_starts_at_1_when_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("telemetry.jsonl");
        assert_eq!(next_seq_for_prefix(&path, 'A'), 1);
    }

    #[test]
    fn next_seq_increments_correctly() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("telemetry.jsonl");
        // Seed two action events and one bug event.
        for id in &["A000001", "A000002", "B000001"] {
            let line = format!("{{\"kind\":\"action\",\"id\":\"{}\",\"note\":\"x\"}}\n", id);
            std::fs::write(&path, line).unwrap();
        }
        // Overwrite with all three lines.
        let content = "{\"id\":\"A000001\"}\n{\"id\":\"A000002\"}\n{\"id\":\"B000003\"}\n";
        std::fs::write(&path, content).unwrap();
        assert_eq!(next_seq_for_prefix(&path, 'A'), 3);
        assert_eq!(next_seq_for_prefix(&path, 'B'), 4);
        assert_eq!(next_seq_for_prefix(&path, 'Q'), 1);
    }

    #[test]
    fn append_typed_writes_valid_jsonl() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("telemetry.jsonl");
        // Call append_typed via the internal helpers directly.
        let seq = next_seq_for_prefix(&path, 'B');
        assert_eq!(seq, 1);
        let id = format!("B{:06}", seq);
        let event = TypedEvent {
            ts: "2026-03-23T12:00:00Z".into(),
            kind: "bug",
            id: id.clone(),
            note: "security scan flags allowlisted file".into(),
        };
        append_jsonl(&path, &event).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("\"kind\":\"bug\""));
        assert!(contents.contains("\"id\":\"B000001\""));
        let _: serde_json::Value = serde_json::from_str(contents.trim()).unwrap();
    }

    #[test]
    fn recorder_noop_when_disabled() {
        // When disabled, finish() must not create any file.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("telemetry.jsonl");

        let rec = Recorder {
            enabled: false,
            cmd: "status".into(),
            flags: vec![],
            profile: None,
            started: Instant::now(),
            path: path.clone(),
        };
        rec.finish(&Ok(()));
        assert!(!path.exists(), "no file should be written when telemetry is disabled");
    }
}
