//! Resolve the archives that an install order already has.
//!
//! Resolution reads supplied files, the manager profile archive cache and each
//! release's planned sources. It is pure with respect to the game and the
//! network: no download, no cache write, no game write.

use super::order::OrderedRelease;
use super::plan::{PendingDownload, ServedArchive, ServedOrigin};
use crate::acquisition::{plan_sources, ArchiveCache, CacheUse, ReleaseDigest};
use crate::archive;
use crate::buckets::{ArchiveFormat, BucketPackage, PackageSource};
use anyhow::{bail, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The digest and archive that one ordered release already has.
#[derive(Debug)]
pub(super) struct ResolvedArchive {
    /// The digest Modn has for the release.
    pub(super) digest: ReleaseDigest,
    /// A supplied or verified cached archive, when one exists.
    pub(super) served: Option<ServedArchive>,
}

/// The archive resolution of one install order.
#[derive(Debug)]
pub(super) struct ResolvedArchives {
    /// One entry per release in the order, in the same order.
    pub(super) releases: Vec<ResolvedArchive>,
    /// Releases with no supplied or verified cached archive, in order order.
    pub(super) pending: Vec<PendingDownload>,
    /// Supplied paths whose BLAKE3 matches no release, in `--file` order.
    pub(super) unmatched: Vec<PathBuf>,
}

/// Resolve the archive of every release in `order`.
///
/// A supplied file must be a regular archive whose format matches the release
/// it names; the wrong format is an error. A file whose BLAKE3 matches no
/// release is returned in [`ResolvedArchives::unmatched`] for the caller to
/// refuse. With [`CacheUse::Reuse`], a release that has no supplied archive may
/// be served by a verified cache entry. Every other release is listed as a
/// pending download.
pub(super) fn resolve(
    order: &[OrderedRelease],
    supplied: &[PathBuf],
    cache: &ArchiveCache,
    cache_use: CacheUse,
) -> Result<ResolvedArchives> {
    let (mut served, unmatched) = match_supplied(supplied, order)?;
    if cache_use == CacheUse::Reuse {
        for (index, ordered) in order.iter().enumerate() {
            if served[index].is_some() {
                continue;
            }
            let Some(acquired) = cache.cached_release(&ordered.package)? else {
                continue;
            };
            served[index] = Some(ServedArchive {
                path: acquired.path,
                source: acquired.source,
                digest: acquired.digest,
                origin: ServedOrigin::Cache,
            });
        }
    }
    let mut releases = Vec::with_capacity(order.len());
    let mut pending = Vec::new();
    for (ordered, served) in order.iter().zip(served) {
        let digest = match &served {
            Some(archive) => archive.digest.clone(),
            None => ReleaseDigest::from_pin(ordered.package.package.pin())?,
        };
        if served.is_none() {
            pending.push(pending_download(&ordered.package, digest.clone()));
        }
        releases.push(ResolvedArchive { digest, served });
    }
    Ok(ResolvedArchives {
        releases,
        pending,
        unmatched,
    })
}

/// Return the indexes of `installing` that one stack installation may place,
/// followed by the selected release.
///
/// The indexes are positions in `order`, and `installing` must not contain the
/// selected release, which is the last position. Releases that share one
/// archive are placed once when they declare the same dependencies and
/// conflicts. Two releases that share one archive but declare different
/// requirements, and a selected release that shares its pinned archive with an
/// installing release, are errors.
pub(super) fn one_archive_per_digest(
    order: &[OrderedRelease],
    archives: &ResolvedArchives,
    installing: &[usize],
) -> Result<Vec<usize>> {
    let mut by_digest: BTreeMap<&str, usize> = BTreeMap::new();
    let mut kept = Vec::new();
    for &index in installing {
        let Some(digest) = archives.releases[index].digest.as_str() else {
            kept.push(index);
            continue;
        };
        let Some(&first) = by_digest.get(digest) else {
            by_digest.insert(digest, index);
            kept.push(index);
            continue;
        };
        if declared(&order[first].package) != declared(&order[index].package) {
            bail!(
                "{} and {} share one archive but declare different dependencies or conflicts; install them separately",
                order[first].package.reference(),
                order[index].package.reference()
            );
        }
    }
    let Some((selected_index, selected)) = order.iter().enumerate().next_back() else {
        bail!("install order is empty");
    };
    if selected
        .package
        .package
        .pin()
        .is_some_and(|pin| by_digest.contains_key(pin))
    {
        bail!(
            "{}: its archive is also the archive of a missing dependency; install that dependency first",
            selected.package.reference()
        );
    }
    kept.push(selected_index);
    Ok(kept)
}

fn match_supplied(
    files: &[PathBuf],
    order: &[OrderedRelease],
) -> Result<(Vec<Option<ServedArchive>>, Vec<PathBuf>)> {
    let mut served = vec![None; order.len()];
    let mut unmatched = Vec::new();
    for file in files {
        if !std::fs::symlink_metadata(file)?.file_type().is_file() {
            bail!("archive must be a regular file: {}", file.display());
        }
        let inspected = archive::inspect(file)?;
        let matches: Vec<usize> = order
            .iter()
            .enumerate()
            .filter(|(_, ordered)| ordered.package.package.pin() == Some(inspected.blake3.as_str()))
            .map(|(index, _)| index)
            .collect();
        if matches.is_empty() {
            unmatched.push(file.clone());
            continue;
        }
        for index in matches {
            let package = &order[index].package;
            if Some(inspected.format) != expected_format(package.package.archive_format.as_ref()) {
                bail!(
                    "{}: archive format does not match catalog manifest",
                    package.reference()
                );
            }
            served[index] = Some(ServedArchive {
                path: file.clone(),
                source: supplied_source(package, file, &inspected.blake3),
                digest: ReleaseDigest::computed(&inspected.blake3)?,
                origin: ServedOrigin::Supplied,
            });
        }
    }
    Ok((served, unmatched))
}

fn pending_download(package: &BucketPackage, digest: ReleaseDigest) -> PendingDownload {
    let sources = plan_sources(package);
    let downloadable = sources
        .iter()
        .any(|planned| !matches!(planned.source, PackageSource::Manual { .. }));
    PendingDownload {
        reference: package.reference(),
        version: package.package.version.clone(),
        digest,
        sources,
        downloadable,
    }
}

fn expected_format(format: Option<&ArchiveFormat>) -> Option<archive::ArchiveFormat> {
    match format {
        Some(ArchiveFormat::Zip) => Some(archive::ArchiveFormat::Zip),
        Some(ArchiveFormat::SevenZ) => Some(archive::ArchiveFormat::SevenZ),
        None => None,
    }
}

fn supplied_source(release: &BucketPackage, file: &Path, blake3: &str) -> PackageSource {
    if release.package.pin() == Some(blake3) {
        if let Some(source) = refetchable_source(&release.package.sources) {
            return source.clone();
        }
    }
    PackageSource::Manual {
        filename: file
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
    }
}

fn refetchable_source(sources: &[PackageSource]) -> Option<&PackageSource> {
    sources.iter().find(|source| {
        matches!(
            source,
            PackageSource::Https { .. } | PackageSource::GithubRelease { .. }
        )
    })
}

fn declared(package: &BucketPackage) -> (BTreeSet<&str>, BTreeSet<&str>) {
    (
        package
            .package
            .dependencies
            .iter()
            .map(String::as_str)
            .collect(),
        package
            .package
            .conflicts
            .iter()
            .map(String::as_str)
            .collect(),
    )
}
