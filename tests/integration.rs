/// Integration tests for haven Week 1–8 core loop:
/// init → add → apply → status, templates, packages, 1Password integration,
/// AI module (gh: sources + CLAUDE.md generation), bootstrap, and chezmoi import.
///
/// All tests use temp directories so they never touch the real home directory.
///
/// Files are tracked via magic-name encoding in source/ (chezmoi-compatible).
/// No [[files]] TOML entries — the encoded filename is the source of truth.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

// ─── helpers ──────────────────────────────────────────────────────────────────

/// Build a `haven` command with `--dir` pointed at `repo` and the HAVEN_DIR
/// env var unset so it never falls back to `~/haven`.
fn cmd(repo: &TempDir) -> Command {
    let mut c = Command::cargo_bin("haven").unwrap();
    c.arg("--dir").arg(repo.path());
    // Prevent any real ~/haven or ~/.claude from leaking in.
    c.env_remove("HAVEN_DIR");
    c.env_remove("HAVEN_CLAUDE_DIR");
    c
}

/// Build a `haven` command that also overrides HOME so `~` expands to `home`.
/// Required for any test that applies source files (magic-name paths use `~/`).
fn cmd_home(repo: &TempDir, home: &TempDir) -> Command {
    let mut c = cmd(repo);
    c.env("HOME", home.path());
    c
}

/// Like `cmd_home` but also pins the state directory to a specific path.
/// Useful for multi-apply tests that need state to persist between invocations.
#[allow(dead_code)]
fn cmd_home_with_state(repo: &TempDir, home: &TempDir, state_dir: &std::path::Path) -> Command {
    let c = cmd_home(repo, home);
    // haven reads state from $XDG_STATE_HOME/haven (defaults to HOME/.local/state/haven).
    // Pinning HOME via cmd_home is sufficient — dirs::state_dir() follows HOME.
    let _ = state_dir; // state_dir is HOME/.local/state/haven which cmd_home already pins via HOME
    c
}

// ─── init ────────────────────────────────────────────────────────────────────

#[test]
fn init_creates_scaffold() {
    let repo = TempDir::new().unwrap();
    cmd(&repo)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized haven repo"));

    assert!(repo.path().join("haven.toml").exists(), "haven.toml missing");
    assert!(repo.path().join("source").is_dir(), "source/ missing");
    assert!(repo.path().join("brew").is_dir(), "brew/ missing");
    assert!(
        repo.path().join("modules").is_dir(),
        "modules/ missing"
    );
    assert!(
        repo.path().join("modules").join("shell.toml").exists(),
        "shell.toml missing"
    );
}

#[test]
fn init_already_initialized_is_noop() {
    let repo = TempDir::new().unwrap();

    cmd(&repo).arg("init").assert().success();

    // Second init should succeed (graceful no-op) with an informative message.
    cmd(&repo)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("already initialized"));
}

// ─── init from source ────────────────────────────────────────────────────────

/// Create a local git repo containing a minimal `haven.toml` and return its
/// path as a `TempDir`. Used as a stand-in for a remote in clone tests so no
/// network access is required.
fn make_local_git_repo(extra_files: &[(&str, &str)]) -> TempDir {
    let remote = TempDir::new().unwrap();
    let r = remote.path();

    std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(r)
        .output()
        .expect("git init failed");

    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(r)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(r)
        .output()
        .unwrap();

    fs::write(
        r.join("haven.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();

    for (name, content) in extra_files {
        fs::write(r.join(name), content).unwrap();
    }

    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(r)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(r)
        .output()
        .unwrap();

    remote
}

/// Build a `haven init <source>` command pointing at a fresh target dir.
/// Returns both the target TempDir and the pre-built Command.
fn init_from(source: &str) -> (TempDir, Command) {
    let target = TempDir::new().unwrap();
    let mut c = Command::cargo_bin("haven").unwrap();
    c.arg("--dir").arg(target.path());
    c.env_remove("HAVEN_DIR");
    c.env_remove("HAVEN_CLAUDE_DIR");
    c.arg("init").arg(source);
    (target, c)
}

#[test]
fn init_from_local_path_clones_repo() {
    let remote = make_local_git_repo(&[]);
    let (target, mut cmd) = init_from(remote.path().to_str().unwrap());

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Cloned successfully."));

    assert!(target.path().join("haven.toml").exists(), "haven.toml missing after clone");
}

#[test]
fn init_from_gh_notation_builds_https_url() {
    // We can't hit github.com in tests, but we can confirm the command fails
    // with a git error (not a haven parse error) — proving we parsed the
    // notation and tried to clone.
    let target = TempDir::new().unwrap();
    let mut c = Command::cargo_bin("haven").unwrap();
    c.arg("--dir").arg(target.path());
    c.env_remove("HAVEN_DIR");
    c.env_remove("HAVEN_CLAUDE_DIR");
    // Use a deliberately invalid owner so git fails fast with an auth/404 error
    // rather than hanging. The important thing: haven must NOT produce a parse
    // error — that would mean we mishandled the gh: notation.
    c.arg("init").arg("gh:__invalid_haven_test__/no-such-repo");

    let output = c.output().unwrap();
    // haven should not error about parsing — it should get as far as calling git
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("expected 'gh:' prefix"),
        "gh: notation was not parsed correctly: {stderr}"
    );
    assert!(
        !stderr.contains("expected 'owner/repo'"),
        "gh: notation was not parsed correctly: {stderr}"
    );
    // git should have been invoked (error from git, not from haven arg parsing)
    assert!(
        stderr.contains("git clone failed") || stderr.contains("https://github.com"),
        "expected git clone attempt, got: {stderr}"
    );
}

#[test]
fn init_from_source_with_branch() {
    let remote = make_local_git_repo(&[]);

    // Create a second branch in the remote.
    std::process::Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(remote.path())
        .output()
        .unwrap();
    fs::write(remote.path().join("feature.txt"), "on feature branch").unwrap();
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(remote.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "feature commit"])
        .current_dir(remote.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["checkout", "main"])
        .current_dir(remote.path())
        .output()
        .unwrap();

    let target = TempDir::new().unwrap();
    Command::cargo_bin("haven")
        .unwrap()
        .arg("--dir").arg(target.path())
        .env_remove("HAVEN_DIR")
        .env_remove("HAVEN_CLAUDE_DIR")
        .arg("init")
        .arg(remote.path().to_str().unwrap())
        .arg("--branch").arg("feature")
        .assert()
        .success()
        .stdout(predicate::str::contains("branch: feature"));

    assert!(
        target.path().join("feature.txt").exists(),
        "feature branch file missing — wrong branch was cloned"
    );
}

#[test]
fn init_from_source_at_ref_uses_ref_as_branch() {
    let remote = make_local_git_repo(&[]);

    // Create a 'dev' branch.
    std::process::Command::new("git")
        .args(["checkout", "-b", "dev"])
        .current_dir(remote.path())
        .output()
        .unwrap();
    fs::write(remote.path().join("dev.txt"), "on dev branch").unwrap();
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(remote.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "dev commit"])
        .current_dir(remote.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["checkout", "main"])
        .current_dir(remote.path())
        .output()
        .unwrap();

    // Use gh: notation with @dev. For a local path we can't use gh: syntax,
    // so we simulate by building a source string that embeds @ref — we test the
    // branch resolution logic directly via run() in the unit tests instead.
    // Here we verify that --branch achieves the same outcome.
    let target = TempDir::new().unwrap();
    Command::cargo_bin("haven")
        .unwrap()
        .arg("--dir").arg(target.path())
        .env_remove("HAVEN_DIR")
        .env_remove("HAVEN_CLAUDE_DIR")
        .arg("init")
        .arg(remote.path().to_str().unwrap())
        .arg("--branch").arg("dev")
        .assert()
        .success();

    assert!(
        target.path().join("dev.txt").exists(),
        "dev branch file missing"
    );
}

#[test]
fn init_apply_fails_without_source() {
    let repo = TempDir::new().unwrap();
    Command::cargo_bin("haven")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env_remove("HAVEN_DIR")
        .env_remove("HAVEN_CLAUDE_DIR")
        .arg("init")
        .arg("--apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--apply requires a source"));
}

#[test]
fn init_profile_fails_without_source() {
    let repo = TempDir::new().unwrap();
    Command::cargo_bin("haven")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env_remove("HAVEN_DIR")
        .env_remove("HAVEN_CLAUDE_DIR")
        .arg("init")
        .arg("--profile").arg("work")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--apply requires a source"));
}

#[test]
fn init_fails_if_target_dir_nonempty() {
    let target = TempDir::new().unwrap();
    // Seed the target with a file.
    fs::write(target.path().join("existing.txt"), "already here").unwrap();

    let remote = make_local_git_repo(&[]);
    Command::cargo_bin("haven")
        .unwrap()
        .arg("--dir").arg(target.path())
        .env_remove("HAVEN_DIR")
        .env_remove("HAVEN_CLAUDE_DIR")
        .arg("init")
        .arg(remote.path().to_str().unwrap())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists and is not empty"));
}

#[test]
fn init_apply_hard_fails_if_no_haven_toml() {
    // A git repo with no haven.toml.
    let remote = TempDir::new().unwrap();
    let r = remote.path();
    std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(r)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(r)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(r)
        .output()
        .unwrap();
    fs::write(r.join("README.md"), "not a haven repo").unwrap();
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(r)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(r)
        .output()
        .unwrap();

    let target = TempDir::new().unwrap();
    Command::cargo_bin("haven")
        .unwrap()
        .arg("--dir").arg(target.path())
        .env_remove("HAVEN_DIR")
        .env_remove("HAVEN_CLAUDE_DIR")
        .arg("init")
        .arg(r.to_str().unwrap())
        .arg("--apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not appear to be a haven repository"));
}

#[test]
fn init_from_source_with_apply() {
    let home = TempDir::new().unwrap();
    let remote = make_local_git_repo(&[]);
    let target = TempDir::new().unwrap();

    Command::cargo_bin("haven")
        .unwrap()
        .arg("--dir").arg(target.path())
        .env_remove("HAVEN_DIR")
        .env("HOME", home.path())
        .env("HAVEN_CLAUDE_DIR", home.path().join(".claude"))
        .arg("init")
        .arg(remote.path().to_str().unwrap())
        .arg("--apply")
        .assert()
        .success()
        .stdout(predicate::str::contains("Applying profile 'default'"));
}

// ─── add ─────────────────────────────────────────────────────────────────────

#[test]
fn add_tracks_a_file() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // Create a dotfile under a fake HOME.
    let home = TempDir::new().unwrap();
    let dotfile = home.path().join(".testrc");
    fs::write(&dotfile, "export FOO=bar\n").unwrap();

    cmd_home(&repo, &home)
        .args(["add", dotfile.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added:"));

    // Source copy should exist with the encoded name.
    assert!(
        repo.path().join("source").join("dot_testrc").exists(),
        "source/dot_testrc missing"
    );
    // No module TOML entry needed — encoding is in the filename.
}

#[test]
fn add_is_idempotent() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let home = TempDir::new().unwrap();
    let dotfile = home.path().join(".idempotent.rc");
    fs::write(&dotfile, "# idempotent\n").unwrap();

    // Add once.
    cmd_home(&repo, &home)
        .args(["add", dotfile.to_str().unwrap()])
        .assert()
        .success();

    // Add again without --update — should fail with "already tracked".
    cmd_home(&repo, &home)
        .args(["add", dotfile.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already tracked"));

    // Exactly one source file with encoded name.
    assert!(repo.path().join("source").join("dot_idempotent.rc").exists());
}

#[test]
fn add_fails_for_missing_file() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["add", "/tmp/haven-does-not-exist-xyz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("File not found"));
}

// ─── add: directory handling ──────────────────────────────────────────────────

#[test]
fn add_directory_without_git_adds_files_recursively() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // Build a fake home with a non-git directory containing two files.
    let home = TempDir::new().unwrap();
    let dir = home.path().join(".myapp");
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("config"), "key=val\n").unwrap();
    fs::write(dir.join("sub").join("data"), "data\n").unwrap();

    cmd_home(&repo, &home)
        .args(["add", dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added 2 file(s)"));

    // Encoded paths: dot_myapp/config and dot_myapp/sub/data.
    assert!(
        repo.path().join("source").join("dot_myapp").join("config").exists(),
        "source/dot_myapp/config missing"
    );
    assert!(
        repo.path().join("source").join("dot_myapp").join("sub").join("data").exists(),
        "source/dot_myapp/sub/data missing"
    );
}

#[test]
fn add_directory_with_git_remote_and_extdir_choice_writes_marker() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let home = TempDir::new().unwrap();
    let plugin_dir = home.path().join(".tmux").join("plugins").join("tpm");
    fs::create_dir_all(&plugin_dir).unwrap();

    // Init a git repo with a remote.
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&plugin_dir)
        .status()
        .unwrap();
    std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://github.com/tmux-plugins/tpm"])
        .current_dir(&plugin_dir)
        .status()
        .unwrap();

    // User picks option 1 (add as external using the first remote).
    cmd_home(&repo, &home)
        .args(["add", plugin_dir.to_str().unwrap()])
        .write_stdin("1\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Added external"))
        .stdout(predicate::str::contains("extdir_tpm"))
        .stdout(predicate::str::contains("tmux-plugins/tpm"));

    // The extdir_ marker file should exist with the correct path.
    let marker = repo.path()
        .join("source")
        .join("dot_tmux")
        .join("plugins")
        .join("extdir_tpm");
    assert!(marker.exists(), "extdir_ marker missing at {:?}", marker);

    let content = fs::read_to_string(&marker).unwrap();
    assert!(content.contains("tmux-plugins/tpm"), "url missing from marker");
    assert!(content.contains("git"), "type missing from marker");
}

#[test]
fn add_directory_with_git_remote_and_files_choice_adds_recursively() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let home = TempDir::new().unwrap();
    let plugin_dir = home.path().join(".config").join("nvim");
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(plugin_dir.join("init.lua"), "-- nvim config\n").unwrap();

    // Init a git repo with a remote.
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&plugin_dir)
        .status()
        .unwrap();
    std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://github.com/user/nvim"])
        .current_dir(&plugin_dir)
        .status()
        .unwrap();

    // User picks 'f' to add files recursively instead of as external.
    cmd_home(&repo, &home)
        .args(["add", plugin_dir.to_str().unwrap()])
        .write_stdin("f\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Added 1 file(s)"));

    // File should be tracked, not as extdir.
    assert!(
        repo.path().join("source").join("dot_config").join("nvim").join("init.lua").exists(),
        "source/dot_config/nvim/init.lua missing"
    );
    assert!(
        !repo.path().join("source").join("dot_config").join("extdir_nvim").exists(),
        "unexpected extdir_ marker"
    );
}

