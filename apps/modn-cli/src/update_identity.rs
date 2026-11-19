//! What makes two releases the same release for `modn update`.
//!
//! An archive record pins the archive BLAKE3. A Git-only record pins a commit;
//! its packed archive digest becomes known only after acquisition, so the
//! recorded commit is the identity Modn compares.

use modn_core::buckets::{PackageManifest, PackageSource};
use modn_core::transactions::ManagedPackage;

/// What makes two releases the same release for `modn update`.
#[derive(Clone, Copy)]
pub(super) enum ReleaseIdentity<'a> {
    Archive(&'a str),
    Commit(&'a str),
    /// No pin and no commit: never equal to, or different from, another release.
    Unknown,
}

pub(super) fn source_identity(manifest: &PackageManifest) -> ReleaseIdentity<'_> {
    match manifest.primary_source() {
        PackageSource::Git { commit, .. } => ReleaseIdentity::Commit(commit),
        _ => manifest
            .pin()
            .map_or(ReleaseIdentity::Unknown, ReleaseIdentity::Archive),
    }
}

/// The commit of the manifest Git source with this repository, when listed.
fn listed_git_commit<'a>(
    manifest: &'a PackageManifest,
    recorded: &PackageSource,
) -> Option<&'a str> {
    let PackageSource::Git { repository, .. } = recorded else {
        return None;
    };
    manifest.sources.iter().find_map(|source| match source {
        PackageSource::Git {
            repository: listed,
            commit,
        } if listed == repository => Some(commit.as_str()),
        _ => None,
    })
}

/// What makes the target release current for the installed package.
///
/// The install records the source that served the archive. When the manifest
/// still lists that Git source, the source's own commit is the identity Modn
/// compares, so an install that fell back from the primary source stays
/// current. A release that dropped the recorded source falls back to the
/// primary source. A Git source listed with another commit still needs an
/// update.
pub(super) fn target_identity<'a>(
    manifest: &'a PackageManifest,
    installed: &ManagedPackage,
) -> ReleaseIdentity<'a> {
    installed
        .source
        .as_ref()
        .and_then(|source| listed_git_commit(manifest, source))
        .map_or_else(|| source_identity(manifest), ReleaseIdentity::Commit)
}

pub(super) fn installed_identity(package: &ManagedPackage) -> ReleaseIdentity<'_> {
    match &package.source {
        Some(PackageSource::Git { commit, .. }) => ReleaseIdentity::Commit(commit),
        _ => ReleaseIdentity::Archive(&package.archive_blake3),
    }
}

/// Return whether the target release is the one Modn already installed.
pub(super) fn same_release(installed: ReleaseIdentity<'_>, target: ReleaseIdentity<'_>) -> bool {
    match (installed, target) {
        (ReleaseIdentity::Archive(installed), ReleaseIdentity::Archive(target))
        | (ReleaseIdentity::Commit(installed), ReleaseIdentity::Commit(target)) => {
            installed == target
        }
        _ => false,
    }
}
