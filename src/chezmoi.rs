/// chezmoi source directory detection and entry decoding.
///
/// Decodes chezmoi's naming conventions into dfiles-ready entries:
///
///  detect_source_dir()
///        │
///  parse .chezmoiexternal.toml → Vec<ChezmoiExternalEntry>
///        │
///  walkdir(source_dir)
///        │
///        ├─ dir entry starts with '.' → skip dir + all contents
///        ├─ dir entry = dot_* / private_* / executable_* / bare dir → recurse
///        │
///        ├─ file → decode_entry(rel_path)
///        │       ├─ private_* prefix(es)    → strip prefix, Keep with private=true
///        │       ├─ executable_* prefix(es) → strip prefix, Keep with executable=true
///        │       ├─ symlink_*               → read content, resolve target
///        │       │     target resolves      → Keep with link=true, copy_from=target
///        │       │     target unresolvable  → Skip(Symlink)
///        │       ├─ modify_*                       → Skip(Unsupported)
///        │       ├─ exact_* (dir prefix)           → Keep (prefix preserved; apply purges untracked entries)
///        │       ├─ create_*                       → Keep (prefix preserved; apply skips if dest exists)
///        │       ├─ run_once_* / run_* / once_*   → Skip(Script)
///        │       ├─ *.tmpl                         → Skip(Template)
///        │       ├─ .chezmoi* | chezmoistate*      → Skip(Internal)
///        │       ├─ dot_<name>                     → Keep(~/.<name>)
///        │       └─ <bare>                         → Keep(~/<bare>)
///        │
///        └─ (Vec<ChezmoiEntry>, Vec<ChezmoiExternalEntry>, Vec<SkippedEntry>)
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use serde::Deserialize;

// ─── Public types ─────────────────────────────────────────────────────────────

/// A chezmoi source file that should be imported into dfiles.
#[derive(Debug, Clone)]
pub struct ChezmoiEntry {
    /// Relative path inside the chezmoi source directory (e.g. `dot_config/git/config`).
    pub chezmoi_path: PathBuf,
    /// Decoded absolute destination path (e.g. `~/.config/git/config` as a string).
    pub dest_tilde: String,
    /// dfiles source/ filename (e.g. `"config-git-config"`).
    pub source_name: String,
    /// Inferred dfiles module name (e.g. `"shell"`, `"git"`, `"editor"`, `"misc"`).
    #[allow(dead_code)]
    pub module: String,
    /// Decoded from `private_` prefix: destination should be chmod 0600 (or 0700).
    pub private: bool,
    /// Decoded from `executable_` prefix: destination should be chmod 0755 (or 0700).
    pub executable: bool,
    /// When true, the dfiles entry will use `link = true` (symlink instead of copy).
    pub link: bool,
    /// When true, the file is a converted template (`template = true` in TOML).
    /// `converted_content` holds the Tera source to write instead of copying the
    /// original chezmoi source file.
    pub template: bool,
    /// When `template = true`, the converted Tera template text. Written to
    /// `source/<source_name>` instead of copying the original file.
    pub converted_content: Option<String>,
    /// Warnings from the template converter (non-empty when conversion was partial).
    pub template_warnings: Vec<String>,
    /// When set, the file to copy into source/ is this path rather than
    /// `source_dir/chezmoi_path`. Used for `symlink_` entries where the actual
    /// file is the resolved symlink target.
    pub copy_from: Option<PathBuf>,
}

/// A chezmoi source entry that was skipped during import.
#[derive(Debug, Clone)]
pub struct SkippedEntry {
    /// Relative path inside the chezmoi source directory.
    pub chezmoi_path: PathBuf,
    /// Why it was skipped.
    pub reason: SkipReason,
}

/// Why a chezmoi entry was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// `symlink_` prefix — symlink targets (P1 follow-on).
    Symlink,
    /// `exact_` / `create_` / `modify_` prefix — unsupported chezmoi attributes (P1).
    UnsupportedAttribute,
    /// `.tmpl` suffix — Go templates (P1 follow-on).
    Template,
    /// `run_once_` / `run_` / `once_` prefix — install/run scripts (P1 follow-on).
    Script,
    /// chezmoi-internal files (`.chezmoi*`, `chezmoistate.boltdb`) — silently skipped.
    Internal,
    /// Destination matched a pattern in `.chezmoiignore` — skipped unless
    /// `--include-ignored-files` is passed.
    Ignored,
}

impl SkipReason {
    /// Returns a human-readable reason string shown in the skip summary.
    /// `Internal` entries are silent and should not be shown.
    pub fn display(&self) -> Option<&'static str> {
        match self {
            SkipReason::Symlink => Some("symlink — target could not be resolved (manual migration required)"),
            SkipReason::UnsupportedAttribute => Some("unsupported chezmoi attribute (P1)"),
            SkipReason::Template => Some("Go template — could not read file (manual migration required)"),
            SkipReason::Script => Some("unrecognised script — manual migration required (see TODOS.md)"),
            SkipReason::Internal => None, // silent
            SkipReason::Ignored => Some("ignored by .chezmoiignore (use --include-ignored-files to import anyway)"),
        }
    }
}

/// A detected migration from a chezmoi `run_once_` / `run_` / `once_` script.
///
/// Recognised patterns are converted to dfiles module TOML on import.
/// Unrecognised scripts are collected separately and skipped with an explanation.
#[derive(Debug, Clone)]
pub struct ChezmoiScriptEntry {
    /// Relative path inside the chezmoi source directory.
    pub chezmoi_path: PathBuf,
    /// When the script should run.
    pub when: ScriptWhen,
    /// What dfiles migration was detected in the script body.
    pub migration: ScriptMigration,
}

/// When a chezmoi script runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptWhen {
    /// `run_once_` / `once_` — run one time only.
    Once,
    /// `run_` — run on every apply.
    Always,
}

/// What was detected inside a chezmoi script.
#[derive(Debug, Clone)]
pub enum ScriptMigration {
    /// `brew bundle --file=<path>` was found — emit a `[homebrew]` module section.
    BrewBundle { brewfile_path: String },
    /// `mise install` was found — emit a `[mise]` module section.
    MiseInstall,
    /// Nothing recognisable — skip with a manual-migration note.
    Unrecognized,
}

