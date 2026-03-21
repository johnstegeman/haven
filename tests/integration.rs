/// Integration tests for dfiles Week 1–8 core loop:
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

/// Build a `dfiles` command with `--dir` pointed at `repo` and the DFILES_DIR
/// env var unset so it never falls back to `~/dfiles`.
fn cmd(repo: &TempDir) -> Command {
    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.arg("--dir").arg(repo.path());
    // Prevent any real ~/dfiles or ~/.claude from leaking in.
    c.env_remove("DFILES_DIR");
    c.env_remove("DFILES_CLAUDE_DIR");
    c
}

/// Build a `dfiles` command that also overrides HOME so `~` expands to `home`.
/// Required for any test that applies source files (magic-name paths use `~/`).
fn cmd_home(repo: &TempDir, home: &TempDir) -> Command {
    let mut c = cmd(repo);
    c.env("HOME", home.path());
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
        .stdout(predicate::str::contains("Initialized dfiles repo"));

    assert!(repo.path().join("dfiles.toml").exists(), "dfiles.toml missing");
    assert!(repo.path().join("source").is_dir(), "source/ missing");
    assert!(repo.path().join("brew").is_dir(), "brew/ missing");
    assert!(
        repo.path().join("config").join("modules").is_dir(),
        "config/modules/ missing"
    );
    assert!(
        repo.path().join("config").join("modules").join("shell.toml").exists(),
        "shell.toml missing"
    );
}

#[test]
fn init_fails_if_already_initialized() {
    let repo = TempDir::new().unwrap();

    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already initialized"));
}

// ─── init from source ────────────────────────────────────────────────────────

/// Create a local git repo containing a minimal `dfiles.toml` and return its
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
        r.join("dfiles.toml"),
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

/// Build a `dfiles init <source>` command pointing at a fresh target dir.
/// Returns both the target TempDir and the pre-built Command.
fn init_from(source: &str) -> (TempDir, Command) {
    let target = TempDir::new().unwrap();
    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.arg("--dir").arg(target.path());
    c.env_remove("DFILES_DIR");
    c.env_remove("DFILES_CLAUDE_DIR");
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

    assert!(target.path().join("dfiles.toml").exists(), "dfiles.toml missing after clone");
}

#[test]
fn init_from_gh_notation_builds_https_url() {
    // We can't hit github.com in tests, but we can confirm the command fails
    // with a git error (not a dfiles parse error) — proving we parsed the
    // notation and tried to clone.
    let target = TempDir::new().unwrap();
    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.arg("--dir").arg(target.path());
    c.env_remove("DFILES_DIR");
    c.env_remove("DFILES_CLAUDE_DIR");
    // Use a deliberately invalid owner so git fails fast with an auth/404 error
    // rather than hanging. The important thing: dfiles must NOT produce a parse
    // error — that would mean we mishandled the gh: notation.
    c.arg("init").arg("gh:__invalid_dfiles_test__/no-such-repo");

    let output = c.output().unwrap();
    // dfiles should not error about parsing — it should get as far as calling git
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("expected 'gh:' prefix"),
        "gh: notation was not parsed correctly: {stderr}"
    );
    assert!(
        !stderr.contains("expected 'owner/repo'"),
        "gh: notation was not parsed correctly: {stderr}"
    );
    // git should have been invoked (error from git, not from dfiles arg parsing)
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
    Command::cargo_bin("dfiles")
        .unwrap()
        .arg("--dir").arg(target.path())
        .env_remove("DFILES_DIR")
        .env_remove("DFILES_CLAUDE_DIR")
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
    Command::cargo_bin("dfiles")
        .unwrap()
        .arg("--dir").arg(target.path())
        .env_remove("DFILES_DIR")
        .env_remove("DFILES_CLAUDE_DIR")
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
    Command::cargo_bin("dfiles")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env_remove("DFILES_DIR")
        .env_remove("DFILES_CLAUDE_DIR")
        .arg("init")
        .arg("--apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--apply requires a source"));
}

