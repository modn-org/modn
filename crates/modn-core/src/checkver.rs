//! Upstream release checks for catalog authoring tools.
//!
//! This module holds the pure and deterministic parts of
//! `modn catalog checkver`: URL construction, glob matching, release choice,
//! tag-to-version mapping and version comparison. It performs no HTTP request
//! itself. User commands never call into this module.
//!
//! Test binaries may redirect the GitHub API, the GitHub host and the Nexus
//! API through the `test-checkver-endpoints` feature. Release builds compile
//! those lookups out and always use the real HTTPS origins.

mod endpoints;
mod git;
mod github;
mod glob;
mod nexus;
mod version;

pub use endpoints::{
    github_asset_url, github_releases_url, github_remote_url, nexus_files_url,
    test_endpoints_enabled,
};
pub use git::{git_branch_commit, git_source_version};
pub use github::{select_github_release, SelectedGithubRelease};
pub use glob::glob_matches;
pub use nexus::{select_nexus_file, SelectedNexusFile};
pub use version::{github_tag_version, upstream_is_newer};

/// A test-only replacement for the Nexus API key lookup.
///
/// Release builds return `None`, so `modn catalog checkver` reads the key from
/// the environment or the OS credential store. The
/// `test-checkver-endpoints` feature is enabled only by test dependencies.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NexusKeyOverride {
    /// The test asserts the missing-key error without touching a real store.
    Missing,
    /// The test supplies a key without writing to a real store.
    Present(String),
}

/// Return the test-only Nexus API key replacement.
///
/// Release builds return `None` and never read the variable.
#[cfg(feature = "test-checkver-endpoints")]
#[must_use]
pub fn nexus_key_override() -> Option<NexusKeyOverride> {
    let value = std::env::var("MODN_TEST_CHECKVER_NEXUS_KEY").ok()?;
    if value.is_empty() {
        return None;
    }
    if value == "missing" {
        Some(NexusKeyOverride::Missing)
    } else {
        Some(NexusKeyOverride::Present(value))
    }
}

/// Return the test-only Nexus API key replacement.
///
/// Release builds return `None` and never read the variable.
#[cfg(not(feature = "test-checkver-endpoints"))]
#[must_use]
pub fn nexus_key_override() -> Option<NexusKeyOverride> {
    None
}
