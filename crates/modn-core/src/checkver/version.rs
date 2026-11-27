//! Version comparison for a GitHub release check.

use semver::Version;

/// Remove exactly one leading `v` from a release tag.
#[must_use]
pub fn github_tag_version(tag: &str) -> String {
    tag.strip_prefix('v').unwrap_or(tag).to_owned()
}

/// Whether the upstream version is newer than the manifest version.
///
/// When both strings parse as semantic versions, only a greater upstream
/// version is an update. Otherwise any difference is an update. Equality is
/// never an update.
#[must_use]
pub fn upstream_is_newer(current: &str, upstream: &str) -> bool {
    match (Version::parse(current), Version::parse(upstream)) {
        (Ok(current), Ok(upstream)) => upstream > current,
        _ => current != upstream,
    }
}

#[cfg(test)]
mod tests {
    use super::{github_tag_version, upstream_is_newer};

    #[test]
    fn checkver_tag_to_version_removes_one_leading_v() {
        assert_eq!(github_tag_version("v1.2.3"), "1.2.3");
        assert_eq!(github_tag_version("1.2.3"), "1.2.3");
        assert_eq!(github_tag_version("vv1.2.3"), "v1.2.3");
        assert_eq!(github_tag_version("version-1"), "ersion-1");
        assert_eq!(github_tag_version("V1.2.3"), "V1.2.3");
    }

    #[test]
    fn checkver_version_comparison_uses_semver_when_possible() {
        assert!(upstream_is_newer("1.2.3", "1.3.0"));
        assert!(upstream_is_newer("1.2.3", "2.0.0"));
        assert!(!upstream_is_newer("1.2.3", "1.2.2"));
        assert!(!upstream_is_newer("1.2.3", "1.2.3"));
        assert!(upstream_is_newer("1.0", "1.0.1"));
        assert!(upstream_is_newer("1.0.0", "1.0"));
    }

    #[test]
    fn checkver_version_comparison_falls_back_to_difference() {
        assert!(upstream_is_newer("2024.1", "2024.2"));
        assert!(!upstream_is_newer("2024.1", "2024.1"));
        assert!(upstream_is_newer("release-a", "release-b"));
        assert!(upstream_is_newer("1.0", "nightly"));
    }
}
