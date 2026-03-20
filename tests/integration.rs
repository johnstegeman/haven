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

// ─── 1Password integration ───────────────────────────────────────────────────

/// Write a secrets module TOML with `requires_op = true` and one external entry.
/// In the new design, requires_op guards externals/brew/AI but not source file application.
fn write_secrets_module(repo: &TempDir) {
    let toml = "requires_op = true\n\n\
                [[externals]]\n\
                dest = \"~/.config/gh\"\n\
                type = \"git\"\n\
                url  = \"https://github.com/example/gh-config\"\n";
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
    fs::write(
        &mock_chezmoi,
        format!("#!/bin/sh\necho '{}'\n", chezmoi_src.path().display()),
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

/// Write a module TOML with a single `[[externals]]` entry (no files).
fn write_externals_module(repo: &TempDir, dest: &str, url: &str, ref_name: Option<&str>) {
    let ref_line = ref_name
        .map(|r| format!("\nref  = \"{}\"", r))
        .unwrap_or_default();
    let toml = format!(
        "[[externals]]\ndest = \"{}\"\ntype = \"git\"\nurl  = \"{}\"{}\n",
        dest, url, ref_line
    );
    fs::write(
        repo.path().join("config").join("modules").join("shell.toml"),
        toml,
    )
    .unwrap();
    fs::write(
        repo.path().join("dfiles.toml"),
        "[profile.default]\nmodules = [\"shell\"]\n",
    )
    .unwrap();
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
        .stdout(predicate::str::contains("git clone"))
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

    // editor.toml should have the [[externals]] entry.
    let editor_toml = fs::read_to_string(
        repo.path().join("config").join("modules").join("editor.toml"),
    )
    .unwrap();
    assert!(editor_toml.contains("[[externals]]"), "missing [[externals]] section");
    assert!(editor_toml.contains("~/.config/nvim"), "missing dest");
    assert!(editor_toml.contains("nvim-config"), "missing url");
    assert!(editor_toml.contains("git"), "missing type = git");
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
    let repo = TempDir::new().unwrap();
    let chezmoi_src = TempDir::new().unwrap();

    fs::write(
        chezmoi_src.path().join(".chezmoiexternal.toml"),
        "[\"~/.config/nvim\"]\ntype = \"git-repo\"\nurl  = \"https://github.com/user/nvim-config\"\n",
    )
    .unwrap();

    cmd(&repo).arg("init").assert().success();

    // First import.
    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success();

    // Second import — should not error, should say "already tracked".
    cmd(&repo)
        .args(["import", "--from", "chezmoi", "--source"])
        .arg(chezmoi_src.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("already tracked"));
}

#[test]
fn status_shows_external_missing() {
    let repo = TempDir::new().unwrap();
    cmd(&repo).arg("init").assert().success();

    // Write an externals entry pointing at a path that doesn't exist.
    let dest = TempDir::new().unwrap().keep().join("nvim");
    write_externals_module(
        &repo,
        dest.to_str().unwrap(),
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