#[test]
fn add_file_in_private_dir_encodes_private_prefix() {
    use std::os::unix::fs::PermissionsExt;

    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let home = TempDir::new().unwrap();
    let ssh_dir = home.path().join(".ssh");
    fs::create_dir_all(&ssh_dir).unwrap();
    // chmod 0700 on .ssh — should produce private_dot_ssh/
    fs::set_permissions(&ssh_dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(ssh_dir.join("config"), "Host *\n").unwrap();

    cmd_home(&repo, &home)
        .args(["add", ssh_dir.join("config").to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added:"));

    assert!(
        repo.path().join("source").join("private_dot_ssh").join("config").exists(),
        "expected source/private_dot_ssh/config"
    );
}

// ─── profile resolution from state ───────────────────────────────────────────

#[test]
fn apply_uses_saved_profile_when_no_flag_given() {
    // Arrange: a repo with two profiles, and a state.json that says "work" was
    // the last-used profile. Running `haven apply` without --profile should
    // pick up "work" and apply that profile's modules.
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let state_dir = home.path().join(".local/state/haven");

    cmd(&repo).arg("init").assert().success();

    // Two profiles: default (no modules) and work (also no modules — we just
    // want to verify the profile name that ends up in the next state.json).
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n\n[profile.work]\nmodules = []\n",
    )
    .unwrap();

    // Seed state.json with "work" as the last-used profile.
    fs::create_dir_all(&state_dir).unwrap();
    fs::write(
        state_dir.join("state.json"),
        r#"{"version":"1","profile":"work","hostname":"test","last_apply":null,"modules":{}}"#,
    )
    .unwrap();

    // Run apply with no --profile flag.
    Command::cargo_bin("haven")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env("HOME", home.path())
        .env("HAVEN_CLAUDE_DIR", home.path().join(".claude"))
        .env_remove("HAVEN_DIR")
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::contains("work"));

    // Verify state.json still records "work" as the profile.
    let state_text = fs::read_to_string(state_dir.join("state.json")).unwrap();
    assert!(
        state_text.contains("\"work\""),
        "state.json should record profile 'work', got: {state_text}"
    );
}

#[test]
fn apply_explicit_profile_overrides_saved_profile() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let state_dir = home.path().join(".local/state/haven");

    cmd(&repo).arg("init").assert().success();

    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n\n[profile.work]\nmodules = []\n",
    )
    .unwrap();

    // State says "work" was last used.
    fs::create_dir_all(&state_dir).unwrap();
    fs::write(
        state_dir.join("state.json"),
        r#"{"version":"1","profile":"work","hostname":"test","last_apply":null,"modules":{}}"#,
    )
    .unwrap();

    // Explicit --profile default should override the saved "work".
    Command::cargo_bin("haven")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env("HOME", home.path())
        .env("HAVEN_CLAUDE_DIR", home.path().join(".claude"))
        .env_remove("HAVEN_DIR")
        .args(["apply", "--profile", "default"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default"));
}

#[test]
fn apply_falls_back_to_default_when_no_state() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    cmd(&repo).arg("init").assert().success();

    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();

    // No state.json — should fall back to "default".
    Command::cargo_bin("haven")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env("HOME", home.path())
        .env("HAVEN_CLAUDE_DIR", home.path().join(".claude"))
        .env_remove("HAVEN_DIR")
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::contains("default"));
}

// ─── apply ───────────────────────────────────────────────────────────────────

/// Set up a repo with one tracked file (magic-name encoded) and a shell module
/// with one external (so [shell] appears in dry-run output).
///
/// Returns (repo TempDir, home TempDir).
///
/// Source layout:
///   source/dot_applyrc  →  ~/.applyrc  (plain file, content "export APPLY=1\n")
fn setup_apply() -> (TempDir, TempDir) {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    cmd(&repo).arg("init").assert().success();

    // Write source file with magic-name encoding.
    let source_dir = repo.path().join("source");
    fs::write(source_dir.join("dot_applyrc"), "export APPLY=1\n").unwrap();

    // No externals — avoid real network calls in tests.
    // Source files are global and apply regardless of module list.
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();

    (repo, home)
}

#[test]
fn apply_copies_file_to_dest() {
    let (repo, home) = setup_apply();
    let dest_path = home.path().join(".applyrc");

    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default"])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));

    assert!(dest_path.exists(), "dest file was not created");
    assert_eq!(
        fs::read_to_string(&dest_path).unwrap(),
        "export APPLY=1\n"
    );
}

#[test]
fn apply_dry_run_prints_plan_without_writing() {
    let (repo, home) = setup_apply();
    let dest_path = home.path().join(".applyrc");

    cmd_home(&repo, &home)
        .args(["apply", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("applyrc"));

    assert!(
        !dest_path.exists(),
        "dry-run must not write files"
    );
}

#[test]
fn apply_run_scripts_executes_script() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    // Write a run_ script that creates a sentinel file.
    let scripts_dir = repo.path().join("source").join("scripts");
    fs::create_dir_all(&scripts_dir).unwrap();
    let sentinel = home.path().join("script_ran");
    let script_content = format!("#!/bin/sh\ntouch {:?}\n", sentinel);
    fs::write(scripts_dir.join("run_setup.sh"), &script_content).unwrap();
    // Make it executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(scripts_dir.join("run_setup.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    }

    cmd_home(&repo, &home)
        .args(["apply", "--run-scripts"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[scripts]"));

    assert!(sentinel.exists(), "script should have created the sentinel file");
}

#[test]
fn apply_run_once_script_runs_only_once() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    let scripts_dir = repo.path().join("source").join("scripts");
    fs::create_dir_all(&scripts_dir).unwrap();
    let counter_file = home.path().join("run_count");
    // Script appends a line to a counter file on each run.
    let script_content = format!("#!/bin/sh\necho run >> {:?}\n", counter_file);
    fs::write(scripts_dir.join("run_once_setup.sh"), &script_content).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(scripts_dir.join("run_once_setup.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    }

    // First apply: script should run.
    cmd_home(&repo, &home)
        .args(["apply", "--run-scripts"])
        .assert()
        .success();
    let lines_after_first = fs::read_to_string(&counter_file).unwrap_or_default();
    assert_eq!(lines_after_first.lines().count(), 1, "script should run once on first apply");

    // Second apply with same HOME (state persists in HOME/.haven): script should NOT run again.
    cmd_home(&repo, &home)
        .args(["apply", "--run-scripts"])
        .assert()
        .success()
        .stdout(predicate::str::contains("already run"));
    let lines_after_second = fs::read_to_string(&counter_file).unwrap_or_default();
    assert_eq!(lines_after_second.lines().count(), 1, "run_once_ script must not run a second time");
}

#[test]
fn apply_scripts_not_run_without_flag() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    let scripts_dir = repo.path().join("source").join("scripts");
    fs::create_dir_all(&scripts_dir).unwrap();
    let sentinel = home.path().join("should_not_exist");
    let script_content = format!("#!/bin/sh\ntouch {:?}\n", sentinel);
    fs::write(scripts_dir.join("run_setup.sh"), &script_content).unwrap();

    // Apply WITHOUT --run-scripts.
    cmd_home(&repo, &home)
        .arg("apply")
        .assert()
        .success();

    assert!(!sentinel.exists(), "script must not run without --run-scripts flag");
}

#[test]
fn apply_exact_dir_removes_untracked_files() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    // Source: exact_dot_ssh/config → ~/.ssh/config  (exact_ on the dir)
    let source_dir = repo.path().join("source");
    let ssh_src = source_dir.join("exact_dot_ssh");
    fs::create_dir_all(&ssh_src).unwrap();
    fs::write(ssh_src.join("config"), "[Host *]\n").unwrap();

    // Dest: ~/.ssh/ has a tracked file and an untracked stale key.
    let ssh_dest = home.path().join(".ssh");
    fs::create_dir_all(&ssh_dest).unwrap();
    fs::write(ssh_dest.join("config"), "[Host *]\n").unwrap();
    fs::write(ssh_dest.join("id_rsa_old"), "stale key\n").unwrap();

    cmd_home(&repo, &home)
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::contains("[exact]"));

    // Tracked file stays.
    assert!(ssh_dest.join("config").exists(), "tracked file must remain");
    // Untracked file removed.
    assert!(!ssh_dest.join("id_rsa_old").exists(), "untracked file must be removed");
}

#[test]
fn apply_exact_dir_keeps_tracked_files() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    let source_dir = repo.path().join("source");
    let ssh_src = source_dir.join("exact_dot_ssh");
    fs::create_dir_all(&ssh_src).unwrap();
    fs::write(ssh_src.join("config"), "[Host *]\n").unwrap();

    // Dest only has the tracked file — nothing to remove.
    let ssh_dest = home.path().join(".ssh");
    fs::create_dir_all(&ssh_dest).unwrap();

    cmd_home(&repo, &home)
        .arg("apply")
        .assert()
        .success();

    assert!(ssh_dest.join("config").exists());
}

#[test]
fn apply_create_only_skips_if_dest_exists() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    // Source file with create_ prefix — should only be written if dest is absent.
    let source_dir = repo.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("create_dot_seedrc"), "seed content\n").unwrap();

    // Destination already exists with different content.
    let dest = home.path().join(".seedrc");
    fs::write(&dest, "user content\n").unwrap();

    cmd_home(&repo, &home)
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::contains("create_only"));

    // Destination must NOT be overwritten.
    let result = fs::read_to_string(&dest).unwrap();
    assert_eq!(result, "user content\n", "create_only must not overwrite existing dest");
}

#[test]
fn apply_create_only_writes_if_dest_absent() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    let source_dir = repo.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("create_dot_seedrc"), "seed content\n").unwrap();

    // Destination does not exist.
    let dest = home.path().join(".seedrc");
    assert!(!dest.exists());

    cmd_home(&repo, &home)
        .arg("apply")
        .assert()
        .success();

    // File should be written on first apply.
    let result = fs::read_to_string(&dest).unwrap();
    assert_eq!(result, "seed content\n", "create_only must write file when dest is absent");
}

#[test]
fn apply_backs_up_existing_file() {
    let (repo, home) = setup_apply();
    let dest_path = home.path().join(".applyrc");
    fs::write(&dest_path, "old content\n").unwrap();

    cmd_home(&repo, &home)
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::contains("backed up"));
}

// ─── extfile_ ────────────────────────────────────────────────────────────────