/// A chezmoi source Brewfile that should be copied to `brew/` instead of `source/`.
///
/// Detected by filename anywhere in the decoded destination path:
/// `Brewfile`, `.Brewfile`, `Brewfile.<suffix>` (not `.lock`).
#[derive(Debug, Clone)]
pub struct ChezmoiBrewfileEntry {
    /// Relative path inside the chezmoi source directory.
    pub chezmoi_path: PathBuf,
    /// Decoded absolute destination path with tilde (e.g. `"~/config/Brewfile"`).
    pub dest_tilde: String,
    /// Target path inside brew/ (e.g. `"brew/Brewfile.packages"`).
    pub brew_dest: String,
    /// Module name inferred from filename (e.g. `"packages"`).
    pub module_name: String,
}

/// A chezmoi external that should become a dfiles `[[externals]]` entry.
#[derive(Debug, Clone)]
pub struct ChezmoiExternalEntry {
    /// Destination path with tilde (e.g. `"~/.config/nvim"`).
    pub dest_tilde: String,
    /// Source type — `"git"` or `"archive"`.
    pub kind: String,
    /// Remote URL.
    pub url: String,
    /// Branch/tag/SHA from the `ref` field. Optional.
    pub ref_name: Option<String>,
    /// Inferred dfiles module name.
    pub module: String,
}

/// Return value of `decode_entry`: every file either keeps or skips.
pub enum ImportEntry {
    Keep(ChezmoiEntry),
    Skip(SkippedEntry),
}

// ─── Source directory detection ───────────────────────────────────────────────

/// Locate the chezmoi source directory.
///
/// Detection order:
/// 1. `override_path` (--source flag) — used as-is.
/// 2. `chezmoi source-path` subprocess (if `chezmoi` is on PATH).
/// 3. `~/.local/share/chezmoi` XDG default.
/// 4. Hard error if none of the above exists.
pub fn detect_source_dir(override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.to_path_buf());
    }

    // Try subprocess.
    if let Some(path) = try_chezmoi_source_path() {
        return Ok(path);
    }

    // XDG fallback.
    let fallback = dirs::home_dir()
        .context("Cannot determine home directory")?
        .join(".local")
        .join("share")
        .join("chezmoi");

    if fallback.exists() {
        return Ok(fallback);
    }

    anyhow::bail!(
        "chezmoi source directory not found.\n\
         Use --source <path> to specify it, or run `chezmoi init` first."
    )
}

/// Run `chezmoi source-path` and return the path if successful.
fn try_chezmoi_source_path() -> Option<PathBuf> {
    let output = std::process::Command::new("chezmoi")
        .arg("source-path")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let path_str = String::from_utf8(output.stdout).ok()?;
    let path = PathBuf::from(path_str.trim());
    if path.exists() { Some(path) } else { None }
}

// ─── Scanner ──────────────────────────────────────────────────────────────────

