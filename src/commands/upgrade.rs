/// Self-update: download the latest haven binary from GitHub releases.
///
/// Workflow:
///   1. Query the GitHub releases API for the latest tag.
///   2. Compare to the current binary's version (`CARGO_PKG_VERSION`).
///   3. Download the platform-specific tarball + SHA256SUMS.
///   4. Verify the checksum.
///   5. Extract the `haven` binary, write to a sibling temp file, and atomically
///      rename it over the current executable.
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;

pub struct UpgradeOptions {
    /// Print whether an update is available and exit without installing.
    /// Exits 0 when up-to-date, 1 when an update is available.
    pub check_only: bool,
    /// Upgrade even if the current version equals the latest.
    pub force: bool,
}

const REPO: &str = "johnstegeman/haven";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Return the target triple for the current binary at compile time.
///
/// Must match the filenames produced by the release workflow (e.g.
/// `haven-v0.3.0-aarch64-apple-darwin.tar.gz`).
fn current_target() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-musl"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-musl"
    } else if cfg!(all(target_os = "linux", target_arch = "arm")) {
        "armv7-unknown-linux-musleabihf"
    } else if cfg!(all(target_os = "linux", target_arch = "x86")) {
        "i686-unknown-linux-musl"
    } else {
        "unknown"
    }
}

/// Fetch the latest release tag name from the GitHub releases API.
fn fetch_latest_version() -> Result<String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);
    let resp = ureq::get(&url)
        .set(
            "User-Agent",
            "haven/0.1 (+https://github.com/johnstegeman/haven)",
        )
        .set("Accept", "application/vnd.github+json")
        .call()
        .with_context(|| format!("Failed to fetch latest release info from {}", url))?;

    let mut body = Vec::new();
    resp.into_reader()
        .read_to_end(&mut body)
        .context("Failed to read GitHub API response")?;

    let parsed: serde_json::Value =
        serde_json::from_slice(&body).context("Failed to parse GitHub API response as JSON")?;

    parsed["tag_name"]
        .as_str()
        .map(|s| s.trim_start_matches('v').to_string())
        .context("GitHub API response missing 'tag_name' field")
}

/// Download raw bytes from `url`.
fn download_bytes(url: &str) -> Result<Vec<u8>> {
    let resp = ureq::get(url)
        .set(
            "User-Agent",
            "haven/0.1 (+https://github.com/johnstegeman/haven)",
        )
        .call()
        .with_context(|| format!("HTTP download failed: {}", url))?;

    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .context("Failed to read response body")?;

    Ok(bytes)
}

/// Verify that `bytes` matches the SHA256 entry for `filename` in a `SHA256SUMS` file.
///
/// `shasums` is the raw text content of the `SHA256SUMS` file (one
/// `"<hex>  <filename>"` pair per line, standard sha256sum format).
fn verify_sha256(bytes: &[u8], shasums: &str, filename: &str) -> Result<()> {
    // Compute the actual SHA256 of the downloaded bytes.
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();

    // Find the matching line in SHA256SUMS.
    for line in shasums.lines() {
        // Lines look like: "abc123...  haven-v0.3.0-aarch64-apple-darwin.tar.gz"
        // (two spaces between hash and filename, per sha256sum convention)
        let Some((expected_hex, name)) = line.split_once("  ") else {
            continue;
        };
        if name.trim() == filename {
            if actual != expected_hex.trim() {
                anyhow::bail!(
                    "SHA256 mismatch for {}:\n  expected: {}\n  actual:   {}",
                    filename,
                    expected_hex.trim(),
                    actual
                );
            }
            return Ok(());
        }
    }

    anyhow::bail!(
        "SHA256SUMS file does not contain an entry for '{}'. \
         This may indicate a release packaging issue.",
        filename
    )
}

/// Extract the `haven` binary from a `.tar.gz` archive and write it to `dest`.
///
/// The binary may be at the archive root or one level deep (depending on how
/// the release tarball is structured). The file is made executable on Unix.
fn extract_binary(tarball: &[u8], dest: &std::path::Path) -> Result<()> {
    let gz = flate2::read::GzDecoder::new(tarball);
    let mut archive = tar::Archive::new(gz);

    for entry in archive.entries().context("Failed to read tar entries")? {
        let mut entry = entry.context("Invalid tar entry")?;
        let path = entry.path().context("Invalid entry path")?.into_owned();

        // Reject symlinks and hardlinks — same protection as `extract_tarball` in github.rs.
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            continue;
        }

        let is_binary = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n == "haven")
            .unwrap_or(false);

        if is_binary && entry_type.is_file() {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            entry
                .unpack(dest)
                .with_context(|| format!("Failed to write binary to {}", dest.display()))?;

            // Ensure the new binary is executable.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(dest)
                    .with_context(|| {
                        format!("Could not stat newly written binary at {}", dest.display())
                    })?
                    .permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(dest, perms)
                    .context("Could not make new binary executable")?;
            }

            return Ok(());
        }
    }

    anyhow::bail!(
        "Could not find a 'haven' binary inside the release archive. \
         The archive may be malformed or for the wrong platform."
    )
}