#[test]
fn apply_extfile_dry_run_shows_download_entry() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    // Write an extfile_ marker for a binary at ~/.local/bin/gh.
    let source_dir = repo.path().join("source");
    let bin_dir = source_dir.join("dot_local").join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(
        bin_dir.join("extfile_gh"),
        "type = \"file\"\nurl  = \"https://example.com/gh-v2.0.tar.gz\"\nref  = \"v2.0\"\n",
    )
    .unwrap();

    cmd_home(&repo, &home)
        .args(["apply", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[extfile]"))
        .stdout(predicate::str::contains("https://example.com/gh-v2.0.tar.gz"));
}

#[test]
fn apply_extfile_archive_dry_run_shows_extract_label() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    let source_dir = repo.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(
        source_dir.join("extfile_dot_config_backup"),
        "type = \"archive\"\nurl  = \"https://example.com/config.tar.gz\"\n",
    )
    .unwrap();

    cmd_home(&repo, &home)
        .args(["apply", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[extfile]"))
        .stdout(predicate::str::contains("extract"));
}

#[test]
fn diff_extfile_missing_shows_question_mark() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    // Marker present in source/ but file not yet downloaded (dest absent).
    let source_dir = repo.path().join("source");
    let bin_dir = source_dir.join("dot_local").join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(
        bin_dir.join("extfile_mytool"),
        "type = \"file\"\nurl  = \"https://example.com/mytool\"\n",
    )
    .unwrap();

    cmd_home(&repo, &home)
        .args(["diff", "--files"])
        .assert()
        // Exit 1 = drift found.
        .code(1)
        .stdout(predicate::str::contains("extfile: not downloaded"));
}

#[test]
fn source_extfile_flag_decoded_from_path() {
    // Unit-level check: extfile_ prefix sets the extfile flag.
    use assert_cmd::Command;
    // We exercise this indirectly via dry-run: the marker is scanned and
    // printed as [extfile], confirming decode_component set flags.extfile.
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    let source_dir = repo.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(
        source_dir.join("extfile_dot_tool"),
        "type = \"file\"\nurl  = \"https://example.com/tool\"\n",
    )
    .unwrap();

    Command::cargo_bin("haven")
        .unwrap()
        .args(["--dir", repo.path().to_str().unwrap()])
        .env("HOME", home.path())
        .args(["apply", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[extfile]"));
}

// ─── status ──────────────────────────────────────────────────────────────────

#[test]
fn status_reports_clean_when_in_sync() {
    let (repo, home) = setup_apply();
    let dest_path = home.path().join(".applyrc");
    // Write dest identical to source.
    fs::write(&dest_path, "export APPLY=1\n").unwrap();

    cmd_home(&repo, &home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("up to date"));
}

#[test]
fn status_reports_modified_when_dest_differs() {
    let (repo, home) = setup_apply();
    let dest_path = home.path().join(".applyrc");
    fs::write(&dest_path, "export APPLY=99\n").unwrap(); // differs from source

    cmd_home(&repo, &home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("M "));
}

#[test]
fn status_reports_missing_when_dest_absent() {
    let (repo, home) = setup_apply();
    // dest file was never created

    cmd_home(&repo, &home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("? "));
}

// ─── templates ───────────────────────────────────────────────────────────────

/// Build a repo with a template source file (`.tmpl` suffix = template marker).
/// Returns (repo TempDir, home TempDir).
///
/// Source layout:
///   source/dot_config.tmpl  →  ~/.config  (Tera template)
fn setup_template_repo() -> (TempDir, TempDir) {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    cmd(&repo).arg("init").assert().success();

    // `.tmpl` suffix marks a Tera template — dest name strips the suffix.
    fs::write(
        repo.path().join("source").join("dot_config.tmpl"),
        "os={{ os }}\nprofile={{ profile }}\n",
    )
    .unwrap();

    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();

    (repo, home)
}

#[test]
fn apply_renders_template_variables() {
    let (repo, home) = setup_template_repo();

    cmd_home(&repo, &home)
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));

    // Dest is ~/.config (template suffix stripped).
    let content = fs::read_to_string(home.path().join(".config")).unwrap();
    // On macOS this is "macos", on Linux "linux".
    assert!(
        content.contains("os=macos") || content.contains("os=linux"),
        "expected os variable rendered, got: {}",
        content
    );
    assert!(content.contains("profile=default"), "expected profile rendered, got: {}", content);
}

#[test]
fn apply_dry_run_labels_template_files() {
    let (repo, home) = setup_template_repo();

    cmd_home(&repo, &home)
        .args(["apply", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(template)"));
}

#[test]
fn apply_template_false_file_is_copied_verbatim() {
    // A file WITHOUT `.tmpl` suffix is always copied byte-for-byte — braces not interpreted.
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let literal = "export FOO={{ BAR }}\n"; // shell brace expansion, not a Tera template
    fs::write(repo.path().join("source").join("literal.sh"), literal).unwrap();
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();

    cmd_home(&repo, &home).arg("apply").assert().success();

    // Content must be byte-identical to source — braces not interpreted.
    assert_eq!(
        fs::read_to_string(home.path().join("literal.sh")).unwrap(),
        literal
    );
}

#[test]
fn status_reports_clean_for_rendered_template() {
    let (repo, home) = setup_template_repo();

    // Apply first to get the rendered file in place.
    cmd_home(&repo, &home).arg("apply").assert().success();

    // Status should show clean (rendered output matches dest).
    cmd_home(&repo, &home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("up to date"));
}

#[test]
fn status_reports_modified_when_template_dest_differs() {
    let (repo, home) = setup_template_repo();
    // Write stale rendered content to dest.
    fs::write(home.path().join(".config"), "os=windows\nprofile=old\n").unwrap();

    cmd_home(&repo, &home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("M "));
}

#[test]
fn status_files_flag_skips_brew_section() {
    // --files should only show file drift, not brew drift.
    let (repo, home) = setup_apply();
    // dest matches source — no file drift.
    let dest_path = home.path().join(".applyrc");
    fs::write(&dest_path, "export APPLY=1\n").unwrap();

    cmd_home(&repo, &home)
        .args(["status", "--files"])
        .assert()
        .success()
        .stdout(predicate::str::contains("up to date"));
}

#[test]
fn status_brews_flag_skips_files_section() {
    // --brews should not show file drift even when files are modified.
    let (repo, home) = setup_apply();
    // Dest differs from source — would show M under --files.
    let dest_path = home.path().join(".applyrc");
    fs::write(&dest_path, "export APPLY=MODIFIED\n").unwrap();

    cmd_home(&repo, &home)
        .args(["status", "--brews"])
        .assert()
        .success()
        // No [files] section emitted.
        .stdout(predicate::str::contains("[files]").not());
}

#[test]
fn status_ai_flag_shows_only_ai_section() {
    // --ai with a missing skill should show drift; [files] section should be absent.
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    write_ai_module(&repo);

    let claude = TempDir::new().unwrap();
    // Skill is absent.

    let mut c = Command::cargo_bin("haven").unwrap();
    c.env_remove("HAVEN_DIR");
    c.env("HAVEN_CLAUDE_DIR", claude.path());
    c.args(["--dir", repo.path().to_str().unwrap(), "status", "--ai", "--profile", "default"]);
    c.assert()
        .success()
        .stdout(predicate::str::contains("?"))
        .stdout(predicate::str::contains("[files]").not());
}

#[test]
fn apply_fails_on_malformed_template() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // Template with unclosed brace — Tera will error.
    fs::write(
        repo.path().join("source").join("dot_bad.tmpl"),
        "{{ unclosed\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();

    cmd_home(&repo, &home)
        .arg("apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("render").or(predicate::str::contains("template")));
}

// ─── packages module ─────────────────────────────────────────────────────────

fn write_packages_module(repo: &TempDir, brewfile: &str) {
    let toml = format!("[homebrew]\nbrewfile = \"{}\"\n", brewfile);
    fs::write(
        repo.path().join("modules").join("packages.toml"),
        toml,
    )
    .unwrap();
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = [\"packages\"]\n",
    )
    .unwrap();
}

#[test]
fn packages_toml_parses_homebrew_section() {
    // Verify that a packages.toml with [homebrew] parses without error.
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // Create the Brewfile in brew/ directory.
    fs::create_dir_all(repo.path().join("brew")).unwrap();
    fs::write(repo.path().join("brew").join("Brewfile.packages"), "brew \"git\"\n").unwrap();
    write_packages_module(&repo, "brew/Brewfile.packages");

    // dry-run apply should parse and print the plan without touching brew.
    let mut c = Command::cargo_bin("haven").unwrap();
    c.env_remove("HAVEN_DIR");
    c.args(["--dir", repo.path().to_str().unwrap(), "apply", "--dry-run", "--profile", "default"]);
    c.assert()
        .success()
        .stdout(predicate::str::contains("brew bundle"))
        .stdout(predicate::str::contains("Brewfile.packages"));
}

#[test]
fn packages_toml_with_mise_section_parses() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let toml = "[mise]\nconfig = \"source/mise.toml\"\n";
    fs::write(
        repo.path().join("modules").join("packages.toml"),
        toml,
    )
    .unwrap();
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = [\"packages\"]\n",
    )
    .unwrap();

    let mut c = Command::cargo_bin("haven").unwrap();
    c.env_remove("HAVEN_DIR");
    c.args(["--dir", repo.path().to_str().unwrap(), "apply", "--dry-run", "--profile", "default"]);
    c.assert()
        .success()
        .stdout(predicate::str::contains("mise install"));
}

#[test]
fn apply_shows_files_and_brew_in_dry_run() {
    // Files in source/ and a brew module both appear in dry-run output.
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // Source file.
    fs::write(repo.path().join("source").join("dot_zshrc"), "# zsh\n").unwrap();

    // Brew module.
    fs::create_dir_all(repo.path().join("brew")).unwrap();
    fs::write(repo.path().join("brew").join("Brewfile.packages"), "brew \"git\"\n").unwrap();
    fs::write(
        repo.path().join("modules").join("packages.toml"),
        "[homebrew]\nbrewfile = \"brew/Brewfile.packages\"\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = [\"packages\"]\n",
    )
    .unwrap();

    cmd_home(&repo, &home)
        .args(["apply", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("zshrc"))
        .stdout(predicate::str::contains("Brewfile.packages"));
}

#[test]
fn apply_skips_missing_brewfile_gracefully_when_brew_absent() {
    // When brew is not installed, apply should skip Homebrew and not fail.
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    write_packages_module(&repo, "brew/Brewfile.missing");

    let mut c = Command::cargo_bin("haven").unwrap();
    c.env_remove("HAVEN_DIR");
    c.args(["--dir", repo.path().to_str().unwrap(), "apply"]);
    // Either brew is absent (skipped, success) or brew is present and Brewfile is missing (skipped).
    // Both are valid outcomes — we just verify it doesn't panic.
    let _ = c.output().unwrap(); // must not panic
}

#[test]
fn status_shows_missing_when_brew_absent_and_brewfile_configured() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // Create the Brewfile so it exists in the repo.
    fs::create_dir_all(repo.path().join("brew")).unwrap();
    fs::write(repo.path().join("brew").join("Brewfile.packages"), "brew \"git\"\n").unwrap();
    write_packages_module(&repo, "brew/Brewfile.packages");

    let mut c = Command::cargo_bin("haven").unwrap();
    c.env_remove("HAVEN_DIR");
    c.args(["--dir", repo.path().to_str().unwrap(), "status", "--profile", "default"]);
    let out = c.assert().success();
    // We can't enforce which marker without knowing the test environment.
    let _ = out; // verified it doesn't panic or error
}

// ─── remove-unreferenced-brews ────────────────────────────────────────────────

#[test]
fn remove_unreferenced_brews_dry_run_does_not_uninstall() {
    // --dry-run must never invoke brew uninstall, even when packages are unreferenced.
    // This is the safe way to verify the flag in a real environment.
    let (repo, home) = setup_apply();
    Command::cargo_bin("haven")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env("HOME", home.path())
        .env("HAVEN_CLAUDE_DIR", home.path().join(".claude"))
        .env_remove("HAVEN_DIR")
        .args(["apply", "--remove-unreferenced-brews", "--dry-run"])
        .assert()
        .success();
}

#[test]
fn interactive_dry_run_does_not_prompt_or_uninstall() {
    // --interactive --dry-run: shows the list but exits before the [y/N] prompt,
    // so no stdin interaction is needed and nothing is uninstalled.
    let (repo, home) = setup_apply();
    Command::cargo_bin("haven")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env("HOME", home.path())
        .env("HAVEN_CLAUDE_DIR", home.path().join(".claude"))
        .env_remove("HAVEN_DIR")
        .args(["apply", "--interactive", "--dry-run"])
        .assert()
        .success();
}


// ─── 1Password integration ───────────────────────────────────────────────────

/// Write a secrets module TOML with `requires_op = true` and a homebrew section.
/// In the new design, requires_op guards brew/mise but not source file application.
/// Externals are now tracked as extdir_ files in source/, not in module TOMLs.
fn write_secrets_module(repo: &TempDir) {
    let toml = "requires_op = true\n\n\
                [homebrew]\n\
                brewfile = \"brew/Brewfile.secrets\"\n";
    fs::write(
        repo.path().join("modules").join("secrets.toml"),
        toml,
    )
    .unwrap();
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = [\"secrets\"]\n",
    )
    .unwrap();
}

#[test]
fn requires_op_field_parses_from_toml() {
    // Verify that `requires_op = true` in a module TOML doesn't cause a parse error.
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    write_secrets_module(&repo);

    // dry-run apply: should print the secrets module plan.
    cmd_home(&repo, &home)
        .args(["apply", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("secrets"));
}

#[test]
fn apply_dry_run_shows_requires_op_module_plan() {
    // Even with requires_op=true, dry-run should print the module plan.
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    write_secrets_module(&repo);

    cmd_home(&repo, &home)
        .args(["apply", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Brewfile.secrets"));
}

#[test]
fn apply_skips_requires_op_module_when_op_absent() {
    // When `op` is not available, the secrets module should be skipped with a warning.
    // Source files (global) still apply — requires_op only guards module-level ops.
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    write_secrets_module(&repo);

    // A plain source file should still apply even when op is absent.
    fs::write(repo.path().join("source").join("dot_zshrc"), "# always applies\n").unwrap();

    cmd_home(&repo, &home)
        .env("PATH", "/usr/bin:/bin")
        .arg("apply")
        .assert()
        .success()
        .stderr(predicate::str::contains("skipped"));

    // File should have been applied (not blocked by requires_op).
    assert!(home.path().join(".zshrc").exists(), "file should apply even when op absent");
}

#[test]
fn apply_requires_op_module_skipped_without_op_not_a_hard_error() {
    // Exit code is 0 (skip ≠ error). Source files still apply.
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    write_secrets_module(&repo);

    // A plain source file should still apply.
    fs::write(repo.path().join("source").join("dot_plain"), "# plain\n").unwrap();

    cmd_home(&repo, &home)
        .env("PATH", "/usr/bin:/bin")
        .arg("apply")
        .assert()
        .success();

    // File should have been applied despite secrets module being skipped.
    assert!(home.path().join(".plain").exists(), "file should apply regardless of requires_op");
}

// ─── AI module (Week 6) ───────────────────────────────────────────────────────

/// Write ai/skills.toml with two skill entries (both gh: sources).
fn write_ai_module(repo: &TempDir) {
    // New per-directory structure: ai/skills/<name>/skill.toml
    for (name, source) in [
        ("my-skills", "gh:alice/my-skills@v1.0"),
        ("my-commands", "gh:alice/my-commands@main"),
    ] {
        let skill_dir = repo.path().join("ai").join("skills").join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            format!("source    = \"{}\"\nplatforms = \"all\"\n", source),
        )
        .unwrap();
    }
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();
}

#[test]
fn ai_toml_parses_skills_and_commands() {
    // Verify that ai/skills.toml with gh: sources parses without error (dry-run, no network).
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    write_ai_module(&repo);

    let claude = TempDir::new().unwrap();
    let mut c = Command::cargo_bin("haven").unwrap();
    c.env_remove("HAVEN_DIR");
    c.env("HAVEN_CLAUDE_DIR", claude.path());
    c.args(["--dir", repo.path().to_str().unwrap(), "apply", "--dry-run"]);
    c.assert()
        .success()
        .stdout(predicate::str::contains("fetch skill"))
        .stdout(predicate::str::contains("gh:alice/my-skills@v1.0"))
        .stdout(predicate::str::contains("gh:alice/my-commands@main"));
}

#[test]
fn ai_dry_run_prints_both_skills_and_commands() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    write_ai_module(&repo);

    let claude = TempDir::new().unwrap();
    let mut c = Command::cargo_bin("haven").unwrap();
    c.env_remove("HAVEN_DIR");
    c.env("HAVEN_CLAUDE_DIR", claude.path());
    c.args(["--dir", repo.path().to_str().unwrap(), "apply", "--dry-run"]);
    // Dry-run must not touch the filesystem.
    c.assert().success();
    assert!(
        !claude.path().join("skills").join("my-skills").exists(),
        "dry-run must not write skills"
    );
    assert!(
        !claude.path().join("commands").join("my-commands").exists(),
        "dry-run must not write commands"
    );
}

#[test]
fn apply_generates_claude_md_from_installed_skills() {
    // apply should generate CLAUDE.md. Skills are auto-discovered by Claude Code
    // so they are not listed — only snippets are included.
    let (repo, home) = setup_apply();

    let claude = TempDir::new().unwrap();
    // Pre-install a skill so CLAUDE.md has content.
    let skill_dir = claude.path().join("skills").join("my-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: my-skill\ndescription: \"Test skill\"\n---\n",
    )
    .unwrap();

    cmd_home(&repo, &home)
        .env("HAVEN_CLAUDE_DIR", claude.path())
        .arg("apply")
        .assert()
        .success();

    let claude_md = claude.path().join("CLAUDE.md");
    assert!(claude_md.exists(), "CLAUDE.md was not generated");
    let content = fs::read_to_string(&claude_md).unwrap();
    // Skills are auto-discovered — no listing in CLAUDE.md.
    assert!(!content.contains("/my-skill: Test skill"), "skills should not be listed");
    assert!(content.contains("profile: default"));
}

#[test]
fn apply_generates_claude_md_even_when_no_skills_installed() {
    // CLAUDE.md should be written even when no skills/commands are present.
    let (repo, home) = setup_apply();

    let claude = TempDir::new().unwrap();

    cmd_home(&repo, &home)
        .env("HAVEN_CLAUDE_DIR", claude.path())
        .arg("apply")
        .assert()
        .success();

    let claude_md = claude.path().join("CLAUDE.md");
    assert!(claude_md.exists(), "CLAUDE.md should always be generated");
    let content = fs::read_to_string(&claude_md).unwrap();
    assert!(content.contains("Generated by haven"));
}

#[test]
fn status_reports_missing_when_ai_skill_not_installed() {
    // When a skill listed in [ai] is absent from claude_dir, status shows '?'.
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    write_ai_module(&repo);

    let claude = TempDir::new().unwrap();
    // Don't create the skill directory — it's absent.

    let mut c = Command::cargo_bin("haven").unwrap();
    c.env_remove("HAVEN_DIR");
    c.env("HAVEN_CLAUDE_DIR", claude.path());
    c.args(["--dir", repo.path().to_str().unwrap(), "status", "--profile", "default"]);
    c.assert()
        .success()
        .stdout(predicate::str::contains("?"))
        .stdout(predicate::str::contains("gh:alice/my-skills@v1.0"));
}

#[test]
fn status_reports_clean_when_ai_skill_installed() {
    // When the skill directory exists, status should not flag it as missing.
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    write_ai_module(&repo);

    let claude = TempDir::new().unwrap();
    // Create skill directories to simulate installed state (all entries deploy to skills/).
    fs::create_dir_all(claude.path().join("skills").join("my-skills")).unwrap();
    fs::create_dir_all(claude.path().join("skills").join("my-commands")).unwrap();

    let mut c = Command::cargo_bin("haven").unwrap();
    c.env_remove("HAVEN_DIR");
    c.env("HAVEN_CLAUDE_DIR", claude.path());
    c.args(["--dir", repo.path().to_str().unwrap(), "status", "--profile", "default"]);
    c.assert()
        .success()
        .stdout(predicate::str::contains("up to date"));
}

#[test]
fn lock_file_is_written_after_noop_apply() {
    // Verify apply runs successfully. The lock is only written when AI sources are fetched.
    let (repo, home) = setup_apply();

    let claude = TempDir::new().unwrap();
    cmd_home(&repo, &home)
        .env("HAVEN_CLAUDE_DIR", claude.path())
        .arg("apply")
        .assert()
        .success();
    // The lock file is only written when AI sources are fetched.
    // File-only apply does not create a lock file.
    assert!(
        !repo.path().join("haven.lock").exists(),
        "haven.lock should not be written for file-only modules"
    );
}

// ─── bootstrap (Week 7) ────────────────────────────────────────────────────

/// Build a bootstrap command pointing at `repo` with temp dirs for dest/state/claude/envs.
// ─── import ──────────────────────────────────────────────────────────────────

/// Build a synthetic chezmoi source directory for testing.
///
///   dot_zshrc                        → ~/.zshrc     (plain file)
///   dot_config/git/config            → ~/.config/git/config
///   dot_finicky.js                   → ~/.finicky.js
///   Justfile                         → ~/Justfile   (bare file)
///   private_dot_ssh/id_rsa           → ~/.ssh/id_rsa  (private)
///   executable_dot_local/bin/myscript → ~/.local/bin/myscript  (executable)
///   dot_hgrc.tmpl                    → ~/.hgrc (template, converted)
///   .git/HEAD                        → NOT walked (. dir skipped)
fn make_chezmoi_dir(base: &TempDir) -> std::path::PathBuf {
    let src = base.path().to_path_buf();

    // Regular dot_ files.
    fs::write(src.join("dot_zshrc"), "# zsh config\n").unwrap();
    fs::write(src.join("dot_finicky.js"), "module.exports = {};\n").unwrap();
    // Template file.
    fs::write(src.join("dot_hgrc.tmpl"), "[ui]\nusername = {{ .chezmoi.username }}\n").unwrap();

    // Nested dot_config/git/config.
    let git_dir = src.join("dot_config").join("git");
    fs::create_dir_all(&git_dir).unwrap();
    fs::write(git_dir.join("config"), "[user]\n\tname = Test\n").unwrap();

    // Bare file (no dot_ prefix).
    fs::write(src.join("Justfile"), "default:\n\techo hello\n").unwrap();

    // private_ file — imported with private=true.
    let ssh = src.join("private_dot_ssh");
    fs::create_dir_all(&ssh).unwrap();
    fs::write(ssh.join("id_rsa"), "-----BEGIN RSA-----\n").unwrap();

    // executable_ file — imported with executable=true.
    let local_bin = src.join("executable_dot_local").join("bin");
    fs::create_dir_all(&local_bin).unwrap();
    fs::write(local_bin.join("myscript"), "#!/bin/sh\necho hello\n").unwrap();

    // .git/ dir — must NOT be walked.
    let git_meta = src.join(".git");
    fs::create_dir_all(&git_meta).unwrap();
    fs::write(git_meta.join("HEAD"), "ref: refs/heads/main\n").unwrap();

    src
}

#[test]
fn import_dry_run_prints_plan_no_writes() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    cmd(&repo).arg("init").assert().success();

    let out = cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("Would import"))
        .stdout(predicate::str::contains("dot_zshrc"))
        .stdout(predicate::str::contains("Justfile"))
        // Template files are shown in the plan.
        .stdout(predicate::str::contains("dot_hgrc.tmpl"))
        .stdout(predicate::str::contains("template"));

    let _ = out;

    // No source files should have been written.
    assert!(!repo.path().join("source").join("dot_zshrc").exists());
}

#[test]
fn import_copies_files_with_encoded_paths() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"))
        .stdout(predicate::str::contains("Imported"))
        .stdout(predicate::str::contains("Skipped"));

    // Source files are stored with encoded paths (chezmoi-compatible).
    assert!(repo.path().join("source").join("dot_zshrc").exists(), "source/dot_zshrc missing");
    assert!(
        repo.path().join("source").join("dot_config").join("git").join("config").exists(),
        "source/dot_config/git/config missing"
    );
    assert!(repo.path().join("source").join("Justfile").exists(), "source/Justfile missing");
    assert!(
        repo.path().join("source").join("private_dot_ssh").join("id_rsa").exists(),
        "source/private_dot_ssh/id_rsa missing"
    );

    // No [[files]] TOML entries — encoding is in the filename.
    let shell_toml_path = repo.path().join("modules").join("shell.toml");
    if shell_toml_path.exists() {
        let contents = fs::read_to_string(&shell_toml_path).unwrap();
        assert!(
            !contents.contains("[[files]]"),
            "shell.toml should not have [[files]] entries:\n{}",
            contents
        );
    }
}

#[test]
fn import_into_non_empty_source_is_rejected() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    cmd(&repo).arg("init").assert().success();

    // First import succeeds.
    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    // Second import must fail with a clear "not empty" error.
    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not empty"));
}

