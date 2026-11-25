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