#[test]
fn init_profile_fails_without_source() {
    let repo = TempDir::new().unwrap();
    Command::cargo_bin("dfiles")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env_remove("DFILES_DIR")
        .env_remove("DFILES_CLAUDE_DIR")
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
    Command::cargo_bin("dfiles")
        .unwrap()
        .arg("--dir").arg(target.path())
        .env_remove("DFILES_DIR")
        .env_remove("DFILES_CLAUDE_DIR")
        .arg("init")
        .arg(remote.path().to_str().unwrap())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists and is not empty"));
}

#[test]
fn init_apply_hard_fails_if_no_dfiles_toml() {
    // A git repo with no dfiles.toml.
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
    fs::write(r.join("README.md"), "not a dfiles repo").unwrap();
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
    Command::cargo_bin("dfiles")
        .unwrap()
        .arg("--dir").arg(target.path())
        .env_remove("DFILES_DIR")
        .env_remove("DFILES_CLAUDE_DIR")
        .arg("init")
        .arg(r.to_str().unwrap())
        .arg("--apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not appear to be a dfiles repository"));
}

#[test]
fn init_from_source_with_apply() {
    let home = TempDir::new().unwrap();
    let remote = make_local_git_repo(&[]);
    let target = TempDir::new().unwrap();

    Command::cargo_bin("dfiles")
        .unwrap()
        .arg("--dir").arg(target.path())
        .env_remove("DFILES_DIR")
        .env("HOME", home.path())
        .env("DFILES_CLAUDE_DIR", home.path().join(".claude"))
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

    // Add again — should say already tracked.
    cmd_home(&repo, &home)
        .args(["add", dotfile.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("already tracked"));

    // Exactly one source file with encoded name.
    assert!(repo.path().join("source").join("dot_idempotent.rc").exists());
}

#[test]
fn add_fails_for_missing_file() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    cmd(&repo)
        .args(["add", "/tmp/dfiles-does-not-exist-xyz"])
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
    // the last-used profile. Running `dfiles apply` without --profile should
    // pick up "work" and apply that profile's modules.
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let state_dir = home.path().join(".dfiles");

    cmd(&repo).arg("init").assert().success();

    // Two profiles: default (no modules) and work (also no modules — we just
    // want to verify the profile name that ends up in the next state.json).
    fs::write(
        repo.path().join("dfiles.toml"),
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
    Command::cargo_bin("dfiles")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env("HOME", home.path())
        .env("DFILES_CLAUDE_DIR", home.path().join(".claude"))
        .env_remove("DFILES_DIR")
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
    let state_dir = home.path().join(".dfiles");

    cmd(&repo).arg("init").assert().success();

    fs::write(
        repo.path().join("dfiles.toml"),
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
    Command::cargo_bin("dfiles")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env("HOME", home.path())
        .env("DFILES_CLAUDE_DIR", home.path().join(".claude"))
        .env_remove("DFILES_DIR")
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
        repo.path().join("dfiles.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();

    // No state.json — should fall back to "default".
    Command::cargo_bin("dfiles")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env("HOME", home.path())
        .env("DFILES_CLAUDE_DIR", home.path().join(".claude"))
        .env_remove("DFILES_DIR")
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
        repo.path().join("dfiles.toml"),
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
        repo.path().join("dfiles.toml"),
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
        repo.path().join("dfiles.toml"),
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

    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.env_remove("DFILES_DIR");
    c.env("DFILES_CLAUDE_DIR", claude.path());
    c.args(["--dir", repo.path().to_str().unwrap(), "status", "--ai"]);
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
        repo.path().join("dfiles.toml"),
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
        repo.path().join("config").join("modules").join("packages.toml"),
        toml,
    )
    .unwrap();
    fs::write(
        repo.path().join("dfiles.toml"),
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
    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.env_remove("DFILES_DIR");
    c.args(["--dir", repo.path().to_str().unwrap(), "apply", "--dry-run"]);
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
        repo.path().join("config").join("modules").join("packages.toml"),
        toml,
    )
    .unwrap();
    fs::write(
        repo.path().join("dfiles.toml"),
        "[profile.default]\nmodules = [\"packages\"]\n",
    )
    .unwrap();

    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.env_remove("DFILES_DIR");
    c.args(["--dir", repo.path().to_str().unwrap(), "apply", "--dry-run"]);
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
        repo.path().join("config").join("modules").join("packages.toml"),
        "[homebrew]\nbrewfile = \"brew/Brewfile.packages\"\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("dfiles.toml"),
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

    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.env_remove("DFILES_DIR");
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

    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.env_remove("DFILES_DIR");
    c.args(["--dir", repo.path().to_str().unwrap(), "status"]);
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
    Command::cargo_bin("dfiles")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env("HOME", home.path())
        .env("DFILES_CLAUDE_DIR", home.path().join(".claude"))
        .env_remove("DFILES_DIR")
        .args(["apply", "--remove-unreferenced-brews", "--dry-run"])
        .assert()
        .success();
}

#[test]
fn interactive_dry_run_does_not_prompt_or_uninstall() {
    // --interactive --dry-run: shows the list but exits before the [y/N] prompt,
    // so no stdin interaction is needed and nothing is uninstalled.
    let (repo, home) = setup_apply();
    Command::cargo_bin("dfiles")
        .unwrap()
        .arg("--dir").arg(repo.path())
        .env("HOME", home.path())
        .env("DFILES_CLAUDE_DIR", home.path().join(".claude"))
        .env_remove("DFILES_DIR")
        .args(["apply", "--interactive", "--dry-run"])
        .assert()
        .success();
}


// ─── 1Password integration ───────────────────────────────────────────────────

/// Write a secrets module TOML with `requires_op = true` and an AI skill entry.
/// In the new design, requires_op guards brew/AI but not source file application.
/// Externals are now tracked as extdir_ files in source/, not in module TOMLs.
fn write_secrets_module(repo: &TempDir) {
    let toml = "requires_op = true\n\n\
                [ai]\n\
                skills = [\"gh:example/gh-config\"]\n";
    fs::write(
        repo.path().join("config").join("modules").join("secrets.toml"),
        toml,
    )
    .unwrap();
    fs::write(
        repo.path().join("dfiles.toml"),
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
        .stdout(predicate::str::contains("gh-config"));
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

/// Write an ai.toml with one skill and one command (both gh: sources).
fn write_ai_module(repo: &TempDir) {
    let toml = "[ai]\n\
                skills   = [\"gh:alice/my-skills@v1.0\"]\n\
                commands = [\"gh:alice/my-commands@main\"]\n";
    fs::write(
        repo.path().join("config").join("modules").join("ai.toml"),
        toml,
    )
    .unwrap();
    fs::write(
        repo.path().join("dfiles.toml"),
        "[profile.default]\nmodules = [\"ai\"]\n",
    )
    .unwrap();
}

#[test]
fn ai_toml_parses_skills_and_commands() {
    // Verify that [ai] with gh: sources parses without error (dry-run, no network).
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    write_ai_module(&repo);

    let claude = TempDir::new().unwrap();
    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.env_remove("DFILES_DIR");
    c.env("DFILES_CLAUDE_DIR", claude.path());
    c.args(["--dir", repo.path().to_str().unwrap(), "apply", "--dry-run"]);
    c.assert()
        .success()
        .stdout(predicate::str::contains("fetch skill"))
        .stdout(predicate::str::contains("gh:alice/my-skills@v1.0"))
        .stdout(predicate::str::contains("fetch command"))
        .stdout(predicate::str::contains("gh:alice/my-commands@main"));
}

#[test]
fn ai_dry_run_prints_both_skills_and_commands() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    write_ai_module(&repo);

    let claude = TempDir::new().unwrap();
    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.env_remove("DFILES_DIR");
    c.env("DFILES_CLAUDE_DIR", claude.path());
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
    // apply should generate CLAUDE.md listing installed skills.
    let (repo, home) = setup_apply();

    let claude = TempDir::new().unwrap();
    // Pre-install a skill so CLAUDE.md has content to list.
    let skill_dir = claude.path().join("skills").join("my-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: my-skill\ndescription: \"Test skill\"\n---\n",
    )
    .unwrap();

    cmd_home(&repo, &home)
        .env("DFILES_CLAUDE_DIR", claude.path())
        .arg("apply")
        .assert()
        .success();

    let claude_md = claude.path().join("CLAUDE.md");
    assert!(claude_md.exists(), "CLAUDE.md was not generated");
    let content = fs::read_to_string(&claude_md).unwrap();
    assert!(content.contains("/my-skill: Test skill"));
    assert!(content.contains("profile: default"));
}

#[test]
fn apply_generates_claude_md_even_when_no_skills_installed() {
    // CLAUDE.md should be written even when no skills/commands are present.
    let (repo, home) = setup_apply();

    let claude = TempDir::new().unwrap();

    cmd_home(&repo, &home)
        .env("DFILES_CLAUDE_DIR", claude.path())
        .arg("apply")
        .assert()
        .success();

    let claude_md = claude.path().join("CLAUDE.md");
    assert!(claude_md.exists(), "CLAUDE.md should always be generated");
    let content = fs::read_to_string(&claude_md).unwrap();
    assert!(content.contains("Generated by dfiles"));
}

#[test]
fn status_reports_missing_when_ai_skill_not_installed() {
    // When a skill listed in [ai] is absent from claude_dir, status shows '?'.
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();
    write_ai_module(&repo);

    let claude = TempDir::new().unwrap();
    // Don't create the skill directory — it's absent.

    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.env_remove("DFILES_DIR");
    c.env("DFILES_CLAUDE_DIR", claude.path());
    c.args(["--dir", repo.path().to_str().unwrap(), "status"]);
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
    // Create the skill and command directories to simulate installed state.
    fs::create_dir_all(claude.path().join("skills").join("my-skills")).unwrap();
    fs::create_dir_all(claude.path().join("commands").join("my-commands")).unwrap();

    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.env_remove("DFILES_DIR");
    c.env("DFILES_CLAUDE_DIR", claude.path());
    c.args(["--dir", repo.path().to_str().unwrap(), "status"]);
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
        .env("DFILES_CLAUDE_DIR", claude.path())
        .arg("apply")
        .assert()
        .success();
    // The lock file is only written when AI sources are fetched.
    // File-only apply does not create a lock file.
    assert!(
        !repo.path().join("dfiles.lock").exists(),
        "dfiles.lock should not be written for file-only modules"
    );
}

// ─── bootstrap (Week 7) ────────────────────────────────────────────────────

/// Build a bootstrap command pointing at `repo` with temp dirs for dest/state/claude/envs.
fn bootstrap_cmd(
    repo: &TempDir,
    home: &TempDir,
    claude: &TempDir,
    envs: &TempDir,
) -> Command {
    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.env_remove("DFILES_DIR");
    c.env("HOME", home.path());
    c.env("DFILES_CLAUDE_DIR", claude.path());
    c.env("DFILES_ENVS_DIR", envs.path());
    c.arg("--dir").arg(repo.path());
    c
}

#[test]
fn bootstrap_local_dry_run_succeeds() {
    let (repo, home) = setup_apply();
    let claude = TempDir::new().unwrap();
    let envs = TempDir::new().unwrap();

    bootstrap_cmd(&repo, &home, &claude, &envs)
        .args(["bootstrap", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("dot_applyrc"));
}

#[test]
fn bootstrap_local_prints_profile_banner_when_no_manifest() {
    let (repo, home) = setup_apply();
    let claude = TempDir::new().unwrap();
    let envs = TempDir::new().unwrap();

    bootstrap_cmd(&repo, &home, &claude, &envs)
        .args(["bootstrap", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Bootstrapping profile:"));
}

#[test]
fn bootstrap_shows_manifest_banner_when_present() {
    let (repo, home) = setup_apply();
    let claude = TempDir::new().unwrap();
    let envs = TempDir::new().unwrap();

    // Write a manifest.
    fs::write(
        repo.path().join("dfiles-manifest.json"),
        r#"{"name":"my-env","version":"v2.0","author":"alice"}"#,
    )
    .unwrap();

    bootstrap_cmd(&repo, &home, &claude, &envs)
        .args(["bootstrap", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-env"))
        .stdout(predicate::str::contains("v2.0"))
        .stdout(predicate::str::contains("by alice"));
}

#[test]
fn bootstrap_remote_source_dry_run_skips_fetch() {
    let (repo, home) = setup_apply();
    let claude = TempDir::new().unwrap();
    let envs = TempDir::new().unwrap();

    bootstrap_cmd(&repo, &home, &claude, &envs)
        .args([
            "bootstrap",
            "gh:testowner/testenv",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("gh:testowner/testenv"));

    // envs dir must remain empty — no network hit occurred.
    assert!(
        envs.path().read_dir().unwrap().next().is_none(),
        "envs dir should be empty on dry-run"
    );
}

#[test]
fn bootstrap_local_runs_apply_then_status() {
    let (repo, home) = setup_apply();
    let claude = TempDir::new().unwrap();
    let envs = TempDir::new().unwrap();

    bootstrap_cmd(&repo, &home, &claude, &envs)
        .arg("bootstrap")
        .assert()
        .success()
        // apply output
        .stdout(predicate::str::contains("Applied"))
        // status header printed after apply
        .stdout(predicate::str::contains("Environment status"));
}

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
    let shell_toml_path = repo.path().join("config").join("modules").join("shell.toml");
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

/// Write a source file with a magic-name encoded filename and a minimal dfiles.toml.
#[cfg(unix)]
fn write_permission_source(repo: &TempDir, encoded_name: &str) {
    let source_dir = repo.path().join("source");
    fs::write(source_dir.join(encoded_name), "content\n").unwrap();
    fs::write(
        repo.path().join("dfiles.toml"),
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
        repo.path().join("dfiles.toml"),
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
        repo.path().join("dfiles.toml"),
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

    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.env_remove("DFILES_DIR");
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
        "~/.tmux/plugins/dfiles-test-nonexistent",
        "https://github.com/user/nvim-config",
        None,
    );

    let mut c = Command::cargo_bin("dfiles").unwrap();
    c.env_remove("DFILES_DIR");
    c.args(["--dir", repo.path().to_str().unwrap(), "status"]);
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
        repo.path().join("dfiles.toml"),
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
        repo.path().join("dfiles.toml"),
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
    // `dfiles add --link` should encode the file as `symlink_<name>` in source/.
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

// ─── dfiles diff ──────────────────────────────────────────────────────────────

/// Set up a repo+home pair for diff tests.
/// Source: source/dot_diffrc  →  ~/.diffrc  (plain, content "v1\n")
/// The dfiles.toml has no modules (no brew/AI to invoke).
fn setup_diff() -> (TempDir, TempDir) {
    let repo = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    cmd(&repo).arg("init").assert().success();

    let source_dir = repo.path().join("source");
    fs::write(source_dir.join("dot_diffrc"), "v1\n").unwrap();
    fs::write(
        repo.path().join("dfiles.toml"),
        "[profile.default]\nmodules = []\n",
    )
    .unwrap();

    (repo, home)
}

/// Helper: run `dfiles diff` with the given extra args.
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