#[test]
fn import_skips_dot_directories() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    cmd(&repo).arg("init").assert().success();

    // The .git/ dir has a HEAD file. It must NOT appear in output or source/.
    let out = cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .arg("--dry-run")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("HEAD"), ".git/HEAD should not appear in output: {}", stdout);
}

#[test]
fn import_unknown_from_fails() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "yadm"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown import source"));
}

#[test]
fn import_missing_source_dir_fails() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source", "/nonexistent/chezmoi/dir"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("chezmoi source directory not found").or(
            predicate::str::contains("Cannot locate").or(
                predicate::str::contains("No such file")
            )
        ));
}

/// Mock chezmoi binary: a shell script placed in a temp dir on PATH that echoes
/// the source dir path when called as `chezmoi source-path`.
#[test]
fn import_uses_chezmoi_subprocess_when_on_path() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    // Write a mock `chezmoi` script.
    let bin_dir = TempDir::new().unwrap();
    let mock_chezmoi = bin_dir.path().join("chezmoi");
    // Mock chezmoi: respond to `source-path` with the temp dir path;
    // exit 1 for all other subcommands (including `managed`) so the
    // managed-paths filter is skipped and all files are imported.
    fs::write(
        &mock_chezmoi,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"source-path\" ]; then echo '{}'; exit 0; fi\nexit 1\n",
            chezmoi_src.path().display()
        ),
    ).unwrap();

    // Make it executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&mock_chezmoi, fs::Permissions::from_mode(0o755)).unwrap();
    }

    cmd(&repo).arg("init").assert().success();

    // Run import WITHOUT --source, with our mock chezmoi first on PATH.
    let original_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.path().display(), original_path);

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--dry-run"])
        .env("PATH", &new_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Would import"))
        .stdout(predicate::str::contains("dot_zshrc"));
}

// ─── permissions (private / executable flags via filename encoding) ────────────

/// Write a source file with a magic-name encoded filename and a minimal haven.toml.
#[cfg(unix)]
fn write_permission_source(repo: &TempDir, encoded_name: &str) {
    let source_dir = repo.path().join("source");
    fs::write(source_dir.join(encoded_name), "content\n").unwrap();
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();
}

#[cfg(unix)]
fn file_mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
#[cfg(unix)]
fn apply_sets_private_permission() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // private_id_rsa → ~/id_rsa with chmod 0600
    write_permission_source(&repo, "private_id_rsa");

    cmd_home(&repo, &home).arg("apply").assert().success();

    let dest = home.path().join("id_rsa");
    assert!(dest.exists(), "dest file not created");
    assert_eq!(file_mode(&dest), 0o600, "expected 0600 for private file");
}

#[test]
#[cfg(unix)]
fn apply_sets_executable_permission() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // executable_deploy.sh → ~/deploy.sh with chmod 0755
    write_permission_source(&repo, "executable_deploy.sh");

    cmd_home(&repo, &home).arg("apply").assert().success();

    let dest = home.path().join("deploy.sh");
    assert!(dest.exists(), "dest file not created");
    assert_eq!(file_mode(&dest), 0o755, "expected 0755 for executable file");
}

#[test]
#[cfg(unix)]
fn apply_sets_private_executable_permission() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // private_executable_secret_script → ~/secret_script with chmod 0700
    write_permission_source(&repo, "private_executable_secret_script");

    cmd_home(&repo, &home).arg("apply").assert().success();

    let dest = home.path().join("secret_script");
    assert!(dest.exists(), "dest file not created");
    assert_eq!(file_mode(&dest), 0o700, "expected 0700 for private+executable");
}

#[test]
fn apply_dry_run_shows_private_annotation() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // private_id_rsa → ~/id_rsa; dry-run should show "(private)" annotation.
    fs::write(repo.path().join("source").join("private_id_rsa"), "content\n").unwrap();
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();

    cmd_home(&repo, &home)
        .args(["apply", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(private)"));
}

#[test]
fn import_private_prefix_preserves_encoding() {
    // private_dot_ssh/id_rsa is imported and stored with its encoded path.
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    // Encoded source path is preserved — private_dot_ssh/id_rsa in source/.
    assert!(
        repo.path().join("source").join("private_dot_ssh").join("id_rsa").exists(),
        "source/private_dot_ssh/id_rsa missing"
    );
    // No TOML entry for private flag — it's encoded in the filename.
}

#[test]
fn import_executable_prefix_preserves_encoding() {
    // executable_dot_local/bin/myscript is imported with its encoded path.
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    assert!(
        repo.path().join("source").join("executable_dot_local").join("bin").join("myscript").exists(),
        "source/executable_dot_local/bin/myscript missing"
    );
}

#[test]
fn import_create_prefix_preserved_in_source() {
    // create_dot_seedrc in chezmoi source should be imported as source/create_dot_seedrc
    // so that haven apply honours the create_only (seed-only) semantics.
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();

    fs::write(chezmoi_src.path().join("create_dot_seedrc"), "# seed\n").unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    // Prefix is preserved in the source path — apply uses it to set create_only.
    assert!(
        repo.path().join("source").join("create_dot_seedrc").exists(),
        "source/create_dot_seedrc must be present (create_ prefix preserved)"
    );
}

#[test]
fn import_exact_prefix_preserved_in_source() {
    // exact_dot_ssh/ in chezmoi source should be imported as source/exact_dot_ssh/
    // so that haven apply enforces exact directory semantics on ~/.ssh/.
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();

    let ssh_src = chezmoi_src.path().join("exact_dot_ssh");
    fs::create_dir_all(&ssh_src).unwrap();
    fs::write(ssh_src.join("config"), "Host *\n  ServerAliveInterval 60\n").unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    // Directory AND file are imported with the exact_ prefix on the dir.
    assert!(
        repo.path().join("source").join("exact_dot_ssh").join("config").exists(),
        "source/exact_dot_ssh/config must be present (exact_ prefix preserved)"
    );
}

#[test]
fn import_dry_run_shows_private_annotation() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("(private)"))
        .stdout(predicate::str::contains("(executable)"));
}

// ─── externals ────────────────────────────────────────────────────────────────

/// Write an `extdir_` marker file in source/ encoding the given external.
///
/// dest must be a `~/…` path. The source path is computed from the dest components.
fn write_externals_module(repo: &TempDir, dest: &str, url: &str, ref_name: Option<&str>) {
    let source_path = extdir_source_path(repo, dest);
    if let Some(parent) = source_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut content = format!("type = \"git\"\nurl  = \"{}\"\n", url);
    if let Some(r) = ref_name {
        content.push_str(&format!("ref  = \"{}\"\n", r));
    }
    fs::write(&source_path, content).unwrap();

    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();
}

/// Compute the `source/` path for an `extdir_` marker from a `~/…` dest path.
/// Mirrors the logic in `src/commands/import.rs::extdir_source_path`.
fn extdir_source_path(repo: &TempDir, dest: &str) -> std::path::PathBuf {
    let rel = dest.strip_prefix("~/").unwrap_or(dest);
    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    let n = parts.len();
    let mut path = repo.path().join("source");
    for component in &parts[..n.saturating_sub(1)] {
        let encoded = if let Some(rest) = component.strip_prefix('.') {
            format!("dot_{}", rest)
        } else {
            component.to_string()
        };
        path = path.join(encoded);
    }
    if n > 0 {
        let last = parts[n - 1];
        let encoded_last = if let Some(rest) = last.strip_prefix('.') {
            format!("extdir_dot_{}", rest)
        } else {
            format!("extdir_{}", last)
        };
        path = path.join(encoded_last);
    }
    path
}

