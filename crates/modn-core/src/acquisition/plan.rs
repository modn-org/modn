//! Planned artifact sources and their archive-cache identity.
//!
//! A package manifest fixes the order in which Modn tries a release's artifact
//! sources and the URL that names each archive cache entry. Both belong to the
//! manifest contract, so planning lives here and every front end derives the
//! same order and the same keys. Planning is pure: it reads no network,
//! credential or file.

use super::{GithubReleaseAsset, NexusFile, SourceCandidate};
use crate::buckets::{BucketPackage, PackageSource};
use anyhow::Result;

/// The cache identity URL of one artifact source.
///
/// The cache entry name hashes this URL. `Ok(None)` means the source has no
/// archive cache entry of its own: a Git source is keyed by its commit, and a
/// manual source is never acquired automatically.
///
/// # Errors
///
/// Returns the validation error of an invalid Nexus selector.
pub fn source_cache_url(source: &PackageSource) -> Result<Option<String>> {
    Ok(match source {
        PackageSource::Https { url } => Some(url.clone()),
        PackageSource::GithubRelease {
            repository,
            tag,
            asset,
        } => Some(
            GithubReleaseAsset::new(repository.clone(), tag.clone(), asset.clone()).download_url(),
        ),
        PackageSource::Nexus {
            game_domain,
            mod_id,
            file_id,
            file_name,
        } => Some(NexusFile::new(game_domain, *mod_id, *file_id, file_name)?.cache_url()),
        _ => None,
    })
}

/// What Modn does with one planned artifact source.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlannedAction {
    /// Fetch a pinned Git commit.
    Git { repository: String, commit: String },
    /// Download the pinned Nexus Mods file.
    Nexus {
        game_domain: String,
        mod_id: u64,
        file_id: u64,
        file_name: String,
    },
    /// Fetch one source candidate without further resolution.
    Fetch { candidate: SourceCandidate },
    /// Resolve GitHub release metadata, then fetch the named asset.
    Github {
        selector: GithubReleaseAsset,
        expected: String,
    },
}

/// One artifact source of a release and what Modn does with it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PlannedSource {
    /// The manifest source this step acquires.
    pub source: PackageSource,
    /// The acquisition step for that source.
    pub action: PlannedAction,
}

/// Plan every artifact source of `release`, in manifest order.
#[must_use]
pub fn plan_sources(release: &BucketPackage) -> Vec<PlannedSource> {
    release
        .package
        .sources
        .iter()
        .map(|source| PlannedSource {
            source: source.clone(),
            action: planned_action(source),
        })
        .collect()
}

fn planned_action(source: &PackageSource) -> PlannedAction {
    match source {
        PackageSource::Git { repository, commit } => PlannedAction::Git {
            repository: repository.clone(),
            commit: commit.clone(),
        },
        PackageSource::Nexus {
            game_domain,
            mod_id,
            file_id,
            file_name,
        } => PlannedAction::Nexus {
            game_domain: game_domain.clone(),
            mod_id: *mod_id,
            file_id: *file_id,
            file_name: file_name.clone(),
        },
        PackageSource::GithubRelease {
            repository,
            tag,
            asset,
        } => {
            let selector = GithubReleaseAsset::new(repository.clone(), tag.clone(), asset.clone());
            PlannedAction::Github {
                expected: selector.download_url(),
                selector,
            }
        }
        PackageSource::Https { url } => PlannedAction::Fetch {
            candidate: SourceCandidate::fetch(url.clone()),
        },
        PackageSource::Manual { filename } => PlannedAction::Fetch {
            candidate: SourceCandidate::manual(filename.clone()),
        },
    }
}

#[cfg(test)]
mod tests;
