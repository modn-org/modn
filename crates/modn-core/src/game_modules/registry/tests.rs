use super::super::loader;
use super::layout::compare_versions;
use std::cmp::Ordering;

#[test]
fn unsafe_version_strings_are_rejected() {
    for version in [".", "..", ".hidden", "", "1/2", "1\\2"] {
        assert!(
            !loader::valid_version(version),
            "accepted unsafe version {version:?}"
        );
    }
    for version in ["1", "1.0.0", "2.0.0-beta.1", "2.0.0+build.5", "nightly"] {
        assert!(loader::valid_version(version), "rejected {version:?}");
    }
}

#[test]
fn versions_order_by_semver_when_parseable_and_bytes_otherwise() {
    assert_eq!(compare_versions("1.10.0", "1.9.0"), Ordering::Greater);
    assert_eq!(compare_versions("2.0.0", "2.0.0"), Ordering::Equal);
    assert_eq!(compare_versions("2.0.0-alpha", "2.0.0"), Ordering::Less);
    assert_eq!(compare_versions("nightly", "stable"), Ordering::Less);
    assert_eq!(compare_versions("10", "2"), Ordering::Less);
}