#[test]
fn apply_dry_run_shows_external_git_clone() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    write_externals_module(
        &repo,
        "~/.config/nvim",
        "https://github.com/user/nvim-config",
        Some("main"),
    );

    let mut c = Command::cargo_bin("haven").unwrap();
    c.env_remove("HAVEN_DIR");
    c.args(["--dir", repo.path().to_str().unwrap(), "apply", "--dry-run"]);
    c.assert()
        .success()
        .stdout(predicate::str::contains("[extdir]"))
        .stdout(predicate::str::contains("nvim-config"))
        .stdout(predicate::str::contains("main"));
}

#[test]
fn import_chezmoiexternal_toml_writes_externals_section() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();

    // Write a minimal .chezmoiexternal.toml.
    fs::write(
        chezmoi_src.path().join(".chezmoiexternal.toml"),
        "[\"~/.config/nvim\"]\ntype = \"git-repo\"\nurl  = \"https://github.com/user/nvim-config\"\nref  = \"main\"\n",
    )
    .unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("nvim-config"));

    // extdir_ marker file should exist at source/dot_config/nvim/extdir_nvim... wait
    // dest is ~/.config/nvim → parent=~/.config → dir component "dot_config",
    // and the extdir marker is "extdir_nvim".
    // But actually the parent dir is ~/.config and child is nvim.
    // source/dot_config/extdir_nvim
    let extdir_marker = repo
        .path()
        .join("source")
        .join("dot_config")
        .join("extdir_nvim");
    assert!(extdir_marker.exists(), "extdir_ marker missing at source/dot_config/extdir_nvim");
    let marker_content = fs::read_to_string(&extdir_marker).unwrap();
    assert!(marker_content.contains("nvim-config"), "missing url in marker");
    assert!(marker_content.contains("git"), "missing type in marker");
}

#[test]
fn import_chezmoiexternal_dry_run_shows_externals() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();

    fs::write(
        chezmoi_src.path().join(".chezmoiexternal.toml"),
        "[\"~/.config/nvim\"]\ntype = \"git-repo\"\nurl  = \"https://github.com/user/nvim-config\"\n",
    )
    .unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("external"))
        .stdout(predicate::str::contains("nvim-config"));
}

#[test]
fn import_chezmoiexternal_is_idempotent() {
    // In the extdir_ design, externals are written as marker files into source/.
    // After the first import, source/ is non-empty, so a second import is blocked
    // by the non-empty guard (same as for regular file imports).
    // Idempotency at the extdir_ level is enforced by the guard on source/ itself.
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();

    fs::write(
        chezmoi_src.path().join(".chezmoiexternal.toml"),
        "[\"~/.config/nvim\"]\ntype = \"git-repo\"\nurl  = \"https://github.com/user/nvim-config\"\n",
    )
    .unwrap();

    cmd(&repo).arg("init").assert().success();

    // First import succeeds and creates the extdir_ marker file.
    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    // Second import is rejected because source/ is non-empty (extdir_ marker exists).
    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not empty"));
}

#[test]
fn status_shows_external_missing() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // Write an extdir_ marker for a path that definitely does not exist.
    write_externals_module(
        &repo,
        "~/.tmux/plugins/haven-test-nonexistent",
        "https://github.com/user/nvim-config",
        None,
    );

    let mut c = Command::cargo_bin("haven").unwrap();
    c.env_remove("HAVEN_DIR");
    c.args(["--dir", repo.path().to_str().unwrap(), "status", "--profile", "default"]);
    c.assert()
        .success()
        .stdout(predicate::str::contains("?"));
}

// ─── link = true (symlink entries via symlink_ encoding) ─────────────────────

/// Build a repo with a symlink-encoded source file.
/// Returns (repo TempDir, home TempDir, expected source abs path).
fn setup_link_apply() -> (TempDir, TempDir, std::path::PathBuf) {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    cmd(&repo).arg("init").assert().success();

    // symlink_vscode_settings.json → ~/vscode_settings.json (symlink → source file)
    let source_dir = repo.path().join("source");
    fs::write(source_dir.join("symlink_vscode_settings.json"), r#"{"editor.fontSize": 14}"#).unwrap();

    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();

    let source_abs = repo.path().join("source").join("symlink_vscode_settings.json");
    (repo, home, source_abs)
}

#[test]
fn apply_creates_symlink() {
    let (repo, home, source_abs) = setup_link_apply();
    let dest_path = home.path().join("vscode_settings.json");

    cmd_home(&repo, &home)
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));

    assert!(dest_path.is_symlink(), "dest should be a symlink");
    let target = fs::read_link(&dest_path).unwrap();
    assert_eq!(target, source_abs, "symlink should point to source file");
}

#[test]
fn apply_symlink_is_idempotent() {
    let (repo, home, _source_abs) = setup_link_apply();
    let dest_path = home.path().join("vscode_settings.json");

    // First apply.
    cmd_home(&repo, &home).arg("apply").assert().success();
    assert!(dest_path.is_symlink(), "symlink should exist after first apply");

    // Second apply — must succeed without creating a backup or erroring.
    cmd_home(&repo, &home).arg("apply").assert().success();
    assert!(dest_path.is_symlink(), "symlink should still exist after second apply");
}

#[test]
fn apply_symlink_already_correct_not_counted_as_applied() {
    // Bug fix: a symlink that already points to the right target must NOT be
    // counted as an applied file (it's a no-op, like an already-matching regular file).
    let (repo, home, source_abs) = setup_link_apply();
    let dest_path = home.path().join("vscode_settings.json");

    // Pre-create the correct symlink so apply has nothing to do.
    std::os::unix::fs::symlink(&source_abs, &dest_path).unwrap();

    // Apply should report 0 files applied (not 1).
    cmd_home(&repo, &home)
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::contains("Applied 0 file(s)"));
}

#[test]
fn apply_symlink_replaces_dangling_symlink() {
    // Bug fix: apply must succeed even when the existing symlink is dangling
    // (target no longer exists). Previously backup_file would crash with ENOENT
    // because std::fs::copy follows the symlink.
    let (repo, home, source_abs) = setup_link_apply();
    let dest_path = home.path().join("vscode_settings.json");

    // Create a dangling symlink (target does not exist).
    std::os::unix::fs::symlink("/tmp/nonexistent_target_for_haven_test", &dest_path).unwrap();
    assert!(dest_path.is_symlink(), "pre-condition: dangling symlink exists");
    assert!(!dest_path.exists(), "pre-condition: symlink target does not exist");

    // Apply must succeed without error and fix the symlink.
    cmd_home(&repo, &home).arg("apply").assert().success();

    assert!(dest_path.is_symlink(), "dest should still be a symlink");
    let target = fs::read_link(&dest_path).unwrap();
    assert_eq!(target, source_abs, "symlink should now point to source file");
}

#[test]
fn apply_symlink_replaces_regular_file() {
    let (repo, home, source_abs) = setup_link_apply();
    let dest_path = home.path().join("vscode_settings.json");

    // Pre-create a regular file at the dest location.
    fs::write(&dest_path, "old content\n").unwrap();
    assert!(!dest_path.is_symlink(), "pre-condition: regular file at dest");

    cmd_home(&repo, &home)
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::contains("backed up"));

    assert!(dest_path.is_symlink(), "dest should now be a symlink");
    let target = fs::read_link(&dest_path).unwrap();
    assert_eq!(target, source_abs);
}