/// Ask chezmoi for the set of paths it actually manages on this machine.
///
/// Returns a set of tilde-prefixed paths like `~/.config/fish/fish_plugins`.
/// Returns `None` if chezmoi is not on PATH or the command fails (caller falls
/// back to importing everything without filtering).
/// Returns the set of source-relative paths that chezmoi manages for `source_dir`.
///
/// Uses `--path-style source-relative` so the returned paths match `ChezmoiEntry::chezmoi_path`
/// directly (e.g. `"dot_zshrc"`, `"private_dot_ssh/id_rsa"`).
///
/// Returns `None` if chezmoi is not on PATH, if it fails, or if the managed list
/// is empty (empty = chezmoi has no state for this source, so filtering is meaningless).
fn chezmoi_managed_paths(source_dir: &Path) -> Option<std::collections::HashSet<String>> {
    let output = std::process::Command::new("chezmoi")
        .args(["managed", "--include=files,symlinks", "--path-style", "source-relative", "-S"])
        .arg(source_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let paths: std::collections::HashSet<String> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .collect();
    // If managed returns an empty list it likely means chezmoi has no applied files
    // in this context (e.g. a custom --source dir). Skip filtering rather than
    // treating everything as unmanaged.
    if paths.is_empty() {
        return None;
    }
    Some(paths)
}

/// Walk `source_dir` and decode every file into a Keep or Skip entry.
/// Also parses `.chezmoiexternal.toml` (if present) into external entries.
///
/// Files that chezmoi ignores (via `.chezmoiignore`) are moved to skips with
/// `SkipReason::Ignored`. When chezmoi is on PATH, its `managed` list is used;
/// otherwise `.chezmoiignore` is parsed directly (Go template lines are stripped).
///
/// Pass `include_ignored = true` to skip this filtering entirely — all files will
/// be in keeps regardless of `.chezmoiignore`.
///
/// Directories starting with `.` are skipped entirely (they are chezmoi-internal
/// or system directories — legitimate dotfile dirs use the `dot_` prefix).
pub fn scan(source_dir: &Path, include_ignored: bool) -> Result<(Vec<ChezmoiEntry>, Vec<ChezmoiExternalEntry>, Vec<SkippedEntry>, Vec<ChezmoiScriptEntry>, Vec<ChezmoiBrewfileEntry>)> {
    let managed = chezmoi_managed_paths(source_dir);

    let mut keeps: Vec<ChezmoiEntry> = Vec::new();
    let mut skips: Vec<SkippedEntry> = Vec::new();
    let mut scripts: Vec<ChezmoiScriptEntry> = Vec::new();
    let mut brewfiles: Vec<ChezmoiBrewfileEntry> = Vec::new();

    let walker = WalkDir::new(source_dir)
        .min_depth(1)
        .sort_by_file_name() // lexicographic order → stable collision suffixes
        .into_iter()
        .filter_entry(|e| {
            // Skip any directory whose name starts with '.'.
            // Legitimate chezmoi dotfile dirs use the dot_ prefix, not a leading dot.
            if e.file_type().is_dir() {
                let name = e.file_name().to_string_lossy();
                return !name.starts_with('.');
            }
            true
        });

    for entry in walker {
        let entry = entry.with_context(|| {
            format!("Error walking chezmoi source dir {}", source_dir.display())
        })?;

        // Skip directory entries — we only process files.
        if entry.file_type().is_dir() {
            continue;
        }

        let rel = entry
            .path()
            .strip_prefix(source_dir)
            .expect("walkdir always returns paths under root");

        match decode_entry(rel) {
            ImportEntry::Keep(mut e) => {
                // For template entries, read the source file and convert now while
                // we still know where it lives (`source_dir/chezmoi_path`).
                if e.template {
                    let abs = source_dir.join(&e.chezmoi_path);
                    match std::fs::read_to_string(&abs) {
                        Ok(content) => {
                            let result = crate::chezmoi_template::convert(&content);
                            e.converted_content = Some(result.text);
                            e.template_warnings = result.warnings;
                        }
                        Err(err) => {
                            // Can't read the file — fall back to skip.
                            skips.push(SkippedEntry {
                                chezmoi_path: e.chezmoi_path.clone(),
                                reason: SkipReason::Template,
                            });
                            eprintln!(
                                "warning: cannot read template {}: {}",
                                e.chezmoi_path.display(),
                                err
                            );
                            continue;
                        }
                    }
                }
                // Check if the decoded dest is a Brewfile — redirect to brew/ instead of source/.
                let dest_filename = Path::new(&e.dest_tilde)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if is_brewfile_name(dest_filename) {
                    brewfiles.push(ChezmoiBrewfileEntry {
                        brew_dest: brewfile_brew_dest(dest_filename),
                        module_name: brewfile_module_name(dest_filename),
                        chezmoi_path: e.chezmoi_path,
                        dest_tilde: e.dest_tilde,
                    });
                } else {
                    keeps.push(e);
                }
            }
            ImportEntry::Skip(SkippedEntry { chezmoi_path, reason: SkipReason::Symlink }) => {
                // Try to resolve the symlink target from the file's content.
                let abs = source_dir.join(&chezmoi_path);
                if let Some(entry) = try_resolve_symlink(&abs, &chezmoi_path) {
                    keeps.push(entry);
                } else {
                    skips.push(SkippedEntry { chezmoi_path, reason: SkipReason::Symlink });
                }
            }
            ImportEntry::Skip(SkippedEntry { chezmoi_path, reason: SkipReason::Script }) => {
                // Try to detect a recognised migration pattern in the script body.
                let abs = source_dir.join(&chezmoi_path);
                let when = script_when(&chezmoi_path);
                match std::fs::read_to_string(&abs) {
                    Ok(content) => {
                        let migration = detect_script_migration(&content);
                        // All scripts go into `scripts` — unrecognised ones still get
                        // copied to source/scripts/ for execution on apply.
                        scripts.push(ChezmoiScriptEntry { chezmoi_path, when, migration });
                    }
                    Err(_) => {
                        // Can't read the script — skip it.
                        skips.push(SkippedEntry { chezmoi_path, reason: SkipReason::Script });
                    }
                }
            }
            ImportEntry::Skip(e) => skips.push(e),
        }
    }

    // Filter keeps to only files that chezmoi actually manages on this system,
    // moving ignored entries to skips so --include-ignored-files can surface them.
    if !include_ignored {
        if let Some(ref managed_paths) = managed {
            // chezmoi is on PATH — use its output to determine what is managed.
            let (kept, ignored): (Vec<_>, Vec<_>) = keeps
                .into_iter()
                .partition(|e| managed_paths.contains(&e.chezmoi_path.to_string_lossy().into_owned()));
            keeps = kept;
            for e in ignored {
                skips.push(SkippedEntry { chezmoi_path: e.chezmoi_path, reason: SkipReason::Ignored });
            }
        } else {
            // chezmoi not on PATH — parse .chezmoiignore directly (strip Go template lines).
            let ignore_file = source_dir.join(".chezmoiignore");
            if ignore_file.exists() {
                if let Ok(content) = std::fs::read_to_string(&ignore_file) {
                    let ignore = crate::ignore::IgnoreList::from_chezmoi_ignore(&content);
                    let (kept, ignored): (Vec<_>, Vec<_>) = keeps
                        .into_iter()
                        .partition(|e| !ignore.is_ignored(&e.dest_tilde));
                    keeps = kept;
                    for e in ignored {
                        skips.push(SkippedEntry {
                            chezmoi_path: e.chezmoi_path,
                            reason: SkipReason::Ignored,
                        });
                    }
                }
            }
        }
    }

    let externals = parse_chezmoiexternal(source_dir)?;

    Ok((keeps, externals, skips, scripts, brewfiles))
}

// ─── .chezmoiexternal.toml parsing ───────────────────────────────────────────

/// Raw deserialization shape for a single entry in `.chezmoiexternal.toml`.
#[derive(Debug, Deserialize)]
struct RawExternalEntry {
    #[serde(rename = "type")]
    kind: String,
    url: Option<String>,
    #[serde(rename = "ref")]
    ref_name: Option<String>,
}

/// Parse `.chezmoiexternal.toml` from the chezmoi source directory.
///
/// The file format is a TOML table where each key is the destination path:
///
/// ```toml
/// ["~/.config/nvim"]
/// type = "git-repo"
/// url  = "https://github.com/user/nvim-config"
/// ref  = "main"
/// ```
///
/// Returns an empty Vec if the file does not exist or is empty.
fn parse_chezmoiexternal(source_dir: &Path) -> Result<Vec<ChezmoiExternalEntry>> {
    let path = source_dir.join(".chezmoiexternal.toml");
    if !path.exists() {
        return Ok(Vec::new());
    }

    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Cannot read {}", path.display()))?;

    let table: toml::Table = toml::from_str(&text)
        .with_context(|| format!("Invalid TOML in {}", path.display()))?;

    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    let mut entries = Vec::new();

    for (dest_raw, value) in &table {
        let raw: RawExternalEntry = value
            .clone()
            .try_into()
            .with_context(|| format!("Invalid external entry for '{}'", dest_raw))?;

        // Map chezmoi "git-repo" → dfiles "git"; anything else is passed through.
        let kind = if raw.kind == "git-repo" {
            "git".to_string()
        } else {
            raw.kind.clone()
        };

        let url = match raw.url {
            Some(u) => u,
            None => {
                eprintln!(
                    "warning: .chezmoiexternal.toml: '{}' has no url — skipped",
                    dest_raw
                );
                continue;
            }
        };

        // Decode dest: expand ~ and convert to tilde string.
        let dest_abs = if let Some(rest) = dest_raw.strip_prefix("~/") {
            home.join(rest)
        } else if dest_raw == "~" {
            home.clone()
        } else {
            PathBuf::from(dest_raw)
        };
        let dest_tilde = crate::fs::tilde_path(&dest_abs);

        let module = infer_module(&dest_abs).to_string();

        entries.push(ChezmoiExternalEntry {
            dest_tilde,
            kind,
            url,
            ref_name: raw.ref_name,
            module,
        });
    }

    Ok(entries)
}

// ─── Entry decoding ───────────────────────────────────────────────────────────

/// Strip `private_` and `executable_` prefixes from a path component (in any order,
/// stackable). Returns `(stripped, is_private, is_executable)`.
///
/// Examples:
/// - `"private_dot_ssh"` → `("dot_ssh", true, false)`
/// - `"executable_deploy.sh"` → `("deploy.sh", false, true)`
/// - `"private_executable_dot_local"` → `("dot_local", true, true)`
/// - `"executable_private_dot_local"` → `("dot_local", true, true)`
fn strip_permission_prefixes(s: &str) -> (&str, bool, bool) {
    let mut rest = s;
    let mut private = false;
    let mut executable = false;
    loop {
        if let Some(r) = rest.strip_prefix("private_") {
            private = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("executable_") {
            executable = true;
            rest = r;
        } else {
            break;
        }
    }
    (rest, private, executable)
}

/// Decode a single chezmoi relative path into a Keep or Skip entry.
///
/// Prefix processing order for the first path component:
///   private_ / executable_ (any order, stackable) → strip, record flags
///   symlink_  > exact_ / create_ / modify_  > run_once_ / run_ / once_
///   > .tmpl suffix  > .chezmoi* / chezmoistate*  > dot_<name>  > <bare name>
pub fn decode_entry(rel_path: &Path) -> ImportEntry {
    let first = match rel_path.components().next().and_then(|c| c.as_os_str().to_str()) {
        Some(s) => s,
        None => return skip(rel_path, SkipReason::Internal),
    };

    // Strip private_/executable_ prefixes first.
    let (first_stripped, is_private, is_executable) = strip_permission_prefixes(first);

    // chezmoi-internal files — silent skip.
    if first_stripped.starts_with(".chezmoi") || first_stripped == "chezmoistate.boltdb" {
        return skip(rel_path, SkipReason::Internal);
    }

    // Remaining unsupported prefix checks (after permission prefixes are stripped).
    if first_stripped.starts_with("symlink_") {
        return skip(rel_path, SkipReason::Symlink);
    }
    if first_stripped.starts_with("modify_") {
        return skip(rel_path, SkipReason::UnsupportedAttribute);
    }
    // exact_: supported — the prefix is kept in source/ so that source.rs sets
    // exact on the SourceDir at apply time. apply.rs then purges untracked entries.
    // create_: supported — prefix kept in source/; apply skips write if dest exists.
    if first_stripped.starts_with("run_once_")
        || first_stripped.starts_with("run_")
        || first_stripped.starts_with("once_")
    {
        return skip(rel_path, SkipReason::Script);
    }

    // Template suffix check (applied to the full path, not just first component).
    // Attempt conversion; only skip if the file can't be read at all.
    let path_str = rel_path.to_string_lossy();
    if path_str.ends_with(".tmpl") {
        return decode_template_entry(rel_path, is_private, is_executable);
    }

    // Decode dest path.
    let dest_abs = decode_dest(rel_path);
    let dest_tilde = crate::fs::tilde_path(&dest_abs);
    let sname = source_name(rel_path);
    let module = infer_module(&dest_abs).to_string();

    ImportEntry::Keep(ChezmoiEntry {
        chezmoi_path: rel_path.to_path_buf(),
        dest_tilde,
        source_name: sname,
        module,
        private: is_private,
        executable: is_executable,
        link: false,
        template: false,
        converted_content: None,
        template_warnings: Vec::new(),
        copy_from: None,
    })
}

/// Decode a chezmoi relative path to the absolute destination path.
///
/// Strips `private_` / `executable_` prefixes and `dot_` prefix from the first
/// path component. Subsequent components are used as-is.
///
/// - `private_dot_ssh/config` → `~/.ssh/config`
/// - `executable_dot_local/bin/foo` → `~/.local/bin/foo`
/// - `dot_zshrc` → `~/.zshrc`
/// - `Justfile` → `~/Justfile`
fn decode_dest(rel_path: &Path) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));

    let mut components: Vec<String> = rel_path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();

    // Strip permission prefixes then dot_ from the first component only.
    if let Some(first) = components.first_mut() {
        let (stripped, _, _) = strip_permission_prefixes(first);
        // Also strip create_ and exact_ for dest calculation — prefixes stay in source_name.
        let stripped = stripped.strip_prefix("create_").unwrap_or(stripped);
        let stripped = stripped.strip_prefix("exact_").unwrap_or(stripped);
        if let Some(rest) = stripped.strip_prefix("dot_") {
            *first = format!(".{}", rest);
        } else {
            *first = stripped.to_string();
        }
    }

    let mut path = home;
    for component in &components {
        path = path.join(component);
    }
    path
}

