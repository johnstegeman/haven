/// Capture the short git commit hash at compile time and expose it as
/// `DFILES_GIT_COMMIT` for use in telemetry.
///
/// Falls back to "unknown" if git is unavailable or the repo has no commits
/// (e.g. a clean source tarball).
fn main() {
    let hash = git_short_hash().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=DFILES_GIT_COMMIT={}", hash);

    // Re-run whenever HEAD changes (new commit, branch switch).
    println!("cargo:rerun-if-changed=.git/HEAD");
    // Also re-run when a packed ref changes (e.g. after `git fetch`).
    println!("cargo:rerun-if-changed=.git/packed-refs");
}

fn git_short_hash() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        let s = String::from_utf8(output.stdout).ok()?;
        Some(s.trim().to_string())
    } else {
        None
    }
}
