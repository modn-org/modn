//! Resolve archives for `git` package sources without bypassing the commit pin.
//!
//! Lookups read the commit-keyed cache only. A fetch runs only when the caller
//! has explicit download or installation consent, like other remote sources.
use anyhow::{bail, Result};
use modn_core::acquisition::{ArchiveCache, CacheUse, GitCacheKey, GitSource};
use modn_core::buckets::{BucketPackage, PackageSource};
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

pub(super) fn cached(
    cache: &ArchiveCache,
    release: &BucketPackage,
) -> Result<Option<GitArchive>> {
    let Some(source) = source(release) else {
        return Ok(None);
    };
    let key = key(release, &source);
    Ok(cache
        .git_cached(&key)?
        .map(|(path, sha256)| GitArchive { path, sha256 }))
}

pub(super) fn acquire(
    cache: &ArchiveCache,
    release: &BucketPackage,
    cache_use: CacheUse,
) -> Result<GitArchive> {
    let Some(source) = source(release) else {
        bail!("{} is not a Git source", release.package.id);
    };
    let key = key(release, &source);
    let acquired = source.acquire(cache, &key, cache_use)?;
    Ok(GitArchive {
        path: acquired.path,
        sha256: acquired.sha256,
    })
}