/// Infer the dfiles module from the decoded destination path.
///
/// Rules checked in order; first match wins. All matching is
/// `starts_with` on the canonical path string after `~/` expansion.
fn infer_module(dest: &Path) -> &'static str {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    let s = dest.to_string_lossy();
    // Append `/` so that directory destinations (e.g. `~/.config/nvim`) match
    // directory prefix patterns (e.g. `~/.config/nvim/`) without also matching
    // `~/.config/nvim-extra`.
    let s_slash = if s.ends_with('/') {
        s.to_string()
    } else {
        format!("{}/", s)
    };

    let h = home.to_string_lossy();

    // Shell configs.
    for prefix in &[
        format!("{}/.zshrc", h),
        format!("{}/.zshenv", h),
        format!("{}/.zprofile", h),
        format!("{}/.profile", h),
        format!("{}/.bashrc", h),
        format!("{}/.bash_profile", h),
        format!("{}/.bash_login", h),
    ] {
        if s.as_ref() == prefix.as_str() {
            return "shell";
        }
    }
    for prefix in &[
        format!("{}/.zsh", h),
        format!("{}/.bash", h),
        format!("{}/.tmux", h),
        format!("{}/.config/tmux/", h),
        format!("{}/.config/alacritty/", h),
        format!("{}/.config/kitty/", h),
        format!("{}/.config/wezterm/", h),
    ] {
        if s_slash.starts_with(prefix.as_str()) {
            // Exclude runtime data files that chezmoi shouldn't contain.
            if s.contains("_history") || s.contains("_sessions") || s.contains("compdump") {
                break;
            }
            return "shell";
        }
    }

    // Git configs.
    for prefix in &[
        format!("{}/.gitconfig", h),
        format!("{}/.gitignore", h),
        format!("{}/.config/git/", h),
    ] {
        if s_slash.starts_with(prefix.as_str()) {
            return "git";
        }
    }

    // Editor configs.
    for prefix in &[
        format!("{}/.vimrc", h),
        format!("{}/.config/nvim/", h),
        format!("{}/.config/vim/", h),
        format!("{}/.config/helix/", h),
        format!("{}/.config/zed/", h),
    ] {
        if s_slash.starts_with(prefix.as_str()) {
            return "editor";
        }
    }

    "misc"
}

