use modn_core::buckets::{PackageManifest, PackageSource};
use modn_core::transactions::ManagedPackage;

#[derive(Clone, Copy)]
pub(super) enum ReleaseIdentity<'a> {
    Archive(&'a str),
    Commit(&'a str),
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

pub(super) fn same_release(installed: ReleaseIdentity<'_>, target: ReleaseIdentity<'_>) -> bool {
    match (installed, target) {
        (ReleaseIdentity::Archive(installed), ReleaseIdentity::Archive(target))
        | (ReleaseIdentity::Commit(installed), ReleaseIdentity::Commit(target)) => {
            installed == target
        }
        _ => false,
    }
}
