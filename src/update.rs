//! Core update check + download + verify module.
//!
//! Handles querying GitHub Releases for newer versions, downloading
//! tar.gz artifacts, verifying SHA256 checksums, and extracting binaries.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use semver::Version;
use serde::Deserialize;

/// Metadata for an available update.
#[derive(Debug, Clone)]
pub struct AvailableUpdate {
    pub current: Version,
    pub latest: Version,
    pub release_url: String,
    pub release_notes: Option<String>,
}

/// How Kaelo was installed — determines self-update capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallMethod {
    /// ~/.local/bin or similar — can self-update
    Github,
    /// /opt/homebrew, /usr/local/Cellar, linuxbrew — cannot self-update
    Homebrew,
    /// ~/.cargo/bin — cannot self-update
    Cargo,
    /// Can't determine — try anyway
    Unknown,
}

/// GitHub release JSON (subset of fields we care about).
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
}

/// Get current version from `CARGO_PKG_VERSION`.
pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION is not valid semver")
}

/// Detect how Kaelo was installed by inspecting the current binary path.
pub fn detect_install_method() -> InstallMethod {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return InstallMethod::Unknown,
    };
    let path = exe.to_string_lossy();

    if path.contains("/.cargo/bin/") {
        InstallMethod::Cargo
    } else if path.contains("/Cellar/")
        || path.contains("/homebrew/")
        || path.contains("/linuxbrew/")
    {
        InstallMethod::Homebrew
    } else {
        // ~/.local/bin, /usr/local/bin, etc — treat as GitHub install
        InstallMethod::Github
    }
}

/// Get platform target triple (e.g., "aarch64-apple-darwin").
pub fn platform_target() -> Result<String> {
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        anyhow::bail!("Unsupported architecture")
    };

    let os = if cfg!(target_os = "macos") {
        "apple-darwin"
    } else if cfg!(target_os = "linux") {
        "unknown-linux-gnu"
    } else {
        anyhow::bail!("Unsupported OS")
    };

    Ok(format!("{}-{}", arch, os))
}

/// Check if we should query GitHub (24 h cache).
pub fn should_check(cache_path: &Path) -> bool {
    if !cache_path.exists() {
        return true;
    }
    let metadata = match std::fs::metadata(cache_path) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let modified = match metadata.modified() {
        Ok(t) => t,
        Err(_) => return true,
    };
    let elapsed = modified.elapsed().unwrap_or(Duration::MAX);
    elapsed > Duration::from_secs(24 * 60 * 60)
}

/// Write timestamp to check cache file.
pub fn write_check_cache(cache_path: &Path) -> Result<()> {
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(cache_path, "")?;
    Ok(())
}

/// Check GitHub releases for a newer version.
///
/// Returns `Ok(None)` when up-to-date or on any network/parse error (graceful
/// degradation).
pub async fn check_for_update() -> Result<Option<AvailableUpdate>> {
    let current = current_version();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent(format!("Kaelo/{}", current))
        .build()
        .map_err(|e| {
            tracing::debug!("Failed to build HTTP client: {e}");
            e
        })?;

    let resp = match client
        .get("https://api.github.com/repos/HachemiH/kaelo/releases/latest")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("Update check failed: {e}");
            return Ok(None);
        }
    };

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        tracing::debug!("GitHub API rate limited");
        return Ok(None);
    }

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!("Failed to read response body: {e}");
            return Ok(None);
        }
    };
    let release: GitHubRelease = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("Failed to parse release JSON: {e}");
            return Ok(None);
        }
    };

    let tag = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    let latest: Version = match Version::parse(tag) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("Failed to parse tag as semver: {e}");
            return Ok(None);
        }
    };

    if latest > current {
        Ok(Some(AvailableUpdate {
            current,
            latest,
            release_url: release.html_url,
            release_notes: release.body,
        }))
    } else {
        Ok(None)
    }
}

