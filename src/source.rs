/// Scan and decode the haven `source/` directory.
///
/// Files are stored with chezmoi-compatible magic-name encoding. The path
/// under `source/` encodes both the destination and all file metadata — no
/// separate TOML is needed.
///
/// ## Encoding rules
///
/// **Prefixes** (stripped left-to-right; any combination):
///
/// | Prefix        | Meaning                                      |
/// |---------------|----------------------------------------------|
/// | `dot_`        | destination gets a `.` prefix                |
/// | `private_`    | chmod 0600 (files) / 0700 (directories)      |
/// | `executable_` | chmod 0755                                   |
/// | `symlink_`    | create symlink at dest pointing into source/ |
/// | `extdir_`     | clone a remote git repo into this directory  |
///
/// **Suffix** (files only):
///
/// | Suffix  | Meaning                                             |
/// |---------|-----------------------------------------------------|
/// | `.tmpl` | render through Tera; strip suffix from dest name    |
///
/// ## Examples
///
/// ```
/// source/dot_zshrc                       → ~/.zshrc
/// source/dot_gitconfig                   → ~/.gitconfig
/// source/dot_config/git/config           → ~/.config/git/config
/// source/private_dot_ssh/               → ~/.ssh/  (dir, chmod 0700)
///   config                               → ~/.ssh/config
///   private_id_rsa                       → ~/.ssh/id_rsa  (chmod 0600)
/// source/dot_local/bin/executable_foo   → ~/.local/bin/foo  (chmod 0755)
/// source/dot_vimrc.tmpl                 → ~/.vimrc  (Tera template)
/// source/symlink_dot_config/nvim        → ~/.config/nvim  (symlink → source file)
/// ```
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::ignore::IgnoreList;

// ─── Public types ─────────────────────────────────────────────────────────────

/// What kind of entry a source file represents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    /// A regular file: copy or render-then-copy to the destination.
    PlainFile,
    /// A symlink entry (`symlink_` prefix): create a symlink at dest pointing into source/.
    Symlink,
    /// An external directory marker (`extdir_` prefix): clone a remote git repo into dest.
    ExternalDir,
    /// An external file marker (`extfile_` prefix): download a file or archive to dest.
    ExternalFile,
}

/// Flags decoded from a magic-name path component.
#[derive(Debug, Clone, Default)]
struct FileFlags {
    pub private: bool,
    pub executable: bool,
    pub symlink: bool,
    pub template: bool,
    pub extdir: bool,
    pub extfile: bool,
    pub create_only: bool,
    pub exact: bool,
}

/// A decoded source file entry, ready to be applied.
#[derive(Debug, Clone)]
pub struct SourceEntry {
    /// Absolute path to the file under `source/`.
    pub src: PathBuf,
    /// Destination path using `~` (e.g. `"~/.config/git/config"`).
    pub dest_tilde: String,
    /// What kind of entry this is.
    pub kind: EntryKind,
    /// chmod 0600 (file) / 0700 (dir).
    pub private: bool,
    /// chmod 0755.
    pub executable: bool,
    /// Render through Tera before writing.
    pub template: bool,
    /// Skip writing if the destination already exists (chezmoi `create_` prefix).
    pub create_only: bool,
    /// Directory components between `source/` and the file, in order.
    /// Apply these first to create / permission parent directories.
    pub dirs: Vec<SourceDir>,
}

/// A decoded directory component that may need to be created / permissioned.
#[derive(Debug, Clone)]
pub struct SourceDir {
    /// Destination path with `~` (e.g. `"~/.ssh"`).
    pub dest_tilde: String,
    /// chmod 0700 when true.
    pub private: bool,
    /// Remove untracked files in dest on apply (chezmoi `exact_` prefix).
    pub exact: bool,
}

// ─── Scanner ──────────────────────────────────────────────────────────────────