/// Derive the dfiles `source/` path from a chezmoi relative path.
///
/// The chezmoi path IS the dfiles magic-name encoding — they share the same
/// convention (`private_`, `executable_`, `dot_`, `.tmpl`). So the source path
/// is simply the chezmoi path as-is. The only difference is for templates: the
/// `.tmpl` suffix is kept (dfiles also uses `.tmpl` to mark Tera templates).
///
/// The caller passes the path already stripped of `.tmpl` for template entries
/// (see `decode_template_entry`). For regular entries, the path is used verbatim.
pub fn source_name(rel_path: &Path) -> String {
    rel_path.to_string_lossy().into_owned()
}

// ─── Template entry decoder ───────────────────────────────────────────────────

/// Decode a `.tmpl`-suffixed chezmoi file into a dfiles `template = true` entry.
///
/// Reads the file from `source_dir` (via the scan context), converts the Go
/// template syntax to Tera, and returns a `Keep` entry with `template = true`
/// and `converted_content` set to the converted text.
///
/// If the file cannot be read from disk at decode time (we don't have the source
/// dir path here — the scan loop calls us for this), we return a `Skip(Template)`
/// so the caller can fall back to the old behaviour.
///
/// The `.tmpl` suffix is stripped from both the source name and the decoded
/// destination path, because the template extension is a chezmoi convention that
/// should not appear in the deployed file name.
fn decode_template_entry(
    rel_path: &Path,
    is_private: bool,
    is_executable: bool,
) -> ImportEntry {
    // Strip `.tmpl` from the last component of rel_path to decode the destination.
    // The source_name keeps `.tmpl` so dfiles recognises it as a template in source/.
    let stripped_path = strip_tmpl_suffix(rel_path);
    let stripped_path = stripped_path.as_deref().unwrap_or(rel_path);

    let dest_abs = decode_dest(stripped_path);
    let dest_tilde = crate::fs::tilde_path(&dest_abs);
    // source_name uses the original path (WITH .tmpl) — dfiles uses the suffix too.
    let sname = source_name(rel_path);
    let module = infer_module(&dest_abs).to_string();

    ImportEntry::Keep(ChezmoiEntry {
        chezmoi_path: rel_path.to_path_buf(),
        dest_tilde,
        source_name: sname,
        module,
        private: is_private,
        executable: is_executable,
        link: false,
        template: true,          // content written in import.rs after reading & converting
        converted_content: None, // populated in scan() after reading the file
        template_warnings: Vec::new(),
        copy_from: None,
    })
}

/// Strip the `.tmpl` suffix from the last component of a path.
///
/// `dot_zshrc.tmpl`        → `Some(dot_zshrc)`
/// `dot_config/git/config.tmpl` → `Some(dot_config/git/config)`
/// `dot_zshrc`             → `None` (no suffix to strip)
fn strip_tmpl_suffix(rel_path: &Path) -> Option<PathBuf> {
    let last = rel_path.file_name()?.to_str()?;
    let stripped = last.strip_suffix(".tmpl")?;
    Some(rel_path.with_file_name(stripped))
}

// ─── helpers ──────────────────────────────────────────────────────────────────

fn skip(rel_path: &Path, reason: SkipReason) -> ImportEntry {
    ImportEntry::Skip(SkippedEntry {
        chezmoi_path: rel_path.to_path_buf(),
        reason,
    })
}

/// Attempt to resolve a chezmoi `symlink_` file into a dfiles linked entry.
///
/// The chezmoi `symlink_` file's content is the symlink target path. If that
/// target exists as a regular file on disk, we import it with `link = true` and
/// set `copy_from` to the target so `import` copies the right file.
///
/// Returns `None` if the target cannot be resolved (Go template content,
/// non-existent path, or a directory).
fn try_resolve_symlink(
    chezmoi_file: &Path,
    rel_path: &Path,
) -> Option<ChezmoiEntry> {
    // Read the file content — this is the symlink target path.
    let content = std::fs::read_to_string(chezmoi_file).ok()?;
    let target_str = content.trim();

    // Reject Go template expressions.
    if target_str.contains("{{") {
        return None;
    }

    // Expand ~ and resolve the target path.
    let target = crate::config::module::expand_tilde(target_str).ok()?;
    if !target.exists() || target.is_dir() {
        return None;
    }

    // Decode the destination by stripping `symlink_` from the first component.
    let dest_abs = decode_symlink_dest(rel_path);
    let dest_tilde = crate::fs::tilde_path(&dest_abs);
    let sname = source_name(rel_path);
    let module = infer_module(&dest_abs).to_string();

    Some(ChezmoiEntry {
        chezmoi_path: rel_path.to_path_buf(),
        dest_tilde,
        source_name: sname,
        module,
        private: false,
        executable: false,
        link: true,
        template: false,
        converted_content: None,
        template_warnings: Vec::new(),
        copy_from: Some(target),
    })
}

