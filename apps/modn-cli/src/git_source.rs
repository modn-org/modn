//! Resolve archives for `git` package sources without bypassing the commit pin.
//!
//! Lookups read the commit-keyed cache only. A fetch runs only when the caller
//! has explicit download or installation consent, like other remote sources.
use crate::progress::Progress;
use anyhow::{bail, Result};
use modn_core::acquisition::{ArchiveCache, CacheUse, GitCacheKey, GitSource, Origin};
use modn_core::buckets::{BucketPackage, PackageSource};
use modn_core::progress::ProgressEvent;
use std::fs;
use std::path::PathBuf;

pub(super) struct GitArchive {
    pub(super) path: PathBuf,
    pub(super) sha256: String,
}

fn source(release: &BucketPackage) -> Option<GitSource> {
    match &release.package.source {
        PackageSource::Git { repository, commit } => {
            Some(GitSource::new(repository.clone(), commit.clone()))
        }
        _ => None,
    }
}

fn key<'a>(release: &'a BucketPackage, source: &'a GitSource) -> GitCacheKey<'a> {
    GitCacheKey::new(
        &release.package.game,
        &release.package.id,
        &release.package.version,
        &source.commit,
    )
}

pub(super) fn cached(cache: &ArchiveCache, release: &BucketPackage) -> Result<Option<GitArchive>> {
    let Some(source) = source(release) else {
        return Ok(None);
    };
    let key = key(release, &source);
    Ok(cache
        .git_cached(&key)?
        .map(|(path, sha256)| GitArchive { path, sha256 }))
}

/// Return a verified cached archive or fetch and pack the pinned commit.
///
/// A fetch reports one `download` phase for the package. The phase starts at
/// zero bytes, and ends with the packed archive size. A cancellation before
/// the fetch starts returns [`modn_core::progress::Cancelled`] without running
/// Git. A cancellation observed after the fetch and pack finishes removes the
/// new cache entry, so a cancelled install changes no cache state.
pub(super) fn acquire(
    cache: &ArchiveCache,
    release: &BucketPackage,
    cache_use: CacheUse,
    progress: &Progress,
) -> Result<GitArchive> {
    let Some(source) = source(release) else {
        bail!("{} is not a Git source", release.package.id);
    };
    let key = key(release, &source);
    if cache_use == CacheUse::Reuse {
        if let Some(cached) = cached(cache, release)? {
            return Ok(cached);
        }
    }
    let control = progress.control();
    let reference = release.reference().to_string();
    control.advance(ProgressEvent::Download {
        package: Some(reference.clone()),
        bytes: 0,
        total: None,
    })?;
    let acquired = source.acquire(cache, &key, cache_use)?;
    let archive = GitArchive {
        path: acquired.path,
        sha256: acquired.sha256,
    };
    if let Err(cancelled) = control.checkpoint() {
        if cache_use == CacheUse::Reuse && acquired.origin == Origin::Download {
            let _ = fs::remove_file(&archive.path);
        }
        return Err(cancelled.into());
    }
    if acquired.origin == Origin::Download {
        let bytes = fs::metadata(&archive.path).map_or(0, |metadata| metadata.len());
        control.report(&ProgressEvent::Download {
            package: Some(reference),
            bytes,
            total: Some(bytes),
        });
    }
    Ok(archive)
}