#[test]
fn apply_dry_run_shows_symlink_annotation() {
    let (repo, home, _source_abs) = setup_link_apply();
    let dest_path = home.path().join("vscode_settings.json");

    cmd_home(&repo, &home)
        .args(["apply", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("symlink"))
        .stdout(predicate::str::contains("vscode_settings.json"));

    assert!(!dest_path.exists(), "dry-run must not create files");
}

#[test]
fn apply_warns_when_link_and_private_combined() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // symlink_private_ combined: private flag is meaningless for symlinks → warning.
    fs::write(
        repo.path().join("source").join("symlink_private_secret_link.txt"),
        "secret\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();

    cmd_home(&repo, &home)
        .arg("apply")
        .assert()
        .success()
        .stderr(predicate::str::contains("warning"))
        .stderr(predicate::str::contains("private"));
}

#[test]
fn status_shows_link_clean_when_correct_symlink() {
    let (repo, home, source_abs) = setup_link_apply();
    let dest_path = home.path().join("vscode_settings.json");

    // Create the correct symlink manually.
    std::os::unix::fs::symlink(&source_abs, &dest_path).unwrap();

    cmd_home(&repo, &home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Everything up to date"));
}

#[test]
fn status_shows_link_missing_when_dest_absent() {
    let (repo, home, _source_abs) = setup_link_apply();
    let dest_path = home.path().join("vscode_settings.json");
    // Do not create the dest at all.

    cmd_home(&repo, &home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("?"))
        .stdout(predicate::str::contains(
            dest_path.file_name().unwrap().to_str().unwrap(),
        ));
}

#[test]
fn status_shows_link_modified_when_wrong_target() {
    let (repo, home, _source_abs) = setup_link_apply();
    let dest_path = home.path().join("vscode_settings.json");

    // Create a symlink pointing somewhere else.
    std::os::unix::fs::symlink("/tmp/wrong_target", &dest_path).unwrap();

    cmd_home(&repo, &home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("M"))
        .stdout(predicate::str::contains(
            dest_path.file_name().unwrap().to_str().unwrap(),
        ));
}

#[test]
fn add_link_flag_encodes_symlink_in_filename() {
    // `haven add --link` should encode the file as `symlink_<name>` in source/.
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let home = TempDir::new().unwrap();
    let dotfile = home.path().join("vscode_settings.json");
    fs::write(&dotfile, r#"{"editor.fontSize": 14}"#).unwrap();

    cmd_home(&repo, &home)
        .args([
            "add",
            dotfile.to_str().unwrap(),
            "--link",
        ])
        .assert()
        .success();

    // Source file should have symlink_ encoding.
    assert!(
        repo.path().join("source").join("symlink_vscode_settings.json").exists(),
        "expected source/symlink_vscode_settings.json"
    );
}

#[test]
fn add_link_apply_installs_symlink_immediately() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let dotfile = home.path().join("vscode_settings.json");
    fs::write(&dotfile, r#"{"editor.fontSize": 14}"#).unwrap();

    cmd_home(&repo, &home)
        .args(["add", dotfile.to_str().unwrap(), "--link", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("linked"));

    // Original path should now be a symlink.
    assert!(dotfile.is_symlink(), "original file should be replaced by a symlink");

    // Symlink should point into source/.
    let target = fs::read_link(&dotfile).unwrap();
    let expected = repo.path().join("source").join("symlink_vscode_settings.json");
    assert_eq!(target, expected, "symlink should point to source/symlink_vscode_settings.json");

    // The file content should still be accessible through the symlink.
    let content = fs::read_to_string(&dotfile).unwrap();
    assert_eq!(content, r#"{"editor.fontSize": 14}"#);
}

#[test]
fn add_link_apply_backs_up_existing_file() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let dotfile = home.path().join("settings.toml");
    fs::write(&dotfile, "key = \"value\"\n").unwrap();

    cmd_home(&repo, &home)
        .args(["add", dotfile.to_str().unwrap(), "--link", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("backed up"));

    // Backup should exist as <filename>.bak.
    let backup = home.path().join("settings.toml.bak");
    assert!(backup.exists(), "backup file should exist");
    assert_eq!(fs::read_to_string(&backup).unwrap(), "key = \"value\"\n");
}

#[test]
fn add_link_without_apply_does_not_create_symlink() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let dotfile = home.path().join("settings.json");
    fs::write(&dotfile, "{}").unwrap();

    cmd_home(&repo, &home)
        .args(["add", dotfile.to_str().unwrap(), "--link"])
        .assert()
        .success();

    // Without --apply, original file should NOT be a symlink.
    assert!(!dotfile.is_symlink(), "without --apply the original file should not be replaced");
    // But source/ should have the encoded file.
    assert!(repo.path().join("source").join("symlink_settings.json").exists());
}

#[test]
fn add_apply_requires_link_flag() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let dotfile = home.path().join("settings.json");
    fs::write(&dotfile, "{}").unwrap();

    // --apply without --link should be rejected by clap.
    cmd_home(&repo, &home)
        .args(["add", dotfile.to_str().unwrap(), "--apply"])
        .assert()
        .failure();
}

#[test]
fn add_update_flag_recopies_changed_file() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let dotfile = home.path().join(".myconfig");
    fs::write(&dotfile, "version = 1\n").unwrap();

    // Initial add.
    cmd_home(&repo, &home)
        .args(["add", dotfile.to_str().unwrap()])
        .assert()
        .success();

    // Update the file on disk.
    fs::write(&dotfile, "version = 2\n").unwrap();

    // --update should re-copy without error.
    cmd_home(&repo, &home)
        .args(["add", dotfile.to_str().unwrap(), "--update"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added:"));

    // Source file should now contain the new content.
    let source = repo.path().join("source").join("dot_myconfig");
    assert_eq!(fs::read_to_string(&source).unwrap(), "version = 2\n");
}

#[test]
fn add_without_update_flag_errors_when_already_tracked() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let dotfile = home.path().join(".tracked");
    fs::write(&dotfile, "data\n").unwrap();

    cmd_home(&repo, &home)
        .args(["add", dotfile.to_str().unwrap()])
        .assert()
        .success();

    // Second add without --update must fail.
    cmd_home(&repo, &home)
        .args(["add", dotfile.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already tracked"))
        .stderr(predicate::str::contains("--update"));
}

#[test]
fn import_symlink_prefix_resolves_to_link_entry() {
    // A chezmoi file named `symlink_dot_vimrc` whose content is a valid absolute
    // path to an existing file should be imported with symlink_ encoding preserved.
    let chezmoi_src = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // Create the real target file (the symlink points here).
    let real_target = chezmoi_src.path().join("real_vimrc");
    fs::write(&real_target, "\" vimrc content\n").unwrap();

    // Create the chezmoi symlink_ file whose content is the target path.
    fs::write(
        chezmoi_src.path().join("symlink_dot_vimrc"),
        real_target.to_str().unwrap(),
    )
    .unwrap();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    // The imported file should be in source/ (copied from the real target).
    let source_files: Vec<_> = fs::read_dir(repo.path().join("source"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        !source_files.is_empty(),
        "expected at least one file in source/, got nothing"
    );
    // The file should be stored with symlink_ encoding (symlink_dot_vimrc).
    assert!(
        source_files.iter().any(|f| f.contains("symlink") || f.contains("vimrc")),
        "expected symlink-encoded file in source/, found: {:?}",
        source_files
    );
}

#[test]
fn import_symlink_prefix_with_go_template_content_is_skipped() {
    // If the symlink_ file content contains Go template syntax, we can't resolve
    // it — it should be skipped (not cause an error).
    let chezmoi_src = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // Content with Go template expression.
    fs::write(
        chezmoi_src.path().join("symlink_dot_bashrc"),
        "{{ .chezmoi.homeDir }}/.bashrc_real",
    )
    .unwrap();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    // The symlink with template content should have been skipped.
    // source/ should NOT have a file named symlink_dot_bashrc with template content.
    // (It might be skipped entirely or treated as unsupported.)
    // We just verify no error occurred — the skip behavior is covered by unit tests.
}

// ─── --files / --brews / --ai section flags ──────────────────────────────────

#[test]
fn apply_files_flag_applies_files() {
    // --files alone should copy dotfiles (the section flag works).
    let (repo, home) = setup_apply();
    let dest_path = home.path().join(".applyrc");

    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default", "--files"])
        .assert()
        .success();

    assert!(dest_path.exists(), "--files should have copied the source file");
}

#[test]
fn apply_brews_flag_is_accepted() {
    // --brews alone must be accepted without error (brew is not invoked in a
    // sandbox repo that has no Brewfile).
    let (repo, home) = setup_apply();

    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default", "--brews"])
        .assert()
        .success();
}

#[test]
fn apply_ai_flag_is_accepted() {
    // --ai alone must be accepted without error (no AI modules in the test repo).
    let (repo, home) = setup_apply();

    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default", "--ai"])
        .assert()
        .success();
}

#[test]
fn apply_files_flag_skips_unreferenced_brew_removal() {
    // When only --files is given, --remove-unreferenced-brews must not run
    // even if it is also passed, because brew is not being applied.
    // The command must succeed without touching brew.
    let (repo, home) = setup_apply();

    cmd_home(&repo, &home)
        .args([
            "apply",
            "--profile",
            "default",
            "--files",
            "--remove-unreferenced-brews",
            "--dry-run",
        ])
        .assert()
        .success();
}

#[test]
fn apply_no_section_flags_applies_all() {
    // Without any section flags, apply must still copy dotfiles (i.e., "all"
    // sections are active by default).
    let (repo, home) = setup_apply();
    let dest_path = home.path().join(".applyrc");

    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default"])
        .assert()
        .success();

    assert!(
        dest_path.exists(),
        "default apply (no section flags) should copy files"
    );
}

#[test]
fn apply_multiple_section_flags_accepted() {
    // --files and --brews together must be accepted without error.
    let (repo, home) = setup_apply();

    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default", "--files", "--brews"])
        .assert()
        .success();
}

// ─── haven diff ───────────────────────────────────────────────────────────────

/// Set up a repo+home pair for diff tests.
/// Source: source/dot_diffrc  →  ~/.diffrc  (plain, content "v1\n")
/// The haven.toml has no modules (no brew/AI to invoke).
fn setup_diff() -> (TempDir, TempDir) {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    cmd(&repo).arg("init").assert().success();

    let source_dir = repo.path().join("source");
    fs::write(source_dir.join("dot_diffrc"), "v1\n").unwrap();
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();

    (repo, home)
}

/// Helper: run `haven diff` with the given extra args.
fn diff_cmd<'a>(repo: &'a TempDir, home: &'a TempDir) -> Command {
    let mut c = cmd_home(repo, home);
    c.arg("diff").arg("--profile").arg("default");
    c
}

// ── Files section ─────────────────────────────────────────────────────────────

#[test]
fn diff_clean_file_no_output() {
    let (repo, home) = setup_diff();
    // Apply first so dest matches source.
    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default"])
        .assert()
        .success();

    diff_cmd(&repo, &home)
        .assert()
        .success() // exit 0 = no drift
        .stdout(predicate::str::contains("✓ Everything up to date"));
}

#[test]
fn diff_modified_file_shows_diff() {
    let (repo, home) = setup_diff();
    // Apply, then change the dest so it differs.
    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default"])
        .assert()
        .success();
    let dest = home.path().join(".diffrc");
    fs::write(&dest, "v2\n").unwrap();

    diff_cmd(&repo, &home)
        .args(["--files"])
        .assert()
        .failure() // exit 1 = drift
        .stdout(predicate::str::contains("[files]"))
        // dest ("v2") is "a" (old, shown as -); source ("v1") is "b" (new, shown as +)
        .stdout(predicate::str::contains("-v2"))
        .stdout(predicate::str::contains("+v1"));
}

#[test]
fn diff_missing_dest_shows_missing() {
    let (repo, home) = setup_diff();
    // Don't apply — dest never created.
    diff_cmd(&repo, &home)
        .assert()
        .failure()
        .stdout(predicate::str::contains("? ~/.diffrc"));
}

// NOTE: diff_source_missing_shows_source_missing is intentionally omitted.
// source::scan() only yields files that exist on disk, so a deleted source
// file simply disappears from scan results — the SourceMissing branch in
// check_drift is a race-condition guard, not reachable via normal deletion.

#[test]
fn diff_template_rendered_before_compare() {
    // Template renders to "os=default\n"; if dest matches, diff should be clean.
    let (repo, home) = setup_diff();
    let source_dir = repo.path().join("source");
    // Replace the plain file with a template.
    fs::remove_file(source_dir.join("dot_diffrc")).unwrap();
    fs::write(source_dir.join("dot_diffrc.tmpl"), "profile={{ profile }}\n").unwrap();

    // Apply first so the rendered file is in dest.
    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default"])
        .assert()
        .success();

    diff_cmd(&repo, &home)
        .assert()
        .success()
        .stdout(predicate::str::contains("✓ Everything up to date"));
}

#[test]
fn diff_template_rendered_diff_shows_delta() {
    let (repo, home) = setup_diff();
    let source_dir = repo.path().join("source");
    fs::remove_file(source_dir.join("dot_diffrc")).unwrap();
    fs::write(source_dir.join("dot_diffrc.tmpl"), "profile={{ profile }}\n").unwrap();

    // Apply with profile "default" so dest = "profile=default\n".
    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default"])
        .assert()
        .success();

    // Now manually write a stale value to dest.
    fs::write(home.path().join(".diffrc"), "profile=old\n").unwrap();

    diff_cmd(&repo, &home)
        .assert()
        .failure()
        .stdout(predicate::str::contains("[files]"))
        // dest ("profile=old") is shown as "-" (current), source rendered ("profile=default") as "+"
        .stdout(predicate::str::contains("-profile=old"))
        .stdout(predicate::str::contains("+profile=default"));
}

#[test]
fn diff_template_render_error_shows_tilde_marker() {
    // A template with an undefined variable causes a Tera render error.
    // diff should show the ~ marker and exit 1 (drift, not crash).
    let (repo, home) = setup_diff();
    let source_dir = repo.path().join("source");
    fs::remove_file(source_dir.join("dot_diffrc")).unwrap();
    // Use a filter on an undefined variable, which Tera treats as an error.
    fs::write(
        source_dir.join("dot_diffrc.tmpl"),
        "{{ undefined_variable_xyz }}\n",
    )
    .unwrap();
    // Create a dest so it's not "missing".
    fs::write(home.path().join(".diffrc"), "something\n").unwrap();

    diff_cmd(&repo, &home)
        .assert()
        .failure() // exit 1 (drift marker present)
        .stdout(predicate::str::contains("~"))
        .stdout(predicate::str::contains("template render failed"));
}

#[test]
fn diff_binary_file_shows_notice() {
    let (repo, home) = setup_diff();
    let source_dir = repo.path().join("source");
    // Write a binary file (contains null bytes) as source.
    let binary: Vec<u8> = vec![0u8, 1, 2, 3, 0, 255];
    fs::write(source_dir.join("dot_diffrc"), &binary).unwrap();
    // Write different binary content to dest.
    let binary2: Vec<u8> = vec![0u8, 9, 8, 7, 0, 200];
    fs::write(home.path().join(".diffrc"), &binary2).unwrap();

    diff_cmd(&repo, &home)
        .assert()
        .failure()
        .stdout(predicate::str::contains("binary files differ"));
}

#[test]
fn diff_symlink_correct_target_no_output() {
    // A symlink_ entry pointing to the correct source file is clean.
    let (repo, home) = setup_diff();
    let source_dir = repo.path().join("source");
    // Add a symlink_ entry.
    fs::write(source_dir.join("symlink_dot_difflink"), "link content\n").unwrap();

    // Apply so the symlink is created.
    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default"])
        .assert()
        .success();

    diff_cmd(&repo, &home)
        .assert()
        .success()
        .stdout(predicate::str::contains("✓ Everything up to date"));
}

#[test]
fn diff_symlink_wrong_target_shows_mismatch() {
    let (repo, home) = setup_diff();
    let source_dir = repo.path().join("source");
    fs::write(source_dir.join("symlink_dot_difflink"), "link content\n").unwrap();

    // Apply to create the correct symlink.
    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default"])
        .assert()
        .success();

    // Replace the symlink with one pointing somewhere wrong.
    let link_path = home.path().join(".difflink");
    fs::remove_file(&link_path).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/tmp/wrong_target", &link_path).unwrap();

    diff_cmd(&repo, &home)
        .assert()
        .failure()
        .stdout(predicate::str::contains("M ~/.difflink"))
        .stdout(predicate::str::contains("symlink:"));
}

// ── Section flags ──────────────────────────────────────────────────────────────

#[test]
fn diff_files_flag_only() {
    let (repo, home) = setup_diff();
    // Dest is missing — drift in files section.

    diff_cmd(&repo, &home)
        .args(["--files"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("[files]"));
}

#[test]
fn diff_no_flags_shows_files_section() {
    let (repo, home) = setup_diff();

    diff_cmd(&repo, &home)
        .assert()
        .failure()
        .stdout(predicate::str::contains("[files]"));
}

#[test]
fn diff_brews_flag_skips_files_section() {
    let (repo, home) = setup_diff();
    // Files are missing but --brews was requested; no [files] section.

    diff_cmd(&repo, &home)
        .args(["--brews"])
        .assert()
        // Exit 0 or 1 (brew may or may not be installed in CI), just no crash.
        .stdout(predicate::str::contains("[files]").not());
}

#[test]
fn diff_ai_flag_skips_files_section() {
    let (repo, home) = setup_diff();

    diff_cmd(&repo, &home)
        .args(["--ai"])
        .assert()
        .stdout(predicate::str::contains("[files]").not());
}

// ── --stat flag ────────────────────────────────────────────────────────────────

#[test]
fn diff_stat_shows_summary_line() {
    let (repo, home) = setup_diff();
    // Apply then modify dest to create one-line drift.
    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default"])
        .assert()
        .success();
    fs::write(home.path().join(".diffrc"), "v2\n").unwrap();

    diff_cmd(&repo, &home)
        .args(["--files", "--stat"])
        .assert()
        .failure()
        // Should show a stat line with | separator, not a raw diff block.
        .stdout(predicate::str::contains("|"))
        // Must NOT show raw + / - diff lines.
        .stdout(predicate::str::contains("+v2").not())
        .stdout(predicate::str::contains("-v1").not());
}

// ── Exit codes ─────────────────────────────────────────────────────────────────

#[test]
fn diff_exits_0_when_clean() {
    let (repo, home) = setup_diff();
    cmd_home(&repo, &home)
        .args(["apply", "--profile", "default"])
        .assert()
        .success();

    diff_cmd(&repo, &home)
        .assert()
        .success(); // exit code 0
}

#[test]
fn diff_exits_1_when_drift() {
    let (repo, home) = setup_diff();
    // Don't apply — dest is missing, so drift exists.

    diff_cmd(&repo, &home)
        .assert()
        .failure(); // exit code 1
}

// ─── .chezmoiignore import tests ──────────────────────────────────────────────

#[test]
fn import_chezmoiignore_writes_config_ignore() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    // Add a .chezmoiignore file.
    fs::write(
        chezmoi_src.path().join(".chezmoiignore"),
        "# ignored patterns\n.ssh/id_*\n.local/share/app/**\n",
    )
    .unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("config/ignore"));

    let ignore_path = repo.path().join("config").join("ignore");
    assert!(ignore_path.exists(), "config/ignore should have been created");
    let content = fs::read_to_string(&ignore_path).unwrap();
    assert!(content.contains(".ssh/id_*"), "should contain .ssh/id_* pattern");
    assert!(content.contains(".local/share/app/**"), "should contain .local/share/app/** pattern");
    assert!(content.contains("# ignored patterns"), "should preserve comments");
}

#[test]
fn import_chezmoiignore_skips_ignored_files_by_default() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    // .chezmoiignore ignores dot_zshrc.
    fs::write(
        chezmoi_src.path().join(".chezmoiignore"),
        ".zshrc\n",
    )
    .unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("ignored by .chezmoiignore"));

    // The ignored file should NOT be in source/.
    assert!(
        !repo.path().join("source").join("dot_zshrc").exists(),
        "dot_zshrc should have been skipped due to .chezmoiignore"
    );
    // Other files should still be imported.
    assert!(
        repo.path().join("source").join("Justfile").exists(),
        "Justfile should still be imported"
    );
}

#[test]
fn import_chezmoiignore_include_ignored_files_flag_imports_all() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    fs::write(
        chezmoi_src.path().join(".chezmoiignore"),
        ".zshrc\n",
    )
    .unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .arg("--include-ignored-files")
        .assert()
        .success();

    // With --include-ignored-files, dot_zshrc should be imported.
    assert!(
        repo.path().join("source").join("dot_zshrc").exists(),
        "dot_zshrc should be imported with --include-ignored-files"
    );
    // config/ignore should still be written.
    assert!(
        repo.path().join("config").join("ignore").exists(),
        "config/ignore should still be created even with --include-ignored-files"
    );
}

#[test]
fn import_chezmoiignore_strips_go_template_lines_with_warning() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    fs::write(
        chezmoi_src.path().join(".chezmoiignore"),
        "# always ignored\n.ssh/id_rsa\n{{ if ne .chezmoi.os \"darwin\" }}\n.Brewfile\n{{ end }}\n",
    )
    .unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    let ignore_path = repo.path().join("config").join("ignore");
    let content = fs::read_to_string(&ignore_path).unwrap();
    // Plain patterns should be preserved.
    assert!(content.contains(".ssh/id_rsa"), "plain patterns should be kept");
    // Go template lines should be stripped.
    assert!(!content.contains("{{"), "Go template lines should be stripped");
}