/// Decode the destination path for a `symlink_` entry by stripping the `symlink_`
/// prefix from the first path component, then decoding as normal.
fn decode_symlink_dest(rel_path: &Path) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));

    let mut components: Vec<String> = rel_path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();

    if let Some(first) = components.first_mut() {
        // Strip `symlink_` then any permission prefixes, then `dot_`.
        let without_symlink = first
            .strip_prefix("symlink_")
            .unwrap_or(first.as_str());
        let (stripped, _, _) = strip_permission_prefixes(without_symlink);
        if let Some(rest) = stripped.strip_prefix("dot_") {
            *first = format!(".{}", rest);
        } else {
            *first = stripped.to_string();
        }
    }

    let mut path = home;
    for component in &components {
        path = path.join(component);
    }
    path
}

// ─── Brewfile detection helpers ───────────────────────────────────────────────

/// Returns `true` if `name` matches a Brewfile naming pattern.
///
/// Patterns: `Brewfile`, `.Brewfile`, `Brewfile.<suffix>` (excluding `.lock`).
pub fn is_brewfile_name(name: &str) -> bool {
    if name == "Brewfile" || name == ".Brewfile" {
        return true;
    }
    if let Some(suffix) = name.strip_prefix("Brewfile.") {
        return !suffix.is_empty() && suffix != "lock";
    }
    false
}

/// Compute the `brew/` destination path for a Brewfile.
///
/// - `Brewfile` / `.Brewfile` → `"brew/Brewfile.packages"`
/// - `Brewfile.<suffix>` → `"brew/Brewfile.<suffix>"`
pub fn brewfile_brew_dest(filename: &str) -> String {
    let base = filename.trim_start_matches('.');
    if base == "Brewfile" {
        "brew/Brewfile.packages".to_string()
    } else if let Some(suffix) = base.strip_prefix("Brewfile.") {
        format!("brew/Brewfile.{}", suffix)
    } else {
        "brew/Brewfile.packages".to_string()
    }
}

/// Infer the module name from a Brewfile filename.
///
/// - `Brewfile` / `.Brewfile` → `"packages"`
/// - `Brewfile.<suffix>` → `"<suffix>"`
pub fn brewfile_module_name(filename: &str) -> String {
    let base = filename.trim_start_matches('.');
    if base == "Brewfile" {
        "packages".to_string()
    } else if let Some(suffix) = base.strip_prefix("Brewfile.") {
        suffix.to_string()
    } else {
        "packages".to_string()
    }
}

// ─── Script migration detection ───────────────────────────────────────────────

/// Determine when a script should run from its filename prefix.
fn script_when(rel_path: &Path) -> ScriptWhen {
    let name = rel_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let (stripped, _, _) = strip_permission_prefixes(name);
    if stripped.starts_with("run_once_") || stripped.starts_with("once_") {
        ScriptWhen::Once
    } else {
        ScriptWhen::Always
    }
}

/// Scan a script's content and return the best-matching migration.
///
/// Detection order (first match wins):
/// 1. `brew bundle --file=<path>` → `BrewBundle { brewfile_path }`
/// 2. `brew bundle` (no --file) → `BrewBundle { brewfile_path: "Brewfile" }`
/// 3. `mise install` → `MiseInstall`
/// 4. No match → `Unrecognized`
pub fn detect_script_migration(content: &str) -> ScriptMigration {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue; // skip comments
        }

        // brew bundle [--file=<path>]
        if let Some(idx) = trimmed.find("brew bundle") {
            let after = trimmed[idx + "brew bundle".len()..].trim();
            if let Some(path) = extract_brew_bundle_file(after) {
                return ScriptMigration::BrewBundle { brewfile_path: path };
            }
            // bare `brew bundle` with no --file flag
            if after.is_empty() || after.starts_with('#') || after.starts_with("--") {
                return ScriptMigration::BrewBundle {
                    brewfile_path: "Brewfile".to_string(),
                };
            }
        }

        // mise install
        if trimmed.contains("mise install") {
            return ScriptMigration::MiseInstall;
        }
    }

    ScriptMigration::Unrecognized
}