pub fn run(opts: &UpgradeOptions) -> Result<()> {
    let current = CURRENT_VERSION;
    let target = current_target();

    if target == "unknown" {
        anyhow::bail!(
            "Cannot self-upgrade: unrecognised target platform.\n\
             Download the latest binary from https://github.com/{}/releases",
            REPO
        );
    }

    println!("Checking for updates...");

    let latest = fetch_latest_version().context("Failed to check for the latest version")?;

    // --check: report availability and exit without installing.
    if opts.check_only {
        if current == latest {
            println!("haven v{} is up to date.", current);
        } else {
            println!("Update available: v{} → v{}", current, latest);
            std::process::exit(1);
        }
        return Ok(());
    }

    if current == latest && !opts.force {
        println!("haven v{} is up to date.", current);
        return Ok(());
    }

    println!("Upgrading haven v{} → v{}...", current, latest);

    let archive_name = format!("haven-v{}-{}.tar.gz", latest, target);
    let base_url = format!(
        "https://github.com/{}/releases/download/v{}",
        REPO, latest
    );
    let archive_url = format!("{}/{}", base_url, archive_name);
    let shasums_url = format!("{}/haven-v{}-SHA256SUMS", base_url, latest);

    println!("Downloading {}...", archive_name);
    let archive_bytes = download_bytes(&archive_url)
        .with_context(|| format!("Failed to download {}", archive_url))?;

    println!("Verifying checksum...");
    let shasums_bytes = download_bytes(&shasums_url)
        .with_context(|| format!("Failed to download SHA256SUMS from {}", shasums_url))?;
    let shasums = String::from_utf8(shasums_bytes).context("SHA256SUMS file is not valid UTF-8")?;
    verify_sha256(&archive_bytes, &shasums, &archive_name)
        .context("Checksum verification failed")?;

    let current_exe =
        std::env::current_exe().context("Could not determine the current executable path")?;

    // Write to a sibling temp file, then atomically rename over the live binary.
    // Using rename() guarantees the swap is atomic on POSIX systems — readers never
    // see a partial binary.
    let temp_path = {
        let mut p = current_exe.clone();
        p.set_extension("new");
        p
    };

    extract_binary(&archive_bytes, &temp_path)?;

    std::fs::rename(&temp_path, &current_exe).with_context(|| {
        format!(
            "Failed to replace {} with the new binary. \
             Try running with elevated permissions or install manually.",
            current_exe.display()
        )
    })?;

    println!("haven upgraded to v{}.", latest);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_sha256_passes_for_matching_hash() {
        let data = b"hello haven";
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hex: String = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect();
        let shasums = format!("{}  haven-test.tar.gz\n", hex);
        assert!(verify_sha256(data, &shasums, "haven-test.tar.gz").is_ok());
    }

    #[test]
    fn verify_sha256_rejects_wrong_hash() {
        let data = b"hello haven";
        let shasums = "0000000000000000000000000000000000000000000000000000000000000000  haven-test.tar.gz\n";
        assert!(verify_sha256(data, shasums, "haven-test.tar.gz").is_err());
    }

    #[test]
    fn verify_sha256_errors_when_filename_missing() {
        let data = b"hello haven";
        let shasums = "abc123  other-file.tar.gz\n";
        let err = verify_sha256(data, shasums, "haven-test.tar.gz").unwrap_err();
        assert!(err.to_string().contains("does not contain an entry for"));
    }

    #[test]
    fn current_target_is_not_unknown_on_supported_platforms() {
        // This test only runs on the CI platforms where we publish binaries.
        // On unsupported platforms it would return "unknown" and that's fine to skip.
        let t = current_target();
        if std::env::var("CI").is_ok() {
            assert_ne!(t, "unknown", "CI should always run on a supported platform");
        }
        // Non-CI: just ensure the function doesn't panic.
        let _ = t;
    }
}