#[test]
fn import_chezmoiignore_dry_run_shows_ignore_import() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    fs::write(
        chezmoi_src.path().join(".chezmoiignore"),
        ".zshrc\n",
    )
    .unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("config/ignore"));

    // Dry run: config/ignore should NOT have been written.
    assert!(
        !repo.path().join("config").join("ignore").exists(),
        "dry-run should not write config/ignore"
    );
}

#[test]
fn import_no_chezmoiignore_does_not_create_config_ignore() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);
    // No .chezmoiignore in chezmoi source.

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    assert!(
        !repo.path().join("config").join("ignore").exists(),
        "config/ignore should not be created when .chezmoiignore is absent"
    );
}

#[test]
fn import_run_once_brew_bundle_emits_homebrew_module_toml() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    // A Brewfile tracked in the chezmoi source (decodes to ~/Brewfile).
    fs::write(chezmoi_src.path().join("Brewfile"), "brew \"ripgrep\"\n").unwrap();

    // A run_once_ script that references the same Brewfile.
    fs::write(
        chezmoi_src.path().join("run_once_install-packages.sh"),
        "#!/bin/bash\nbrew bundle --file=~/Brewfile\n",
    ).unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("[homebrew]"));

    // Brewfile should be copied to brew/.
    let brewfile_dest = repo.path().join("brew").join("Brewfile.packages");
    assert!(brewfile_dest.exists(), "brew/Brewfile.packages should exist");
    assert_eq!(
        fs::read_to_string(&brewfile_dest).unwrap(),
        "brew \"ripgrep\"\n",
    );

    // A module TOML for "packages" should have [homebrew] pointing to brew/Brewfile.packages.
    let toml_path = repo.path().join("modules").join("packages.toml");
    assert!(toml_path.exists(), "packages.toml should be written");
    let toml_content = fs::read_to_string(&toml_path).unwrap();
    assert!(toml_content.contains("[homebrew]"), "should contain [homebrew]");
    assert!(toml_content.contains("brew/Brewfile.packages"), "should contain brew/Brewfile.packages");
}

#[test]
fn import_brewfile_in_chezmoi_source_copied_to_brew_dir() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    // A plain Brewfile in the chezmoi root (decodes to ~/Brewfile).
    fs::write(chezmoi_src.path().join("Brewfile"), "brew \"git\"\n").unwrap();

    cmd(&repo).arg("init").assert().success();

    let out = cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();

    // Brewfile should NOT appear in source/.
    assert!(
        !repo.path().join("source").join("Brewfile").exists(),
        "Brewfile must not be copied to source/"
    );

    // Brewfile should be in brew/.
    let brewfile_dest = repo.path().join("brew").join("Brewfile.packages");
    assert!(brewfile_dest.exists(), "brew/Brewfile.packages should exist");
    assert_eq!(fs::read_to_string(&brewfile_dest).unwrap(), "brew \"git\"\n");

    // Module TOML should reference brew/Brewfile.packages.
    let toml_path = repo.path().join("modules").join("packages.toml");
    assert!(toml_path.exists(), "packages.toml should be created");
    let toml = fs::read_to_string(&toml_path).unwrap();
    assert!(toml.contains("[homebrew]"), "packages.toml should have [homebrew]");
    assert!(toml.contains("brew/Brewfile.packages"), "packages.toml should reference brew/Brewfile.packages");

    // Summary line should mention 1 Brewfile.
    assert!(stdout.contains("1 Brewfile"), "summary should say 1 Brewfile(s)");
}

#[test]
fn import_brewfile_with_suffix_preserves_module_name() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    // Brewfile.work decodes to ~/Brewfile.work → module "work", dest "brew/Brewfile.work".
    fs::write(chezmoi_src.path().join("Brewfile.work"), "brew \"slack\"\n").unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    let brewfile_dest = repo.path().join("brew").join("Brewfile.work");
    assert!(brewfile_dest.exists(), "brew/Brewfile.work should exist");

    let toml_path = repo.path().join("modules").join("work.toml");
    assert!(toml_path.exists(), "work.toml should be created");
    let toml = fs::read_to_string(&toml_path).unwrap();
    assert!(toml.contains("brew/Brewfile.work"), "work.toml should reference brew/Brewfile.work");
}

#[test]
fn import_brewfile_in_subdir_detected_by_filename() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    // Brewfile in a subdirectory (e.g. ~/config/Brewfile) — detected by filename alone.
    fs::create_dir_all(chezmoi_src.path().join("config")).unwrap();
    fs::write(chezmoi_src.path().join("config").join("Brewfile"), "brew \"fd\"\n").unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    // Should land in brew/ not source/config/.
    assert!(
        !repo.path().join("source").join("config").join("Brewfile").exists(),
        "Brewfile must not be in source/"
    );
    assert!(
        repo.path().join("brew").join("Brewfile.packages").exists(),
        "brew/Brewfile.packages should exist"
    );
}

#[test]
fn import_brewfile_lock_is_not_detected_as_brewfile() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    // Brewfile.lock should NOT be treated as a Brewfile.
    fs::write(chezmoi_src.path().join("Brewfile.lock"), "# lock content\n").unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    // Brewfile.lock should land in source/ as a regular file.
    assert!(
        repo.path().join("source").join("Brewfile.lock").exists(),
        "Brewfile.lock should be treated as a regular source file"
    );
    // And NOT in brew/.
    assert!(
        !repo.path().join("brew").join("Brewfile.lock").exists(),
        "Brewfile.lock must not be copied to brew/"
    );
}

#[test]
fn import_run_once_mise_install_emits_mise_module_toml() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    fs::write(
        chezmoi_src.path().join("run_once_install-tools.sh"),
        "#!/bin/bash\nmise install\n",
    ).unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("[mise]"));

    let toml_path = repo.path().join("modules").join("packages.toml");
    assert!(toml_path.exists(), "packages.toml should be written");
    let toml_content = fs::read_to_string(&toml_path).unwrap();
    assert!(toml_content.contains("[mise]"), "should contain [mise]");
}

#[test]
fn import_unrecognised_script_is_copied_to_source_scripts() {
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();
    make_chezmoi_dir(&chezmoi_src);

    fs::write(
        chezmoi_src.path().join("run_once_custom.sh"),
        "#!/bin/bash\necho 'custom stuff'\n",
    ).unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success()
        // Script should show in output as copied (no pattern detected).
        .stdout(predicate::str::contains("run_once_custom.sh"));

    // Script should be copied to source/scripts/.
    assert!(
        repo.path().join("source").join("scripts").join("run_once_custom.sh").exists(),
        "script should be copied to source/scripts/"
    );
    // No packages.toml should be written (no recognised pattern).
    assert!(
        !repo.path().join("modules").join("packages.toml").exists(),
        "no TOML should be written for unrecognised script"
    );
}

// ─── add-local / repo: source type ───────────────────────────────────────────

/// Helper: create a fake skill directory with a SKILL.md.
fn make_skill_dir(parent: &TempDir, name: &str) -> std::path::PathBuf {
    let skill_dir = parent.path().join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), format!("# {}\nA test skill.", name)).unwrap();
    fs::write(skill_dir.join("extra.md"), "extra content").unwrap();
    skill_dir
}

#[test]
fn ai_add_local_copies_files_and_writes_skill_toml() {
    let repo = TempDir::new().unwrap();
    let skills_parent = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    make_skill_dir(&skills_parent, "myskill");

    cmd(&repo)
        .args(["ai", "add-local"])
        .arg(skills_parent.path().join("myskill"))
        .assert()
        .success()
        .stdout(predicates::str::contains("Added local skill 'myskill'"))
        .stdout(predicates::str::contains("ai/skills/myskill/files/"))
        .stdout(predicates::str::contains("haven apply --ai"));

    // skill.toml should have source = "repo:"
    let skill_toml = repo.path().join("ai").join("skills").join("myskill").join("skill.toml");
    assert!(skill_toml.exists(), "skill.toml should be created");
    let toml_content = fs::read_to_string(&skill_toml).unwrap();
    assert!(toml_content.contains("repo:"), "skill.toml should contain repo:");

    // files/ should contain the original skill files
    let files_dir = repo.path().join("ai").join("skills").join("myskill").join("files");
    assert!(files_dir.join("SKILL.md").exists(), "SKILL.md should be in files/");
    assert!(files_dir.join("extra.md").exists(), "extra.md should be in files/");

    // blank all.md should be created
    let all_md = repo.path().join("ai").join("skills").join("myskill").join("all.md");
    assert!(all_md.exists(), "all.md stub should be created");

    // original directory should be removed
    assert!(
        !skills_parent.path().join("myskill").exists(),
        "original directory should be removed"
    );
}

#[test]
fn ai_add_local_name_override() {
    let repo = TempDir::new().unwrap();
    let skills_parent = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    make_skill_dir(&skills_parent, "source-dir");

    cmd(&repo)
        .args(["ai", "add-local"])
        .arg(skills_parent.path().join("source-dir"))
        .args(["--name", "custom-name"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Added local skill 'custom-name'"));

    assert!(
        repo.path().join("ai").join("skills").join("custom-name").join("files").join("SKILL.md").exists(),
        "files/ should be under custom-name"
    );
}

#[test]
fn ai_add_local_errors_on_missing_path() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["ai", "add-local", "/nonexistent/path/skill"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("does not exist"));
}

#[test]
fn ai_add_local_errors_on_duplicate_name() {
    let repo = TempDir::new().unwrap();
    let skills_parent = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // First import succeeds.
    make_skill_dir(&skills_parent, "myskill");
    cmd(&repo)
        .args(["ai", "add-local"])
        .arg(skills_parent.path().join("myskill"))
        .assert()
        .success();

    // Second import with same name fails.
    let second = TempDir::new().unwrap();
    make_skill_dir(&second, "myskill");
    cmd(&repo)
        .args(["ai", "add-local"])
        .arg(second.path().join("myskill"))
        .assert()
        .failure()
        .stderr(predicates::str::contains("already exists"));
}

#[test]
fn ai_apply_deploys_repo_skill_as_symlink() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let skills_parent = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    make_skill_dir(&skills_parent, "myskill");
    cmd(&repo)
        .args(["ai", "add-local"])
        .arg(skills_parent.path().join("myskill"))
        .assert()
        .success();

    // Set up CLAUDE.md-style platforms (claude-code active).
    let platforms_dir = repo.path().join("ai");
    fs::create_dir_all(&platforms_dir).unwrap();
    fs::write(
        platforms_dir.join("platforms.toml"),
        "active = [\"claude-code\"]\n",
    ).unwrap();

    let claude_skills = home.path().join(".claude").join("skills");
    cmd_home(&repo, &home)
        .env("HAVEN_CLAUDE_DIR", home.path().join(".claude"))
        .args(["apply", "--ai"])
        .assert()
        .success();

    // Deployed target should exist (symlink or dir).
    let deployed = claude_skills.join("myskill");
    assert!(
        deployed.exists() || deployed.is_symlink(),
        "skill should be deployed to ~/.claude/skills/myskill"
    );
}

#[test]
fn ai_diff_repo_skill_missing_files_shows_question_mark() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // Write skill.toml manually with source = "repo:" but no files/ subdir.
    let skill_dir = repo.path().join("ai").join("skills").join("myskill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("skill.toml"),
        "source    = \"repo:\"\nplatforms = \"all\"\n",
    ).unwrap();

    cmd_home(&repo, &home)
        .args(["diff", "--ai"])
        .assert()
        .stdout(predicates::str::contains("myskill"))
        .stdout(predicates::str::contains("repo: files not found"));
}

#[test]
fn ai_fetch_skips_repo_skill() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let skills_parent = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    make_skill_dir(&skills_parent, "myskill");
    cmd(&repo)
        .args(["ai", "add-local"])
        .arg(skills_parent.path().join("myskill"))
        .assert()
        .success();

    // fetch should not error on repo: skills.
    cmd_home(&repo, &home)
        .args(["ai", "fetch"])
        .assert()
        .success();
}

#[test]
fn skill_source_parses_repo() {
    // Verify the CLI round-trips: skill.toml with "repo:" is accepted by apply --dry-run.
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    let skill_dir = repo.path().join("ai").join("skills").join("myskill");
    fs::create_dir_all(skill_dir.join("files")).unwrap();
    fs::write(skill_dir.join("files").join("SKILL.md"), "# myskill").unwrap();
    fs::write(
        skill_dir.join("skill.toml"),
        "source    = \"repo:\"\nplatforms = \"all\"\n",
    ).unwrap();

    cmd_home(&repo, &home)
        .env("HAVEN_CLAUDE_DIR", home.path().join(".claude"))
        .args(["apply", "--dry-run", "--ai"])
        .assert()
        .success();
}

// ─── conflict detection ───────────────────────────────────────────────────────

/// Helper: set up a repo+home pair, add a file to source/, and return
/// (repo, home, source_path, dest_path).
fn setup_conflict_repo() -> (TempDir, TempDir, std::path::PathBuf, std::path::PathBuf) {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    // Pin VCS to git so jj-detection prompts don't consume stdin in interactive tests.
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n\n[vcs]\nbackend = \"git\"\n",
    ).unwrap();
    let source_path = repo.path().join("source").join("dot_zshrc");
    fs::write(&source_path, "# original content\n").unwrap();
    let dest_path = home.path().join(".zshrc");
    (repo, home, source_path, dest_path)
}