/// Extract the path argument from `--file=<path>` or `--file <path>`.
fn extract_brew_bundle_file(rest: &str) -> Option<String> {
    if let Some(after) = rest.strip_prefix("--file=") {
        let path = after.split_whitespace().next()?.trim_matches(|c| c == '"' || c == '\'');
        if !path.is_empty() {
            return Some(path.to_string());
        }
    } else if let Some(after) = rest.strip_prefix("--file ") {
        let path = after.trim_start().split_whitespace().next()?.trim_matches(|c| c == '"' || c == '\'');
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }
    None
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn keep(rel: &str) -> ChezmoiEntry {
        match decode_entry(Path::new(rel)) {
            ImportEntry::Keep(e) => e,
            ImportEntry::Skip(s) => panic!(
                "Expected Keep for '{}', got Skip({:?})",
                rel, s.reason
            ),
        }
    }

    fn skip_reason(rel: &str) -> SkipReason {
        match decode_entry(Path::new(rel)) {
            ImportEntry::Skip(e) => e.reason,
            ImportEntry::Keep(e) => panic!(
                "Expected Skip for '{}', got Keep(dest={})",
                rel, e.dest_tilde
            ),
        }
    }

    // ── decode_entry: keep cases ───────────────────────────────────────────────

    #[test]
    fn decode_dot_zshrc() {
        let e = keep("dot_zshrc");
        assert!(e.dest_tilde.ends_with("/.zshrc"), "dest={}", e.dest_tilde);
        assert_eq!(e.module, "shell");
        assert_eq!(e.source_name, "dot_zshrc");
    }

    #[test]
    fn decode_dot_config_git_config() {
        let e = keep("dot_config/git/config");
        assert!(e.dest_tilde.ends_with("/.config/git/config"), "dest={}", e.dest_tilde);
        assert_eq!(e.module, "git");
        assert_eq!(e.source_name, "dot_config/git/config");
    }

    #[test]
    fn decode_dot_tmux_conf() {
        let e = keep("dot_tmux.conf");
        assert!(e.dest_tilde.ends_with("/.tmux.conf"), "dest={}", e.dest_tilde);
        assert_eq!(e.module, "shell");
    }

    #[test]
    fn decode_dot_config_nvim_init_lua() {
        let e = keep("dot_config/nvim/init.lua");
        assert!(e.dest_tilde.ends_with("/.config/nvim/init.lua"), "dest={}", e.dest_tilde);
        assert_eq!(e.module, "editor");
    }

    #[test]
    fn decode_dot_finicky_js() {
        let e = keep("dot_finicky.js");
        assert!(e.dest_tilde.ends_with("/.finicky.js"), "dest={}", e.dest_tilde);
        assert_eq!(e.module, "misc");
    }

    #[test]
    fn decode_bare_justfile() {
        let e = keep("Justfile");
        assert!(e.dest_tilde.ends_with("/Justfile"), "dest={}", e.dest_tilde);
        assert_eq!(e.module, "misc");
        assert_eq!(e.source_name, "Justfile"); // no dot_ prefix — stays as-is
    }

    #[test]
    fn decode_bare_bin_mybin() {
        let e = keep("bin/mybin");
        assert!(e.dest_tilde.ends_with("/bin/mybin"), "dest={}", e.dest_tilde);
        assert_eq!(e.module, "misc");
        assert_eq!(e.source_name, "bin/mybin");
    }

    #[test]
    fn decode_bare_settings_json() {
        let e = keep("settings.json");
        assert!(e.dest_tilde.ends_with("/settings.json"), "dest={}", e.dest_tilde);
        assert_eq!(e.module, "misc");
    }

    // ── decode_entry: private_ prefix ─────────────────────────────────────────

    #[test]
    fn decode_private_dot_ssh_config() {
        let e = keep("private_dot_ssh/config");
        assert!(e.dest_tilde.ends_with("/.ssh/config"), "dest={}", e.dest_tilde);
        assert_eq!(e.source_name, "private_dot_ssh/config");
        assert!(e.private, "expected private=true");
        assert!(!e.executable, "expected executable=false");
    }

    #[test]
    fn decode_private_dot_ssh_file() {
        let e = keep("private_dot_ssh");
        assert!(e.dest_tilde.ends_with("/.ssh"), "dest={}", e.dest_tilde);
        assert!(e.private);
        assert!(!e.executable);
    }

    // ── decode_entry: executable_ prefix ──────────────────────────────────────

    #[test]
    fn decode_executable_dot_local_bin_foo() {
        let e = keep("executable_dot_local/bin/foo");
        assert!(e.dest_tilde.ends_with("/.local/bin/foo"), "dest={}", e.dest_tilde);
        assert_eq!(e.source_name, "executable_dot_local/bin/foo");
        assert!(!e.private, "expected private=false");
        assert!(e.executable, "expected executable=true");
    }

    #[test]
    fn decode_executable_bare_script() {
        let e = keep("executable_deploy.sh");
        assert!(e.dest_tilde.ends_with("/deploy.sh"), "dest={}", e.dest_tilde);
        assert_eq!(e.source_name, "executable_deploy.sh");
        assert!(e.executable);
        assert!(!e.private);
    }

    // ── decode_entry: combined prefixes ───────────────────────────────────────

    #[test]
    fn decode_private_executable_combined() {
        let e = keep("private_executable_dot_local/bin/secret");
        assert!(e.dest_tilde.ends_with("/.local/bin/secret"), "dest={}", e.dest_tilde);
        assert!(e.private, "expected private=true");
        assert!(e.executable, "expected executable=true");
    }

    #[test]
    fn decode_executable_private_combined_reversed_order() {
        let e = keep("executable_private_dot_local/bin/secret");
        assert!(e.dest_tilde.ends_with("/.local/bin/secret"), "dest={}", e.dest_tilde);
        assert!(e.private, "expected private=true");
        assert!(e.executable, "expected executable=true");
    }

    // ── decode_entry: neither flag set for plain files ─────────────────────────

    #[test]
    fn decode_plain_file_has_no_flags() {
        let e = keep("dot_zshrc");
        assert!(!e.private);
        assert!(!e.executable);
    }

    // ── decode_entry: skip cases ───────────────────────────────────────────────

    #[test]
    fn skip_symlink_dot_vim() {
        assert_eq!(skip_reason("symlink_dot_vim"), SkipReason::Symlink);
    }

    #[test]
    fn exact_dot_config_decodes_to_config() {
        // exact_ is now supported — it decodes like a regular dir prefix.
        let e = keep("exact_dot_config/fish/config.fish");
        assert!(e.dest_tilde.ends_with("/.config/fish/config.fish"), "dest={}", e.dest_tilde);
        assert_eq!(e.source_name, "exact_dot_config/fish/config.fish");
    }

    #[test]
    fn dot_zshrc_tmpl_decodes_as_template_entry() {
        // `.tmpl` files are now converted rather than skipped.
        // `converted_content` is None here because decode_entry doesn't read files —
        // the scan() loop populates it. We just verify the structural decoding.
        let e = keep("dot_zshrc.tmpl");
        assert!(e.dest_tilde.ends_with("/.zshrc"), "expected dest ~/.zshrc, got {}", e.dest_tilde);
        // source_name keeps the .tmpl suffix — dfiles source/ uses it too.
        assert_eq!(e.source_name, "dot_zshrc.tmpl", "expected source_name with .tmpl suffix");
        assert!(e.template, "expected template=true");
    }

    #[test]
    fn skip_run_once_setup_sh() {
        assert_eq!(skip_reason("run_once_setup.sh"), SkipReason::Script);
    }

    #[test]
    fn skip_once_setup_sh() {
        assert_eq!(skip_reason("once_setup.sh"), SkipReason::Script);
    }

    #[test]
    fn skip_chezmoistate_boltdb() {
        assert_eq!(skip_reason("chezmoistate.boltdb"), SkipReason::Internal);
    }

    #[test]
    fn skip_chezmoi_toml_tmpl() {
        assert_eq!(skip_reason(".chezmoi.toml.tmpl"), SkipReason::Internal);
    }

    // ── source_name: encoded path preserved as-is ─────────────────────────────

    #[test]
    fn source_name_preserves_encoding() {
        assert_eq!(source_name(Path::new("dot_zshrc")), "dot_zshrc");
    }

    #[test]
    fn source_name_nested_preserves_encoding() {
        assert_eq!(source_name(Path::new("dot_config/git/config")), "dot_config/git/config");
    }

    #[test]
    fn source_name_bare_file_no_strip() {
        assert_eq!(source_name(Path::new("Justfile")), "Justfile");
    }

    #[test]
    fn source_name_nested_bare_path() {
        assert_eq!(source_name(Path::new("bin/mybin")), "bin/mybin");
    }

    #[test]
    fn source_name_template_keeps_tmpl_suffix() {
        assert_eq!(source_name(Path::new("dot_gitconfig.tmpl")), "dot_gitconfig.tmpl");
    }

    // ── create_: decoded as Keep, dest strips create_ prefix ──────────────────

    // ── detect_script_migration ────────────────────────────────────────────────

    #[test]
    fn detects_brew_bundle_with_file_flag() {
        let content = "#!/bin/bash\nbrew bundle --file=~/dotfiles/Brewfile\n";
        match detect_script_migration(content) {
            ScriptMigration::BrewBundle { brewfile_path } => {
                assert_eq!(brewfile_path, "~/dotfiles/Brewfile");
            }
            other => panic!("expected BrewBundle, got {:?}", other),
        }
    }

    #[test]
    fn detects_brew_bundle_bare() {
        let content = "#!/bin/bash\nbrew bundle\n";
        match detect_script_migration(content) {
            ScriptMigration::BrewBundle { brewfile_path } => {
                assert_eq!(brewfile_path, "Brewfile");
            }
            other => panic!("expected BrewBundle, got {:?}", other),
        }
    }

    #[test]
    fn detects_mise_install() {
        let content = "#!/bin/bash\nmise install\n";
        match detect_script_migration(content) {
            ScriptMigration::MiseInstall => {}
            other => panic!("expected MiseInstall, got {:?}", other),
        }
    }

    #[test]
    fn detects_nothing_for_unrecognised_script() {
        let content = "#!/bin/bash\necho hello\n";
        match detect_script_migration(content) {
            ScriptMigration::Unrecognized => {}
            other => panic!("expected Unrecognized, got {:?}", other),
        }
    }

    #[test]
    fn skips_commented_brew_bundle_line() {
        let content = "#!/bin/bash\n# brew bundle --file=Brewfile\necho done\n";
        match detect_script_migration(content) {
            ScriptMigration::Unrecognized => {}
            other => panic!("expected Unrecognized for commented line, got {:?}", other),
        }
    }

    #[test]
    fn script_when_run_once_prefix() {
        assert_eq!(script_when(Path::new("run_once_setup.sh")), ScriptWhen::Once);
    }

    #[test]
    fn script_when_run_prefix() {
        assert_eq!(script_when(Path::new("run_setup.sh")), ScriptWhen::Always);
    }

    #[test]
    fn script_when_once_prefix() {
        assert_eq!(script_when(Path::new("once_setup.sh")), ScriptWhen::Once);
    }

    // ── create_: decoded as Keep, dest strips create_ prefix ──────────────────

    #[test]
    fn create_dot_zshrc_decodes_to_zshrc() {
        let e = keep("create_dot_zshrc");
        assert!(e.dest_tilde.ends_with("/.zshrc"), "dest={}", e.dest_tilde);
        // source_name preserves the create_ prefix so apply.rs knows to create_only.
        assert_eq!(e.source_name, "create_dot_zshrc");
    }

    #[test]
    fn private_create_dot_ssh_config_decodes_correctly() {
        // private_ comes before create_ — both should be stripped for dest calculation.
        let e = keep("private_create_dot_ssh/config");
        assert!(e.dest_tilde.ends_with("/.ssh/config"), "dest={}", e.dest_tilde);
        assert!(e.private, "expected private=true");
        // source_name preserves the full encoding (create_ and private_ stay in source/).
        assert_eq!(e.source_name, "private_create_dot_ssh/config");
    }

    #[test]
    fn create_dot_config_fish_config_decodes_correctly() {
        let e = keep("create_dot_config/fish/config.fish");
        assert!(e.dest_tilde.ends_with("/.config/fish/config.fish"), "dest={}", e.dest_tilde);
        assert_eq!(e.source_name, "create_dot_config/fish/config.fish");
    }

    // ── is_brewfile_name ───────────────────────────────────────────────────────

    #[test]
    fn brewfile_detection_recognises_plain_brewfile() {
        assert!(is_brewfile_name("Brewfile"));
    }

    #[test]
    fn brewfile_detection_recognises_dotfile_brewfile() {
        assert!(is_brewfile_name(".Brewfile"));
    }

    #[test]
    fn brewfile_detection_recognises_brewfile_with_suffix() {
        assert!(is_brewfile_name("Brewfile.work"));
        assert!(is_brewfile_name("Brewfile.packages"));
        assert!(is_brewfile_name("Brewfile.personal"));
    }

    #[test]
    fn brewfile_detection_rejects_brewfile_lock() {
        assert!(!is_brewfile_name("Brewfile.lock"));
    }

    #[test]
    fn brewfile_detection_rejects_ordinary_files() {
        assert!(!is_brewfile_name("zshrc"));
        assert!(!is_brewfile_name("Gemfile"));
        assert!(!is_brewfile_name("brewfile"));   // case-sensitive
    }

    // ── brewfile_brew_dest / brewfile_module_name ──────────────────────────────

    #[test]
    fn brewfile_brew_dest_plain_gives_packages() {
        assert_eq!(brewfile_brew_dest("Brewfile"), "brew/Brewfile.packages");
        assert_eq!(brewfile_brew_dest(".Brewfile"), "brew/Brewfile.packages");
    }

    #[test]
    fn brewfile_brew_dest_suffix_preserves_suffix() {
        assert_eq!(brewfile_brew_dest("Brewfile.work"), "brew/Brewfile.work");
        assert_eq!(brewfile_brew_dest("Brewfile.personal"), "brew/Brewfile.personal");
    }

    #[test]
    fn brewfile_module_name_plain_gives_packages() {
        assert_eq!(brewfile_module_name("Brewfile"), "packages");
        assert_eq!(brewfile_module_name(".Brewfile"), "packages");
    }

    #[test]
    fn brewfile_module_name_suffix_gives_suffix() {
        assert_eq!(brewfile_module_name("Brewfile.work"), "work");
        assert_eq!(brewfile_module_name("Brewfile.personal"), "personal");
    }
}