/// Recursively scan `source_dir` and return all tracked file entries, sorted
/// by source path for deterministic apply order.
///
/// Hidden entries (names starting with `.`) are skipped — they are git
/// artefacts, not tracked files. Tracked dotfiles use the `dot_` prefix.
///
/// Entries whose decoded destination path matches a pattern in `ignore` are
/// excluded from the result.
pub fn scan(source_dir: &Path, ignore: &IgnoreList) -> Result<Vec<SourceEntry>> {
    if !source_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();

    for dent in WalkDir::new(source_dir)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            !e.file_name()
                .to_str()
                .map(|s| s.starts_with('.'))
                .unwrap_or(false)
        })
    {
        let dent = dent.with_context(|| format!("Cannot walk {}", source_dir.display()))?;
        if !dent.file_type().is_file() {
            continue;
        }
        let rel = dent
            .path()
            .strip_prefix(source_dir)
            .with_context(|| format!("Cannot strip prefix from {}", dent.path().display()))?;
        let entry = decode_path(dent.path().to_path_buf(), rel);
        if !ignore.is_ignored(&entry.dest_tilde) {
            entries.push(entry);
        }
    }

    entries.sort_by(|a, b| a.src.cmp(&b.src));
    Ok(entries)
}

/// Emit a stderr warning for any destination that multiple source entries decode to.
///
/// Two source files mapping to the same dest (e.g. `variables.sh` and
/// `executable_variables.sh`) is almost always a mistake: the last entry applied
/// silently wins, and `haven status` will permanently show the losing entry as
/// modified.
pub fn warn_duplicate_destinations(entries: &[SourceEntry]) {
    use std::collections::HashMap;

    let mut by_dest: HashMap<&str, Vec<&std::path::Path>> = HashMap::new();
    for entry in entries {
        by_dest
            .entry(entry.dest_tilde.as_str())
            .or_default()
            .push(&entry.src);
    }

    let mut dests: Vec<&str> = by_dest
        .keys()
        .copied()
        .filter(|d| by_dest[d].len() > 1)
        .collect();
    dests.sort();

    for dest in dests {
        let srcs = &by_dest[dest];
        eprintln!("warning: multiple source files map to {dest}:");
        for src in srcs.iter() {
            eprintln!("  {}", src.display());
        }
        eprintln!("  Remove duplicates — the last one applied wins and others will always show as modified.");
    }
}

// ─── Decoder ──────────────────────────────────────────────────────────────────

/// Decode a relative path under `source/` into a `SourceEntry`.
fn decode_path(abs: PathBuf, rel: &Path) -> SourceEntry {
    let components: Vec<&str> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    let n = components.len();
    let mut dest_parts: Vec<String> = Vec::new();
    let mut dirs: Vec<SourceDir> = Vec::new();

    // Decode directory components (all but the last).
    for component in &components[..n - 1] {
        let (name, flags) = decode_component(component, false);
        dest_parts.push(name);
        dirs.push(SourceDir {
            dest_tilde: format!("~/{}", dest_parts.join("/")),
            private: flags.private,
            exact: flags.exact,
        });
    }

    // Decode the filename (last component).
    let (name, flags) = decode_component(components[n - 1], true);
    dest_parts.push(name);

    let kind = if flags.extdir {
        EntryKind::ExternalDir
    } else if flags.extfile {
        EntryKind::ExternalFile
    } else if flags.symlink {
        EntryKind::Symlink
    } else {
        EntryKind::PlainFile
    };

    SourceEntry {
        src: abs,
        dest_tilde: format!("~/{}", dest_parts.join("/")),
        kind,
        private: flags.private,
        executable: flags.executable,
        template: flags.template,
        create_only: flags.create_only,
        dirs,
    }
}