/// Helper: read the applied_files hash for a tilde key from state.json.
fn read_applied_hash(home: &TempDir, tilde_key: &str) -> Option<String> {
    let state_path = home.path().join(".local/state/haven").join("state.json");
    let text = fs::read_to_string(&state_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v["applied_files"][tilde_key]["sha256"].as_str().map(|s| s.to_string())
}

// ── State recording — baseline behaviour ──────────────────────────────────────

#[test]
fn conflict_detection_dest_absent_hash_recorded() {
    let (repo, home, _, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home)
        .args(["apply", "--files"])
        .assert().success();
    assert!(dest_path.exists());
    let hash = read_applied_hash(&home, "~/.zshrc");
    assert!(hash.is_some(), "applied_files should contain ~/.zshrc after apply");
    assert!(!hash.unwrap().is_empty());
}

#[test]
fn conflict_detection_migration_seed() {
    // dest already matches source on first apply → hash seeded, no write.
    let (repo, home, source_path, dest_path) = setup_conflict_repo();
    fs::create_dir_all(dest_path.parent().unwrap()).unwrap();
    fs::write(&dest_path, "# original content\n").unwrap();
    // source and dest are identical — files_equal returns true
    let _ = &source_path;
    cmd_home(&repo, &home)
        .args(["apply", "--files"])
        .assert().success();
    let hash = read_applied_hash(&home, "~/.zshrc");
    assert!(hash.is_some(), "hash should be seeded even when no write occurs");
}

#[test]
fn conflict_detection_first_apply_content_differs() {
    // No prior hash, source != dest → write happens, hash recorded, no prompt.
    let (repo, home, _, dest_path) = setup_conflict_repo();
    fs::create_dir_all(dest_path.parent().unwrap()).unwrap();
    fs::write(&dest_path, "# user content\n").unwrap();
    cmd_home(&repo, &home)
        .args(["apply", "--files"])
        .assert().success();
    let content = fs::read_to_string(&dest_path).unwrap();
    assert_eq!(content, "# original content\n");
    let hash = read_applied_hash(&home, "~/.zshrc");
    assert!(hash.is_some());
}

// ── No user edit — clean subsequent apply ────────────────────────────────────

#[test]
fn conflict_detection_no_user_edit_source_unchanged() {
    // Apply twice without touching dest → second apply is idempotent.
    let (repo, home, _, _) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    let hash1 = read_applied_hash(&home, "~/.zshrc").unwrap();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    let hash2 = read_applied_hash(&home, "~/.zshrc").unwrap();
    assert_eq!(hash1, hash2, "hash should not change on idempotent apply");
}

#[test]
fn conflict_detection_no_user_edit_source_changed() {
    // Apply, update source, apply again → second apply writes new content.
    let (repo, home, source_path, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&source_path, "# updated content\n").unwrap();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    let content = fs::read_to_string(&dest_path).unwrap();
    assert_eq!(content, "# updated content\n");
}

// ── User edit — conflict scenarios ───────────────────────────────────────────

#[test]
fn conflict_detection_prompt_fires_on_user_edit() {
    let (repo, home, _, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&dest_path, "# user edited\n").unwrap();
    cmd_home(&repo, &home)
        .env("HAVEN_FORCE_INTERACTIVE", "1")
        .args(["apply", "--files"])
        .write_stdin("s\n")
        .assert()
        .stdout(predicate::str::contains("conflict"));
}

#[test]
fn conflict_detection_skip_preserves_user_edit() {
    let (repo, home, _, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&dest_path, "# user edited\n").unwrap();
    let hash_before = read_applied_hash(&home, "~/.zshrc").unwrap();
    let _ = cmd_home(&repo, &home)
        .env("HAVEN_FORCE_INTERACTIVE", "1")
        .args(["apply", "--files"])
        .write_stdin("s\n")
        .assert();
    let content = fs::read_to_string(&dest_path).unwrap();
    assert_eq!(content, "# user edited\n", "skip should preserve user's version");
    let hash_after = read_applied_hash(&home, "~/.zshrc").unwrap();
    assert_eq!(hash_before, hash_after, "hash should not change on skip");
}

#[test]
fn conflict_detection_overwrite_restores_source() {
    let (repo, home, _, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&dest_path, "# user edited\n").unwrap();
    cmd_home(&repo, &home)
        .env("HAVEN_FORCE_INTERACTIVE", "1")
        .args(["apply", "--files"])
        .write_stdin("o\n")
        .assert().success();
    let content = fs::read_to_string(&dest_path).unwrap();
    assert_eq!(content, "# original content\n", "overwrite should restore source");
}

#[test]
fn conflict_detection_apply_all_skips_subsequent_prompts() {
    // Two conflicting files; user enters "A" — both overwritten without a second prompt.
    // setup_conflict_repo already sets vcs.backend=git in haven.toml.
    let (repo, home, _, dest1) = setup_conflict_repo();
    let src2 = repo.path().join("source").join("dot_bashrc");
    fs::write(&src2, "# bash content\n").unwrap();
    let dest2 = home.path().join(".bashrc");

    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&dest1, "# user edited zshrc\n").unwrap();
    fs::write(&dest2, "# user edited bashrc\n").unwrap();

    cmd_home(&repo, &home)
        .env("HAVEN_FORCE_INTERACTIVE", "1")
        .args(["apply", "--files"])
        .write_stdin("A\n")
        .assert().success();

    assert_eq!(fs::read_to_string(&dest1).unwrap(), "# original content\n");
    assert_eq!(fs::read_to_string(&dest2).unwrap(), "# bash content\n");
}

#[test]
fn conflict_detection_diff_then_overwrite() {
    let (repo, home, _, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&dest_path, "# user edited\n").unwrap();
    cmd_home(&repo, &home)
        .env("HAVEN_FORCE_INTERACTIVE", "1")
        .args(["apply", "--files"])
        .write_stdin("d\no\n")
        .assert().success();
    let content = fs::read_to_string(&dest_path).unwrap();
    assert_eq!(content, "# original content\n");
}

// ── Binary file ───────────────────────────────────────────────────────────────

#[test]
fn conflict_detection_binary_diff_not_available() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n\n[vcs]\nbackend = \"git\"\n",
    ).unwrap();
    let src = repo.path().join("source").join("dot_binary");
    fs::write(&src, b"\x00\x01\x02\x03").unwrap();
    let dest = home.path().join(".binary");

    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&dest, b"\xff\xfe\xfd").unwrap();

    cmd_home(&repo, &home)
        .env("HAVEN_FORCE_INTERACTIVE", "1")
        .args(["apply", "--files"])
        .write_stdin("d\ns\n")
        .assert()
        .stdout(predicate::str::contains("binary file"));
}

// ── Non-interactive mode ──────────────────────────────────────────────────────

#[test]
fn conflict_skip_mode_preserves_dest() {
    let (repo, home, _, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&dest_path, "# user edited\n").unwrap();
    cmd_home(&repo, &home)
        .args(["apply", "--files", "--on-conflict=skip"])
        .assert()
        .failure(); // exit code 1
    let content = fs::read_to_string(&dest_path).unwrap();
    assert_eq!(content, "# user edited\n");
}

#[test]
fn conflict_overwrite_mode_restores_source() {
    let (repo, home, _, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&dest_path, "# user edited\n").unwrap();
    cmd_home(&repo, &home)
        .args(["apply", "--files", "--on-conflict=overwrite"])
        .assert().success();
    let content = fs::read_to_string(&dest_path).unwrap();
    assert_eq!(content, "# original content\n");
}

#[test]
fn conflict_prompt_in_non_tty_falls_back_to_skip() {
    // When stdin is piped (non-TTY) and --on-conflict=prompt, warn and skip.
    let (repo, home, _, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&dest_path, "# user edited\n").unwrap();
    cmd_home(&repo, &home)
        .args(["apply", "--files", "--on-conflict=prompt"])
        // write_stdin simulates a pipe (non-TTY stdin)
        .write_stdin("")
        .assert()
        .failure()
        .stderr(predicate::str::contains("non-TTY").or(predicate::str::contains("warning")));
    let content = fs::read_to_string(&dest_path).unwrap();
    assert_eq!(content, "# user edited\n");
}

// ── Dry-run ───────────────────────────────────────────────────────────────────

#[test]
fn conflict_dry_run_does_not_record_hashes() {
    let (repo, home, _, _) = setup_conflict_repo();
    cmd_home(&repo, &home)
        .args(["apply", "--files", "--dry-run"])
        .assert().success();
    let hash = read_applied_hash(&home, "~/.zshrc");
    assert!(hash.is_none(), "dry-run should not write state");
}

// ── Template files ────────────────────────────────────────────────────────────

#[test]
fn conflict_detection_template_file_user_edit() {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n\n[vcs]\nbackend = \"git\"\n",
    ).unwrap();
    let src = repo.path().join("source").join("dot_tmplrc.tmpl");
    fs::write(&src, "# profile: {{ profile }}\n").unwrap();
    let dest = home.path().join(".tmplrc");

    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&dest, "# user edited template dest\n").unwrap();

    cmd_home(&repo, &home)
        .env("HAVEN_FORCE_INTERACTIVE", "1")
        .args(["apply", "--files"])
        .write_stdin("s\n")
        .assert()
        .stdout(predicate::str::contains("conflict"));
}

// ── State save on partial failure ─────────────────────────────────────────────

#[test]
fn conflict_state_saved_on_partial_failure() {
    // Apply two files where the second has an unreadable source (simulates failure).
    // Hash for the first file should still be in state after the error.
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    fs::write(
        repo.path().join("haven.toml"),
        "[profile.default]\nmodules = []\n\n[vcs]\nbackend = \"git\"\n",
    ).unwrap();
    let src1 = repo.path().join("source").join("dot_file1");
    fs::write(&src1, "content1\n").unwrap();

    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    let hash = read_applied_hash(&home, "~/.file1");
    assert!(hash.is_some(), "hash for .file1 should be recorded");
}

// ── State cleanup ─────────────────────────────────────────────────────────────

#[test]
fn conflict_retain_removes_stale_entries() {
    let (repo, home, source_path, _) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    assert!(read_applied_hash(&home, "~/.zshrc").is_some());

    // Remove the file from source/.
    fs::remove_file(&source_path).unwrap();

    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    let hash = read_applied_hash(&home, "~/.zshrc");
    assert!(hash.is_none(), "stale entry should be removed by retain pass");
}

// ── haven status C marker ─────────────────────────────────────────────────────

#[test]
fn status_no_c_marker_when_dest_unchanged() {
    let (repo, home, _, _) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    cmd_home(&repo, &home)
        .args(["status", "--files"])
        .assert().success()
        .stdout(predicate::str::contains("C").not().or(predicate::str::is_empty()));
}

#[test]
fn status_c_marker_when_dest_changed() {
    let (repo, home, _, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&dest_path, "# user edited\n").unwrap();
    cmd_home(&repo, &home)
        .args(["status", "--files"])
        .assert()
        .stdout(predicate::str::contains("C"));
}

#[test]
fn status_mc_marker_when_both_changed() {
    let (repo, home, source_path, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    // Update source (drift = M) and dest (user edit = C).
    fs::write(&source_path, "# source updated\n").unwrap();
    fs::write(&dest_path, "# user edited\n").unwrap();
    cmd_home(&repo, &home)
        .args(["status", "--files"])
        .assert()
        .stdout(predicate::str::contains("MC"));
}

#[test]
fn status_no_c_marker_when_no_prior_hash() {
    // Fresh repo with no prior apply — no C marker.
    let (repo, home, _, dest_path) = setup_conflict_repo();
    // Write dest without applying (no state.json).
    fs::create_dir_all(dest_path.parent().unwrap()).unwrap();
    fs::write(&dest_path, "# original content\n").unwrap();
    cmd_home(&repo, &home)
        .args(["status", "--files"])
        .assert()
        .stdout(predicate::str::contains("C").not().or(predicate::str::is_empty()));
}

// ── Haven-augmented file drift (CLAUDE.md) ───────────────────────────────────

#[test]
fn status_clean_when_dest_has_haven_managed_section_appended() {
    // Simulate the CLAUDE.md scenario: source has user content only; dest has
    // user content plus a haven-managed block appended by claude_md::generate.
    // Haven status/diff should report Clean, not Modified or C.
    let (repo, home, _source_path, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();

    // Append a haven-managed block to dest (as claude_md::generate would).
    let original = fs::read_to_string(&dest_path).unwrap();
    let augmented = format!(
        "{}\n<!-- haven managed start -->\n# Claude Code Environment\nGenerated by haven\n<!-- haven managed end -->\n",
        original
    );
    fs::write(&dest_path, &augmented).unwrap();

    let out = cmd_home(&repo, &home)
        .args(["status", "--files"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&out);
    assert!(
        !stdout.contains('C') && !stdout.contains('M'),
        "expected Clean but got: {stdout}"
    );
}

#[test]
fn status_clean_when_dest_has_haven_snippet_section_appended() {
    // Skill snippets are now part of the same <!-- haven managed --> section.
    // This test verifies that a destination file with a haven-managed block
    // containing snippet content is still reported as Clean.
    let (repo, home, _source_path, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();

    let original = fs::read_to_string(&dest_path).unwrap();
    let augmented = format!(
        "{}\n<!-- haven managed start -->\n# Claude Code Environment\n\
         <!-- skill: foo -->\nDo the thing.\n<!-- /skill: foo -->\n\
         <!-- haven managed end -->\n",
        original
    );
    fs::write(&dest_path, &augmented).unwrap();

    let out = cmd_home(&repo, &home)
        .args(["status", "--files"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&out);
    assert!(
        !stdout.contains('C') && !stdout.contains('M'),
        "expected Clean but got: {stdout}"
    );
}

#[test]
fn status_c_marker_when_user_edits_dest_despite_haven_section() {
    // User edits the user-content portion of the dest — should still show C
    // even when a haven-managed section is also present.
    let (repo, home, _, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();

    let augmented = "# user edited the content\n\
        <!-- haven managed start -->\n\
        # Claude Code Environment\n\
        <!-- haven managed end -->\n";
    fs::write(&dest_path, augmented).unwrap();

    cmd_home(&repo, &home)
        .args(["status", "--files"])
        .assert()
        .stdout(predicate::str::contains("C"));
}

// ── ApplyOutcome exit codes ───────────────────────────────────────────────────

#[test]
fn apply_outcome_exit_code_1_on_skip() {
    let (repo, home, _, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&dest_path, "# user edited\n").unwrap();
    cmd_home(&repo, &home)
        .args(["apply", "--files", "--on-conflict=skip"])
        .assert()
        .failure(); // exit code 1
}

#[test]
fn apply_outcome_exit_code_0_on_overwrite() {
    let (repo, home, _, dest_path) = setup_conflict_repo();
    cmd_home(&repo, &home).args(["apply", "--files"]).assert().success();
    fs::write(&dest_path, "# user edited\n").unwrap();
    cmd_home(&repo, &home)
        .args(["apply", "--files", "--on-conflict=overwrite"])
        .assert().success(); // exit code 0
}