/// Download and verify the update for the given target triple.
///
/// Downloads `tar.gz` + `.sha256`, verifies checksum, extracts the binary.
/// Returns the path to the extracted binary inside the temp dir.
pub async fn download_update(target: &str, tag: &str) -> Result<PathBuf> {
    let archive_name = format!("kaelo-{tag}-{target}.tar.gz");
    let checksum_name = format!("{archive_name}.sha256");

    let base_url = format!("https://github.com/HachemiH/kaelo/releases/download/{tag}");
    let archive_url = format!("{base_url}/{archive_name}");
    let checksum_url = format!("{base_url}/{checksum_name}");

    let tmp_path = std::env::temp_dir().join(format!(
        "kaelo-update-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::create_dir_all(&tmp_path)?;

    let archive_path = tmp_path.join(&archive_name);
    let checksum_path = tmp_path.join(&checksum_name);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(format!("Kaelo/{}", current_version()))
        .build()?;

    let archive_bytes = client.get(&archive_url).send().await?.bytes().await?;
    std::fs::write(&archive_path, &archive_bytes)?;

    let checksum_text = client.get(&checksum_url).send().await?.text().await?;
    std::fs::write(&checksum_path, &checksum_text)?;

    let expected_hash = checksum_text
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Empty checksum file"))?;

    let output = std::process::Command::new("shasum")
        .args(["-a", "256"])
        .arg(&archive_path)
        .output()?;

    if !output.status.success() {
        anyhow::bail!("shasum command failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let actual_hash = stdout
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Empty shasum output"))?;

    if expected_hash != actual_hash {
        anyhow::bail!("Checksum mismatch!\n  expected: {expected_hash}\n  actual:   {actual_hash}");
    }

    let status = std::process::Command::new("tar")
        .args(["xzf"])
        .arg(&archive_path)
        .args(["-C"])
        .arg(&tmp_path)
        .arg("kaelo")
        .status()?;

    if !status.success() {
        anyhow::bail!("tar extraction failed");
    }

    let binary_path = tmp_path.join("kaelo");
    if !binary_path.exists() {
        anyhow::bail!("Extracted binary not found at {}", binary_path.display());
    }

    Ok(binary_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn test_current_version() {
        let v = current_version();
        // CARGO_PKG_VERSION should always parse as valid semver.
        assert!(v.major >= 1, "Expected version >= 1.0.0, got {v}");
    }

    #[test]
    #[cfg(not(windows))]
    fn test_platform_target() {
        let target = platform_target().expect("platform_target should succeed on supported OS");
        // Must match one of the 4 supported triples (mirrors release.yml matrix).
        let supported = [
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
        ];
        assert!(
            supported.contains(&target.as_str()),
            "Unsupported target: {target}"
        );
    }

    /// Documents that `kaelo update` intentionally does not wire auto-update
    /// for Windows yet — there are no Windows artifacts in the release matrix
    /// (`.github/workflows/release.yml`). The desired product behaviour on
    /// Windows is a clean `Err` from `platform_target`, not a 404 mid-download.
    #[test]
    #[cfg(windows)]
    fn test_platform_target_unsupported_on_windows() {
        assert!(
            platform_target().is_err(),
            "platform_target should bail on Windows until Windows release artifacts exist"
        );
    }

    #[test]
    fn test_should_check_no_file() {
        let dir = std::env::temp_dir().join("kaelo_test_no_file");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(".update-check");
        assert!(should_check(&path), "Should check when file doesn't exist");
    }

    #[test]
    fn test_should_check_recent() {
        let dir = std::env::temp_dir().join("kaelo_test_recent");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".update-check");
        write_check_cache(&path).unwrap();
        assert!(!should_check(&path), "Should NOT check when cache is fresh");
    }

    #[test]
    fn test_should_check_expired() {
        let dir = std::env::temp_dir().join("kaelo_test_expired");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".update-check");

        // Write file, then set mtime to 25 hours ago.
        std::fs::write(&path, "").unwrap();
        let past = SystemTime::now() - Duration::from_secs(25 * 60 * 60);
        filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(past)).unwrap();

        assert!(
            should_check(&path),
            "Should check when cache is older than 24h"
        );
    }

    #[test]
    fn test_checksum_parsing() {
        let raw = "abc123  filename.tar.gz\n";
        let hash = raw.split_whitespace().next().unwrap();
        assert_eq!(hash, "abc123");
    }

    #[test]
    fn test_checksum_parsing_multiline() {
        let raw = "deadbeef  kaelo-v0.2.0-aarch64-apple-darwin.tar.gz\n";
        let hash = raw.split_whitespace().next().unwrap();
        assert_eq!(hash, "deadbeef");
    }

    #[test]
    fn test_install_method_detection() {
        // Just verify it doesn't panic and returns a valid variant.
        let method = detect_install_method();
        match method {
            InstallMethod::Github
            | InstallMethod::Homebrew
            | InstallMethod::Cargo
            | InstallMethod::Unknown => {} // ok
        }
    }

    #[test]
    fn test_write_check_cache_creates_parent() {
        let dir = std::env::temp_dir().join("kaelo_test_write_cache_nested");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sub").join("dir").join(".update-check");
        write_check_cache(&path).expect("Should create parent dirs");
        assert!(path.exists());
    }
}