/// Decode one path component, stripping magic prefixes.
///
/// `is_file`: when true, also strip the `.tmpl` suffix and set `template`.
fn decode_component(s: &str, is_file: bool) -> (String, FileFlags) {
    let mut flags = FileFlags::default();
    let mut remaining = s;

    // Strip leading prefixes in any order.
    loop {
        if let Some(rest) = remaining.strip_prefix("private_") {
            flags.private = true;
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix("executable_") {
            flags.executable = true;
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix("symlink_") {
            flags.symlink = true;
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix("extdir_") {
            flags.extdir = true;
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix("extfile_") {
            flags.extfile = true;
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix("create_") {
            flags.create_only = true;
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix("exact_") {
            flags.exact = true;
            remaining = rest;
        } else {
            break;
        }
    }

    // `dot_<name>` → `.<name>`.
    let mut name = if let Some(rest) = remaining.strip_prefix("dot_") {
        format!(".{}", rest)
    } else {
        remaining.to_string()
    };

    // `.tmpl` suffix (files only).
    if is_file && name.ends_with(".tmpl") {
        flags.template = true;
        name.truncate(name.len() - 5);
    }

    (name, flags)
}

// ─── Encoder ──────────────────────────────────────────────────────────────────

/// Encode a destination filename into its magic-name form for storage in `source/`.
///
/// This is used by `haven add` to build the encoded filename before copying.
///
/// A leading `.` in `dest_name` is converted to the `dot_` prefix automatically.
///
/// Examples:
/// ```text
/// encode(".zshrc",  false, false, false, false) → "dot_zshrc"
/// encode("id_rsa",  true,  false, false, false) → "private_id_rsa"
/// encode(".vimrc",  false, false, false, true)  → "dot_vimrc.tmpl"
/// encode(".bashrc", true,  true,  false, false) → "private_executable_dot_bashrc"
/// ```
pub fn encode_filename(
    dest_name: &str,
    private: bool,
    executable: bool,
    symlink: bool,
    template: bool,
) -> String {
    let mut out = String::new();
    if private {
        out.push_str("private_");
    }
    if executable {
        out.push_str("executable_");
    }
    if symlink {
        out.push_str("symlink_");
    }

    if let Some(rest) = dest_name.strip_prefix('.') {
        out.push_str("dot_");
        out.push_str(rest);
    } else {
        out.push_str(dest_name);
    }

    if template {
        out.push_str(".tmpl");
    }
    out
}

// ─── Script entries ───────────────────────────────────────────────────────────

/// When a tracked script should execute on apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptExecWhen {
    /// `run_once_` / `once_` — execute only once per machine (tracked in state.json).
    Once,
    /// `run_` — execute on every `haven apply`.
    Always,
}

/// A script tracked in `source/scripts/`, ready to execute on apply.
#[derive(Debug, Clone)]
pub struct ScriptEntry {
    /// Absolute path to the script file under `source/scripts/`.
    pub src: std::path::PathBuf,
    /// Original filename (e.g. `"run_once_setup.sh"`).
    pub name: String,
    /// When this script should run.
    pub when: ScriptExecWhen,
}

/// Scan `source/scripts/` and return all tracked script entries.
pub fn scan_scripts(scripts_dir: &std::path::Path) -> std::io::Result<Vec<ScriptEntry>> {
    if !scripts_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for dent in std::fs::read_dir(scripts_dir)? {
        let dent = dent?;
        if !dent.file_type()?.is_file() {
            continue;
        }
        let name = dent.file_name().to_string_lossy().to_string();
        // Strip permission prefixes to find the timing prefix.
        let mut stripped = name.as_str();
        while let Some(rest) = stripped
            .strip_prefix("private_")
            .or_else(|| stripped.strip_prefix("executable_"))
        {
            stripped = rest;
        }
        let when = if stripped.starts_with("run_once_") || stripped.starts_with("once_") {
            ScriptExecWhen::Once
        } else {
            ScriptExecWhen::Always
        };
        entries.push(ScriptEntry {
            src: dent.path(),
            name,
            when,
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

// ─── Extdir path helper ───────────────────────────────────────────────────────

/// Build the `source/` path for an `extdir_` marker file from a tilde dest path.
///
/// Examples:
/// ```text
/// "~/.tmux/plugins/tpm"  →  source/dot_tmux/plugins/extdir_tpm
/// "~/.config/nvim"       →  source/dot_config/extdir_nvim
/// "~/nvim"               →  source/extdir_nvim
/// ```
///
/// Directory components are encoded with [`encode_filename`] (`dot_` prefix etc.).
/// The final component gets the `extdir_` prefix instead of any other encoding.
pub fn extdir_source_path(repo_source: &Path, dest_tilde: &str) -> PathBuf {
    let rel = dest_tilde.strip_prefix("~/").unwrap_or(dest_tilde);
    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    let n = parts.len();

    let mut path = repo_source.to_path_buf();
    for component in &parts[..n.saturating_sub(1)] {
        path = path.join(encode_filename(component, false, false, false, false));
    }
    if n > 0 {
        let last = parts[n - 1];
        let encoded_last = format!(
            "extdir_{}",
            encode_filename(last, false, false, false, false)
        );
        path = path.join(encoded_last);
    }
    path
}

/// Build the `source/` path for an `extfile_` marker file from a tilde dest path.
///
/// Examples:
/// ```text
/// "~/.local/bin/gh"   →  source/dot_local/bin/extfile_gh
/// "~/.config/tool"    →  source/dot_config/extfile_tool
/// ```
///
/// Directory components are encoded with [`encode_filename`]. The final
/// component gets the `extfile_` prefix.
#[allow(dead_code)]
pub fn extfile_source_path(repo_source: &Path, dest_tilde: &str) -> PathBuf {
    let rel = dest_tilde.strip_prefix("~/").unwrap_or(dest_tilde);
    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    let n = parts.len();

    let mut path = repo_source.to_path_buf();
    for component in &parts[..n.saturating_sub(1)] {
        path = path.join(encode_filename(component, false, false, false, false));
    }
    if n > 0 {
        let last = parts[n - 1];
        let encoded_last = format!(
            "extfile_{}",
            encode_filename(last, false, false, false, false)
        );
        path = path.join(encoded_last);
    }
    path
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn decode(rel: &str) -> SourceEntry {
        decode_path(PathBuf::from("/repo/source").join(rel), Path::new(rel))
    }

    // ── decode_component ──────────────────────────────────────────────────────

    #[test]
    fn component_plain() {
        let (name, flags) = decode_component("zshrc", false);
        assert_eq!(name, "zshrc");
        assert!(!flags.private);
    }

    #[test]
    fn component_dot_prefix() {
        let (name, _) = decode_component("dot_zshrc", false);
        assert_eq!(name, ".zshrc");
    }

    #[test]
    fn component_private() {
        let (name, flags) = decode_component("private_id_rsa", false);
        assert_eq!(name, "id_rsa");
        assert!(flags.private);
    }

    #[test]
    fn component_private_dot() {
        let (name, flags) = decode_component("private_dot_ssh", false);
        assert_eq!(name, ".ssh");
        assert!(flags.private);
    }

    #[test]
    fn component_executable_dot() {
        let (name, flags) = decode_component("executable_dot_local", false);
        assert_eq!(name, ".local");
        assert!(flags.executable);
    }

    #[test]
    fn component_tmpl_suffix() {
        let (name, flags) = decode_component("dot_vimrc.tmpl", true);
        assert_eq!(name, ".vimrc");
        assert!(flags.template);
    }

    #[test]
    fn component_tmpl_suffix_ignored_for_dirs() {
        let (name, flags) = decode_component("dot_vimrc.tmpl", false);
        assert_eq!(name, ".vimrc.tmpl"); // not stripped for directories
        assert!(!flags.template);
    }

    // ── decode_path ───────────────────────────────────────────────────────────

    #[test]
    fn path_flat_dotfile() {
        let e = decode("dot_zshrc");
        assert_eq!(e.dest_tilde, "~/.zshrc");
        assert!(e.dirs.is_empty());
        assert!(!e.private);
        assert_eq!(e.kind, EntryKind::PlainFile);
    }

    #[test]
    fn path_nested_plain() {
        let e = decode("dot_config/git/config");
        assert_eq!(e.dest_tilde, "~/.config/git/config");
        assert_eq!(e.dirs.len(), 2);
        assert_eq!(e.dirs[0].dest_tilde, "~/.config");
        assert_eq!(e.dirs[1].dest_tilde, "~/.config/git");
    }

    #[test]
    fn path_private_dir_private_file() {
        let e = decode("private_dot_ssh/private_id_rsa");
        assert_eq!(e.dest_tilde, "~/.ssh/id_rsa");
        assert!(e.private);
        assert_eq!(e.dirs.len(), 1);
        assert_eq!(e.dirs[0].dest_tilde, "~/.ssh");
        assert!(e.dirs[0].private);
    }

    #[test]
    fn path_executable_file() {
        let e = decode("dot_local/bin/executable_myscript");
        assert_eq!(e.dest_tilde, "~/.local/bin/myscript");
        assert!(e.executable);
        assert!(!e.private);
    }

    #[test]
    fn path_template_file() {
        let e = decode("dot_gitconfig.tmpl");
        assert_eq!(e.dest_tilde, "~/.gitconfig");
        assert!(e.template);
    }

    #[test]
    fn path_symlink_file() {
        let e = decode("symlink_dot_vimrc");
        assert_eq!(e.dest_tilde, "~/.vimrc");
        assert_eq!(e.kind, EntryKind::Symlink);
    }

    #[test]
    fn path_extdir_plain() {
        let e = decode("dot_tmux/plugins/extdir_tpm");
        assert_eq!(e.dest_tilde, "~/.tmux/plugins/tpm");
        assert_eq!(e.kind, EntryKind::ExternalDir);
        assert_eq!(e.dirs.len(), 2);
        assert_eq!(e.dirs[0].dest_tilde, "~/.tmux");
        assert_eq!(e.dirs[1].dest_tilde, "~/.tmux/plugins");
    }

    #[test]
    fn path_extdir_with_dot_inside() {
        let e = decode("dot_tmux/extdir_dot_plugins");
        assert_eq!(e.dest_tilde, "~/.tmux/.plugins");
        assert_eq!(e.kind, EntryKind::ExternalDir);
    }

    #[test]
    fn path_extdir_at_root() {
        let e = decode("extdir_nvim");
        assert_eq!(e.dest_tilde, "~/nvim");
        assert_eq!(e.kind, EntryKind::ExternalDir);
        assert!(e.dirs.is_empty());
    }

    // ── encode_filename ───────────────────────────────────────────────────────

    #[test]
    fn encode_plain_dotfile() {
        assert_eq!(
            encode_filename(".zshrc", false, false, false, false),
            "dot_zshrc"
        );
    }

    #[test]
    fn encode_private_file() {
        assert_eq!(
            encode_filename("id_rsa", true, false, false, false),
            "private_id_rsa"
        );
    }

    #[test]
    fn encode_private_dotfile() {
        assert_eq!(
            encode_filename(".ssh", true, false, false, false),
            "private_dot_ssh"
        );
    }

    #[test]
    fn encode_template() {
        assert_eq!(
            encode_filename(".vimrc", false, false, false, true),
            "dot_vimrc.tmpl"
        );
    }

    #[test]
    fn encode_executable() {
        assert_eq!(
            encode_filename("myscript", false, true, false, false),
            "executable_myscript"
        );
    }

    #[test]
    fn encode_private_executable_dotfile() {
        assert_eq!(
            encode_filename(".bashrc", true, true, false, false),
            "private_executable_dot_bashrc"
        );
    }

    // ── round-trip: encode → decode ───────────────────────────────────────────

    #[test]
    fn roundtrip_private_dot() {
        let encoded = encode_filename(".ssh", true, false, false, false);
        let (decoded, flags) = decode_component(&encoded, false);
        assert_eq!(decoded, ".ssh");
        assert!(flags.private);
    }

    #[test]
    fn roundtrip_template() {
        let encoded = encode_filename(".gitconfig", false, false, false, true);
        let (decoded, flags) = decode_component(&encoded, true);
        assert_eq!(decoded, ".gitconfig");
        assert!(flags.template);
    }

    #[test]
    fn component_create_only() {
        let (name, flags) = decode_component("create_dot_zshrc", true);
        assert_eq!(name, ".zshrc");
        assert!(flags.create_only, "expected create_only=true");
        assert!(!flags.private);
        assert!(!flags.executable);
    }

    #[test]
    fn path_create_only_file() {
        let e = decode("create_dot_zshrc");
        assert_eq!(e.dest_tilde, "~/.zshrc");
        assert!(e.create_only, "expected create_only=true");
    }

    #[test]
    fn component_exact_dir() {
        let (name, flags) = decode_component("exact_dot_config", false);
        assert_eq!(name, ".config");
        assert!(flags.exact, "expected exact=true");
        assert!(!flags.create_only);
    }

    #[test]
    fn path_exact_dir_sets_flag_on_sourcedir() {
        let e = decode("exact_dot_config/fish/config.fish");
        assert_eq!(e.dest_tilde, "~/.config/fish/config.fish");
        assert!(e.dirs[0].exact, "expected exact dir flag on ~/.config");
        // exact_ on a dir must not contaminate the file entry's kind — SourceEntry has no
        // `exact` field, so the file cannot accidentally become an ExternalDir or Symlink.
        assert_eq!(e.kind, EntryKind::PlainFile, "exact_ on dir must not affect file kind");
    }

    #[test]
    fn path_create_only_nested_file() {
        // create_ on a directory component is recorded on the SourceDir, not the file.
        // The file itself has no create_only — the directory level carries the flag.
        let e = decode("create_dot_config/fish/config.fish");
        assert_eq!(e.dest_tilde, "~/.config/fish/config.fish");
        // Dir at index 0 is create_dot_config → decoded to .config with create_only.
        // SourceDir only has `private` and `exact`; create_only on a dir is not tracked.
        assert!(!e.dirs[0].private, "dir should not be private");
    }

    // ── extfile_ ──────────────────────────────────────────────────────────────

    #[test]
    fn component_extfile_sets_flag() {
        let (name, flags) = decode_component("extfile_gh", false);
        assert_eq!(name, "gh");
        assert!(flags.extfile);
        assert!(!flags.extdir);
    }

    #[test]
    fn component_extfile_dot_prefix() {
        let (name, flags) = decode_component("extfile_dot_local", false);
        assert_eq!(name, ".local");
        assert!(flags.extfile);
    }

    #[test]
    fn path_extfile_simple() {
        let e = decode("dot_local/bin/extfile_gh");
        assert_eq!(e.dest_tilde, "~/.local/bin/gh");
        assert_eq!(e.kind, EntryKind::ExternalFile);
        // extfile semantics apply only to the file entry; SourceDir has no extfile concept,
        // so the invariant is type-enforced. Verify the two expected parent dirs are present.
        assert_eq!(e.dirs.len(), 2); // dot_local and bin — neither has extfile semantics
    }

    #[test]
    fn extfile_source_path_simple() {
        let source = PathBuf::from("/repo/source");
        let p = extfile_source_path(&source, "~/.local/bin/gh");
        assert_eq!(p, PathBuf::from("/repo/source/dot_local/bin/extfile_gh"));
    }

    #[test]
    fn extfile_source_path_single_component() {
        let source = PathBuf::from("/repo/source");
        let p = extfile_source_path(&source, "~/mytool");
        assert_eq!(p, PathBuf::from("/repo/source/extfile_mytool"));
    }

    #[test]
    fn extfile_source_path_dotfile() {
        let source = PathBuf::from("/repo/source");
        let p = extfile_source_path(&source, "~/.config/tool");
        assert_eq!(p, PathBuf::from("/repo/source/dot_config/extfile_tool"));
    }
}
