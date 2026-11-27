//! Build-time gated hooks for tests that drive the main bucket release selection.
//!
//! A release build compiles the environment lookup out, so no variable can
//! redirect a released binary. The `test-main-bucket-releases` feature is
//! enabled only by this repository's test dependencies.

/// The release-list location that a test binary may override.
///
/// Release builds return `None`, so `modn bucket update` reads its default
/// location.
#[cfg(feature = "test-main-bucket-releases")]
#[must_use]
pub fn main_release_list_override() -> Option<String> {
    std::env::var("MODN_TEST_MAIN_BUCKET_RELEASES")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The release-list location that a test binary may override.
///
/// Release builds return `None`, so `modn bucket update` reads its default
/// location.
#[cfg(not(feature = "test-main-bucket-releases"))]
#[must_use]
pub fn main_release_list_override() -> Option<String> {
    None
}

/// Redirect one HTTPS source of a known bucket to a local fixture.
///
/// Release builds return `None` and never read the variables, so a listed
/// bucket keeps an HTTPS-only source. The `test-known-source-redirect` feature
/// is enabled only by this repository's test dependencies.
#[cfg(feature = "test-known-source-redirect")]
#[must_use]
pub fn known_source_redirect(source: &str) -> Option<String> {
    let listed = std::env::var("MODN_TEST_KNOWN_BUCKET_SOURCE").ok()?;
    let target = std::env::var("MODN_TEST_KNOWN_BUCKET_TARGET").ok()?;
    let listed = listed.trim();
    let target = target.trim();
    (!listed.is_empty() && !target.is_empty() && source == listed).then(|| target.to_owned())
}

/// Redirect one HTTPS source of a known bucket to a local fixture.
///
/// Release builds return `None` and never read the variables.
#[cfg(not(feature = "test-known-source-redirect"))]
#[must_use]
pub fn known_source_redirect(_source: &str) -> Option<String> {
    None
}
