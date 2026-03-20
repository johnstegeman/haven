/// Scan and decode the dfiles `source/` directory.
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

// ─── Public types ─────────────────────────────────────────────────────────────

/// Flags decoded from a magic-name path component.
#[derive(Debug, Clone, Default)]
pub struct FileFlags {
    pub private: bool,
    pub executable: bool,
    pub symlink: bool,
    pub template: bool,
}

/// A decoded source file entry, ready to be applied.
#[derive(Debug, Clone)]
pub struct SourceEntry {
    /// Absolute path to the file under `source/`.
    pub src: PathBuf,
    /// Destination path using `~` (e.g. `"~/.config/git/config"`).
    pub dest_tilde: String,
    /// Flags decoded from this file's own name component.
    pub flags: FileFlags,
    /// Directory components between `source/` and the file, in order.
    /// Apply these first to create / permission parent directories.
    pub dirs: Vec<SourceDir>,
}

/// A decoded directory component that may need to be created / permissioned.
#[derive(Debug, Clone)]
pub struct SourceDir {
    /// Destination path with `~` (e.g. `"~/.ssh"`).
    pub dest_tilde: String,
    /// Flags (mainly `private` → 0700).
    pub flags: FileFlags,
}

// ─── Scanner ──────────────────────────────────────────────────────────────────

/// Recursively scan `source_dir` and return all tracked file entries, sorted
/// by source path for deterministic apply order.
///
/// Hidden entries (names starting with `.`) are skipped — they are git
/// artefacts, not tracked files. Tracked dotfiles use the `dot_` prefix.
pub fn scan(source_dir: &Path) -> Result<Vec<SourceEntry>> {
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
        entries.push(decode_path(dent.path().to_path_buf(), rel));
    }

    entries.sort_by(|a, b| a.src.cmp(&b.src));
    Ok(entries)
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
            flags,
        });
    }

    // Decode the filename (last component).
    let (name, flags) = decode_component(components[n - 1], true);
    dest_parts.push(name);

    SourceEntry {
        src: abs,
        dest_tilde: format!("~/{}", dest_parts.join("/")),
        flags,
        dirs,
    }
}

/// Decode one path component, stripping magic prefixes.
///
/// `is_file`: when true, also strip the `.tmpl` suffix and set `template`.
pub fn decode_component(s: &str, is_file: bool) -> (String, FileFlags) {
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
/// This is used by `dfiles add` to build the encoded filename before copying.
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
    if private    { out.push_str("private_"); }
    if executable { out.push_str("executable_"); }
    if symlink    { out.push_str("symlink_"); }

    if let Some(rest) = dest_name.strip_prefix('.') {
        out.push_str("dot_");
        out.push_str(rest);
    } else {
        out.push_str(dest_name);
    }

    if template { out.push_str(".tmpl"); }
    out
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn decode(rel: &str) -> SourceEntry {
        decode_path(
            PathBuf::from("/repo/source").join(rel),
            Path::new(rel),
        )
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
        assert!(!e.flags.private);
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
        assert!(e.flags.private);
        assert_eq!(e.dirs.len(), 1);
        assert_eq!(e.dirs[0].dest_tilde, "~/.ssh");
        assert!(e.dirs[0].flags.private);
    }

    #[test]
    fn path_executable_file() {
        let e = decode("dot_local/bin/executable_myscript");
        assert_eq!(e.dest_tilde, "~/.local/bin/myscript");
        assert!(e.flags.executable);
        assert!(!e.flags.private);
    }

    #[test]
    fn path_template_file() {
        let e = decode("dot_gitconfig.tmpl");
        assert_eq!(e.dest_tilde, "~/.gitconfig");
        assert!(e.flags.template);
    }

    #[test]
    fn path_symlink_file() {
        let e = decode("symlink_dot_vimrc");
        assert_eq!(e.dest_tilde, "~/.vimrc");
        assert!(e.flags.symlink);
    }

    // ── encode_filename ───────────────────────────────────────────────────────

    #[test]
    fn encode_plain_dotfile() {
        assert_eq!(encode_filename(".zshrc", false, false, false, false), "dot_zshrc");
    }

    #[test]
    fn encode_private_file() {
        assert_eq!(encode_filename("id_rsa", true, false, false, false), "private_id_rsa");
    }

    #[test]
    fn encode_private_dotfile() {
        assert_eq!(encode_filename(".ssh", true, false, false, false), "private_dot_ssh");
    }

    #[test]
    fn encode_template() {
        assert_eq!(encode_filename(".vimrc", false, false, false, true), "dot_vimrc.tmpl");
    }

    #[test]
    fn encode_executable() {
        assert_eq!(encode_filename("myscript", false, true, false, false), "executable_myscript");
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
}
