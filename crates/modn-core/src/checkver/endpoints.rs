//! Endpoint construction and the test-only origin overrides.

use anyhow::{bail, Result};

const GITHUB_API: &str = "https://api.github.com";
const GITHUB_HOST: &str = "https://github.com";
const NEXUS_API: &str = "https://api.nexusmods.com";

const GITHUB_API_TEST_VARIABLE: &str = "MODN_TEST_CHECKVER_GITHUB_API";
const GITHUB_HOST_TEST_VARIABLE: &str = "MODN_TEST_CHECKVER_GITHUB_HOST";
const NEXUS_API_TEST_VARIABLE: &str = "MODN_TEST_CHECKVER_NEXUS_API";

fn override_base(variable: &str) -> Option<String> {
    #[cfg(feature = "test-checkver-endpoints")]
    {
        std::env::var(variable)
            .ok()
            .map(|value| value.trim_end_matches('/').to_owned())
            .filter(|value| !value.is_empty())
    }
    #[cfg(not(feature = "test-checkver-endpoints"))]
    {
        let _ = variable;
        None
    }
}

/// Whether a test binary redirected a checkver endpoint.
///
/// Release builds return `false` and never read an override variable, so the
/// HTTP client keeps HTTPS-only and the Git client refuses the `file`
/// transport.
#[must_use]
pub fn test_endpoints_enabled() -> bool {
    #[cfg(feature = "test-checkver-endpoints")]
    {
        override_base(GITHUB_API_TEST_VARIABLE).is_some()
            || override_base(GITHUB_HOST_TEST_VARIABLE).is_some()
            || override_base(NEXUS_API_TEST_VARIABLE).is_some()
    }
    #[cfg(not(feature = "test-checkver-endpoints"))]
    {
        false
    }
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_repository(repository: &str) -> Result<()> {
    let mut parts = repository.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        bail!("invalid GitHub repository; expected owner/repo");
    };
    if !valid_segment(owner) || !valid_segment(name) {
        bail!("invalid GitHub repository; expected owner/repo");
    }
    Ok(())
}

/// The GitHub releases API URL of one repository.
///
/// # Errors
///
/// Returns an error when `repository` is not exactly `owner/repo`.
pub fn github_releases_url(repository: &str) -> Result<String> {
    validate_repository(repository)?;
    let base = override_base(GITHUB_API_TEST_VARIABLE).unwrap_or_else(|| GITHUB_API.to_owned());
    Ok(format!("{base}/repos/{repository}/releases?per_page=30"))
}

/// The Git remote URL of one GitHub repository.
///
/// A test binary may replace the GitHub host with a local directory.
///
/// # Errors
///
/// Returns an error when `repository` is not exactly `owner/repo`.
pub fn github_remote_url(repository: &str) -> Result<String> {
    validate_repository(repository)?;
    let base = override_base(GITHUB_HOST_TEST_VARIABLE).unwrap_or_else(|| GITHUB_HOST.to_owned());
    Ok(format!("{base}/{repository}.git"))
}

/// The download URL of one GitHub release asset.
///
/// `modn catalog checkver` compares versions only. A later release update
/// uses this URL after it has chosen an asset.
///
/// # Errors
///
/// Returns an error when `repository` is not exactly `owner/repo`, or when
/// `tag` or `asset` is empty or holds a path separator.
pub fn github_asset_url(repository: &str, tag: &str, asset: &str) -> Result<String> {
    validate_repository(repository)?;
    for (label, value) in [("tag", tag), ("asset", asset)] {
        if value.is_empty()
            || value
                .chars()
                .any(|character| character.is_control() || matches!(character, '/' | '\\'))
        {
            bail!("invalid GitHub release {label}");
        }
    }
    let base = override_base(GITHUB_HOST_TEST_VARIABLE).unwrap_or_else(|| GITHUB_HOST.to_owned());
    Ok(format!(
        "{base}/{repository}/releases/download/{tag}/{asset}"
    ))
}

/// The Nexus Mods file-list API URL of one mod.
#[must_use]
pub fn nexus_files_url(game_domain: &str, mod_id: u64) -> String {
    let base = override_base(NEXUS_API_TEST_VARIABLE).unwrap_or_else(|| NEXUS_API.to_owned());
    format!("{base}/v1/games/{game_domain}/mods/{mod_id}/files.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkver_endpoints_default_to_the_public_https_origins() {
        assert_eq!(
            github_releases_url("owner/repo").expect("URL"),
            "https://api.github.com/repos/owner/repo/releases?per_page=30"
        );
        assert_eq!(
            github_remote_url("owner/repo").expect("URL"),
            "https://github.com/owner/repo.git"
        );
        assert_eq!(
            github_asset_url("owner/repo", "v1", "mod.zip").expect("URL"),
            "https://github.com/owner/repo/releases/download/v1/mod.zip"
        );
        assert_eq!(
            nexus_files_url("cyberpunk2077", 6945),
            "https://api.nexusmods.com/v1/games/cyberpunk2077/mods/6945/files.json"
        );
    }

    #[test]
    fn checkver_endpoints_refuse_a_malformed_repository() {
        for repository in ["", "owner", "owner/repo/extra", "owner/..", "owner/re po"] {
            assert!(
                github_releases_url(repository).is_err(),
                "{repository:?} was accepted"
            );
            assert!(
                github_remote_url(repository).is_err(),
                "{repository:?} was accepted"
            );
        }
    }

    #[cfg(not(feature = "test-checkver-endpoints"))]
    #[test]
    fn checkver_default_build_ignores_endpoint_overrides() {
        std::env::set_var("MODN_TEST_CHECKVER_GITHUB_API", "http://127.0.0.1:1");
        std::env::set_var("MODN_TEST_CHECKVER_GITHUB_HOST", "file:///tmp/repos");
        std::env::set_var("MODN_TEST_CHECKVER_NEXUS_API", "http://127.0.0.1:1");
        assert!(!test_endpoints_enabled());
        assert_eq!(
            github_releases_url("owner/repo").expect("URL"),
            "https://api.github.com/repos/owner/repo/releases?per_page=30"
        );
        assert_eq!(
            github_remote_url("owner/repo").expect("URL"),
            "https://github.com/owner/repo.git"
        );
        assert_eq!(
            nexus_files_url("cyberpunk2077", 6945),
            "https://api.nexusmods.com/v1/games/cyberpunk2077/mods/6945/files.json"
        );
        std::env::remove_var("MODN_TEST_CHECKVER_GITHUB_API");
        std::env::remove_var("MODN_TEST_CHECKVER_GITHUB_HOST");
        std::env::remove_var("MODN_TEST_CHECKVER_NEXUS_API");
    }
}
